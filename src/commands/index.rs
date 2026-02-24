use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use xxhash_rust::xxh3::xxh3_64;

use crate::lang;
use crate::models::{self, build_symbol_id, BlockKind};
use crate::parser;
use crate::scanner::Scanner;
use crate::search::{SearchDoc, SearchIndex};
use crate::store::{BlockRow, FileMeta, ImportRow, Store};

use super::{validate_lang_filter, validate_nonzero};
const IMPORTS_BACKFILL_META_KEY: &str = "imports_backfill_v1_done";

pub struct IndexOpts {
    pub root: PathBuf,
    pub full: bool,
    pub path_filter: Option<String>,
    pub lang_filter: Option<String>,
    pub no_gitignore: bool,
    pub no_default_ignores: bool,
    pub ignore_file: Option<PathBuf>,
    pub max_bytes: u64,
    pub fmt: String,
}

pub fn run(opts: IndexOpts) -> Result<()> {
    if let Err(message) = validate_nonzero("max-bytes", opts.max_bytes) {
        let resp = models::ThinResponse::error(
            "index",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    if let Some(ref lang) = opts.lang_filter {
        if !validate_lang_filter(lang) {
            let resp = models::ThinResponse::error(
                "index",
                opts.max_bytes,
                models::ERR_UNSUPPORTED_LANGUAGE,
                format!("unsupported language filter '{lang}'"),
                None,
            );
            println!("{}", serde_json::to_string(&resp)?);
            return Ok(());
        }
    }

    let codeai_dir = opts.root.join(".worktoolai").join("codeai");
    std::fs::create_dir_all(&codeai_dir)?;

    let db_path = codeai_dir.join("index.db");
    let search_dir = codeai_dir.join("search");

    let store = Store::open(&db_path)?;

    if opts.full {
        store.clear_all()?;
        // Remove and recreate search directory to pick up schema/tokenizer changes
        if search_dir.exists() {
            std::fs::remove_dir_all(&search_dir)?;
        }
    }

    let search_index = SearchIndex::open(&search_dir)?;

    // Build scanner for fallback / full indexing
    let mut scanner = Scanner::new(opts.root.clone())
        .no_gitignore(opts.no_gitignore)
        .no_default_ignores(opts.no_default_ignores);

    if let Some(ref lang) = opts.lang_filter {
        scanner = scanner.lang_filter(lang.clone());
    }
    if let Some(ref ignore) = opts.ignore_file {
        scanner = scanner.ignore_file(ignore.clone());
    }

    let git_sync = compute_git_sync(&opts.root, &store)?;
    let existing_paths = store.all_file_paths()?;
    let existing_set: HashSet<String> = existing_paths.iter().cloned().collect();

    let filtered_mode = opts.lang_filter.is_some() || opts.path_filter.is_some();
    let should_backfill_imports = !opts.full
        && !filtered_mode
        && store.import_count()? == 0
        && store.block_count()? > 0
        && store.get_meta(IMPORTS_BACKFILL_META_KEY)?.is_none();
    let git_mode = !opts.full
        && !filtered_mode
        && !should_backfill_imports
        && git_sync.use_git
        && !git_sync.force_scan;
    let full_compare_mode = opts.full || (!filtered_mode && !git_mode);

    let mut files = if git_mode {
        let changed_set = git_sync.changed_paths();
        if changed_set.is_empty() {
            Vec::new()
        } else {
            scanner
                .scan()?
                .into_iter()
                .filter(|f| changed_set.contains(&f.rel_path))
                .collect()
        }
    } else {
        scanner.scan()?
    };

    // Extra path filter (prefix)
    if let Some(ref pf) = opts.path_filter {
        let norm = pf.replace('\\', "/").trim_start_matches("./").to_string();
        files.retain(|f| f.rel_path.starts_with(&norm));
    }

    let file_map: HashMap<String, crate::scanner::ScanResult> =
        files.into_iter().map(|f| (f.rel_path.clone(), f)).collect();

    let mut to_delete: HashSet<String> = HashSet::new();
    let mut to_index: HashSet<String> = HashSet::new();

    if git_mode {
        // Git-aware sync behavior
        for p in git_sync.delete_paths.iter() {
            if existing_set.contains(p) {
                to_delete.insert(p.clone());
            }
        }
        for p in git_sync.index_paths.iter() {
            to_index.insert(p.clone());
        }
    } else if full_compare_mode {
        // Full/fallback behavior: compare scan result with DB
        let scanned_paths: HashSet<String> = file_map.keys().cloned().collect();
        for ep in &existing_paths {
            if !scanned_paths.contains(ep) {
                to_delete.insert(ep.clone());
            }
        }
        for p in file_map.keys() {
            to_index.insert(p.clone());
        }
    } else {
        // Filtered index mode: only index scanned subset (no global deletes)
        for p in file_map.keys() {
            to_index.insert(p.clone());
        }
    }

    let mut search_writer = search_index.writer()?;

    // Build all_paths set for import resolution
    let all_paths_set: HashSet<String> = existing_set
        .iter()
        .chain(file_map.keys())
        .cloned()
        .collect();

    if should_backfill_imports {
        to_index.extend(existing_set.iter().cloned());
    }

    // Delete removed / renamed-old paths
    for path in &to_delete {
        if let Ok(existing_blocks) = store.blocks_for_file(path) {
            for b in existing_blocks {
                search_index.delete_by_symbol_id(&search_writer, &b.symbol_id)?;
            }
        }
        store.delete_file(path)?;
    }

    let mut indexed_files = 0u64;
    let mut indexed_blocks = 0u64;
    let mut skipped = 0u64;
    let mut errors = 0u64;

    for path in to_index.iter() {
        let Some(file) = file_map.get(path) else {
            // Path no longer exists on disk (e.g. deleted in working tree)
            if existing_set.contains(path) {
                if let Ok(existing_blocks) = store.blocks_for_file(path) {
                    for b in existing_blocks {
                        search_index.delete_by_symbol_id(&search_writer, &b.symbol_id)?;
                    }
                }
                store.delete_file(path)?;
            }
            continue;
        };
        // Check if file changed
        if !should_backfill_imports {
            if let Some(existing) = store.get_file(&file.rel_path)? {
                if existing.mtime == file.mtime && existing.size == file.size {
                    skipped += 1;
                    continue;
                }
            }
        }

        // Read file content
        let content = match std::fs::read(&file.path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: cannot read {}: {}", file.rel_path, e);
                errors += 1;
                continue;
            }
        };

        // Compute hash
        let content_hash = format!("{:016x}", xxh3_64(&content));

        // Check hash for actual change
        if !should_backfill_imports {
            if let Some(existing) = store.get_file(&file.rel_path)? {
                if existing.content_hash == content_hash {
                    // mtime changed but content didn't — update mtime only
                    store.upsert_file(&FileMeta {
                        path: file.rel_path.clone(),
                        mtime: file.mtime,
                        size: file.size,
                        content_hash,
                        language: existing.language,
                        parse_error: existing.parse_error,
                    })?;
                    skipped += 1;
                    continue;
                }
            }
        }

        // File content changed: delete existing search docs for this path first
        for b in store.blocks_for_file(&file.rel_path)? {
            search_index.delete_by_symbol_id(&search_writer, &b.symbol_id)?;
        }

        // Get language config
        let config = match lang::config_for_extension(&file.extension) {
            Some(c) => c,
            None => {
                skipped += 1;
                continue;
            }
        };

        let ts_lang = match lang::ts_language_for_extension(&file.extension) {
            Some(l) => l,
            None => {
                // Language config exists but no grammar crate — store file but skip parsing
                store.upsert_file(&FileMeta {
                    path: file.rel_path.clone(),
                    mtime: file.mtime,
                    size: file.size,
                    content_hash: content_hash.clone(),
                    language: Some(config.language.to_string()),
                    parse_error: false,
                })?;
                store.replace_blocks(&file.rel_path, &[])?;
                skipped += 1;
                continue;
            }
        };

        // Parse and extract blocks
        let extracted = match parser::extract_blocks(
            &content,
            &file.rel_path,
            config.language,
            ts_lang,
            config.function_nodes,
            config.class_nodes,
        ) {
            Ok(blocks) => blocks,
            Err(e) => {
                eprintln!("warning: parse error {}: {}", file.rel_path, e);
                store.upsert_file(&FileMeta {
                    path: file.rel_path.clone(),
                    mtime: file.mtime,
                    size: file.size,
                    content_hash: content_hash.clone(),
                    language: Some(config.language.to_string()),
                    parse_error: true,
                })?;
                errors += 1;
                continue;
            }
        };

        // Build symbol_ids with occurrence tracking
        let mut name_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        let mut block_rows = Vec::new();

        // First pass: count occurrences
        let mut occurrence_map: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, block) in extracted.iter().enumerate() {
            let key = format!("{}#{}", block.kind, block.name);
            occurrence_map.entry(key).or_default().push(i);
        }

        // Second pass: build symbol_ids
        for block in extracted.iter() {
            let key = format!("{}#{}", block.kind, block.name);
            let count_key = key.clone();
            let total = occurrence_map.get(&key).map(|v| v.len()).unwrap_or(1);

            let kind = BlockKind::from_str_loose(&block.kind);
            let occurrence = if total > 1 {
                let idx = name_counts.entry(count_key).or_insert(0);
                let occ = *idx;
                *idx += 1;
                Some(occ)
            } else {
                None
            };

            let symbol_id = build_symbol_id(&file.rel_path, &kind, &block.name, occurrence);

            let block_row = BlockRow {
                symbol_id: symbol_id.clone(),
                path: file.rel_path.clone(),
                language: config.language.to_string(),
                kind: block.kind.clone(),
                name: block.name.clone(),
                start_line: block.start_line,
                start_col: block.start_col,
                end_line: block.end_line,
                end_col: block.end_col,
                signature: block.signature.clone(),
                doc: block.doc.clone(),
                preview: block.preview.clone(),
            };

            // Index in search
            let search_doc = SearchDoc {
                symbol_id: symbol_id.clone(),
                name: block.name.clone(),
                path: file.rel_path.clone(),
                kind: block.kind.clone(),
                signature: block.signature.clone().unwrap_or_default(),
                doc: block.doc.clone().unwrap_or_default(),
                preview: block.preview.clone(),
                strings: block.strings.join("\n"),
            };
            search_index.index_block(&search_writer, &search_doc)?;

            block_rows.push(block_row);
        }

        indexed_blocks += block_rows.len() as u64;

        // Store file metadata
        store.upsert_file(&FileMeta {
            path: file.rel_path.clone(),
            mtime: file.mtime,
            size: file.size,
            content_hash: content_hash.clone(),
            language: Some(config.language.to_string()),
            parse_error: false,
        })?;

        // Store blocks
        store.replace_blocks(&file.rel_path, &block_rows)?;

        // Extract and store imports
        if !config.import_nodes.is_empty() {
            let ts_lang_for_imports = lang::ts_language_for_extension(&file.extension).unwrap();
            if let Ok(extracted_imports) =
                parser::extract_imports(&content, ts_lang_for_imports, config.import_nodes)
            {
                let import_rows: Vec<ImportRow> = extracted_imports
                    .iter()
                    .map(|ei| {
                        let resolved = parser::resolve_import(
                            &ei.raw_import,
                            &file.rel_path,
                            config.language,
                            &all_paths_set,
                        );
                        ImportRow {
                            path: file.rel_path.clone(),
                            raw_import: ei.raw_import.clone(),
                            resolved_path: resolved,
                            kind: ei.kind.clone(),
                        }
                    })
                    .collect();
                let _ = store.replace_imports(&file.rel_path, &import_rows);
            }
        }

        indexed_files += 1;
    }

    // Commit search index
    search_writer.commit().context("commit search index")?;
    search_index.reload()?;

    if should_backfill_imports {
        let _ = store.set_meta(IMPORTS_BACKFILL_META_KEY, "1");
    }

    // Persist current git HEAD for next incremental sync (best effort)
    if let Some(head) = current_git_head(&opts.root) {
        let _ = store.set_last_indexed_head(&head);
    }

    // Update generation
    let gen = store.next_generation()?;

    // Output result
    let total_blocks = store.block_count()?;
    let total_imports = store.import_count()?;
    let resp = models::ThinResponse::success(
        "index",
        opts.max_bytes,
        vec![serde_json::json!({
            "generation": gen,
            "indexed_files": indexed_files,
            "indexed_blocks": indexed_blocks,
            "skipped_files": skipped,
            "errors": errors,
            "total_blocks": total_blocks,
            "total_imports": total_imports,
        })],
    );

    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

// Helper: loose BlockKind parsing that doesn't fail
impl BlockKind {
    fn from_str_loose(s: &str) -> Self {
        s.parse().unwrap_or(BlockKind::Function)
    }
}

#[derive(Debug, Default)]
struct GitSyncPlan {
    use_git: bool,
    force_scan: bool,
    index_paths: HashSet<String>,
    delete_paths: HashSet<String>,
}

impl GitSyncPlan {
    fn changed_paths(&self) -> HashSet<String> {
        self.index_paths
            .iter()
            .cloned()
            .chain(self.delete_paths.iter().cloned())
            .collect()
    }
}

fn compute_git_sync(root: &PathBuf, store: &Store) -> Result<GitSyncPlan> {
    if !root.join(".git").exists() {
        return Ok(GitSyncPlan {
            force_scan: true,
            ..Default::default()
        });
    }
    let mut plan = GitSyncPlan::default();

    let current_head = current_git_head(root);
    let Some(current_head) = current_head else {
        plan.force_scan = true;
        return Ok(plan);
    };

    let last_head = store.last_indexed_head()?;

    // Collect working tree changes (staged/unstaged + untracked)
    let status = git_status_name(root)?;
    if let Some(lines) = status {
        for line in lines.lines() {
            if line.len() < 3 {
                continue;
            }
            let status_code = &line[..2];
            let rest = line[3..].trim();
            if rest.is_empty() {
                continue;
            }

            if status_code.contains('R') {
                if let Some((oldp, newp)) = rest.split_once(" -> ") {
                    plan.delete_paths.insert(normalize_rel_path(oldp));
                    plan.index_paths.insert(normalize_rel_path(newp));
                }
                continue;
            }

            // D = delete, A/M/T/U/? = index/update path
            if status_code.contains('D') {
                plan.delete_paths.insert(normalize_rel_path(rest));
            } else {
                plan.index_paths.insert(normalize_rel_path(rest));
            }
        }
    }

    // Collect committed changes since last indexed head
    if let Some(last) = last_head {
        if last != current_head {
            if let Some(lines) = git_diff_name_status(root, &last, &current_head)? {
                plan.use_git = true;
                for line in lines.lines() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.is_empty() {
                        continue;
                    }
                    let code = parts[0];
                    match code.chars().next().unwrap_or('M') {
                        'D' => {
                            if parts.len() >= 2 {
                                plan.delete_paths.insert(normalize_rel_path(parts[1]));
                            }
                        }
                        'R' => {
                            if parts.len() >= 3 {
                                plan.delete_paths.insert(normalize_rel_path(parts[1]));
                                plan.index_paths.insert(normalize_rel_path(parts[2]));
                            }
                        }
                        _ => {
                            if parts.len() >= 2 {
                                plan.index_paths.insert(normalize_rel_path(parts[1]));
                            }
                        }
                    }
                }
            } else {
                // If diff fails (e.g., rewritten history), fallback to full scan.
                plan.force_scan = true;
            }
        } else {
            plan.use_git = true;
        }
    } else {
        // No prior git_head metadata: first run should use full scan.
        plan.force_scan = true;
    }

    Ok(plan)
}

fn current_git_head(root: &PathBuf) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

fn git_status_name(root: &PathBuf) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output();

    let Ok(out) = out else {
        return Ok(None);
    };
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).to_string()))
}

fn git_diff_name_status(root: &PathBuf, from: &str, to: &str) -> Result<Option<String>> {
    let range = format!("{from}..{to}");
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-status", &range])
        .output();

    let Ok(out) = out else {
        return Ok(None);
    };
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).to_string()))
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}
