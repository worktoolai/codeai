use anyhow::{Context, Result};
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

use crate::lang;
use crate::models::{self, build_symbol_id, BlockKind};
use crate::parser;
use crate::scanner::Scanner;
use crate::search::{SearchDoc, SearchIndex};
use crate::store::{BlockRow, FileMeta, Store};

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
    let codeai_dir = opts.root.join(".codeai");
    std::fs::create_dir_all(&codeai_dir)?;

    let db_path = codeai_dir.join("index.db");
    let search_dir = codeai_dir.join("search");

    let store = Store::open(&db_path)?;
    let search_index = SearchIndex::open(&search_dir)?;

    if opts.full {
        store.clear_all()?;
        search_index.clear_all()?;
    }

    // Scan files
    let mut scanner = Scanner::new(opts.root.clone())
        .no_gitignore(opts.no_gitignore)
        .no_default_ignores(opts.no_default_ignores);

    if let Some(ref lang) = opts.lang_filter {
        scanner = scanner.lang_filter(lang.clone());
    }
    if let Some(ref ignore) = opts.ignore_file {
        scanner = scanner.ignore_file(ignore.clone());
    }

    let files = scanner.scan()?;

    // Detect deleted files
    let existing_paths = store.all_file_paths()?;
    let scanned_paths: std::collections::HashSet<&str> =
        files.iter().map(|f| f.rel_path.as_str()).collect();

    let mut search_writer = search_index.writer()?;

    for ep in &existing_paths {
        if !scanned_paths.contains(ep.as_str()) {
            store.delete_file(ep)?;
            search_index.delete_by_path(&search_writer, ep)?;
        }
    }

    let mut indexed_files = 0u64;
    let mut indexed_blocks = 0u64;
    let mut skipped = 0u64;
    let mut errors = 0u64;

    for file in &files {
        // Check if file changed
        if let Some(existing) = store.get_file(&file.rel_path)? {
            if existing.mtime == file.mtime && existing.size == file.size {
                skipped += 1;
                continue;
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
        let mut name_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mut block_rows = Vec::new();

        // First pass: count occurrences
        let mut occurrence_map: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
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
        indexed_files += 1;
    }

    // Commit search index
    search_writer.commit().context("commit search index")?;
    search_index.reload()?;

    // Update generation
    let gen = store.next_generation()?;

    // Output result
    let total_blocks = store.block_count()?;
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
