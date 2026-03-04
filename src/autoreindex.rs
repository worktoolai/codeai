use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;

use crate::commands::index::reindex_file;
use crate::scanner::ScanResult;
use crate::search::SearchIndex;
use crate::store::Store;

/// Check indexed files for staleness and re-index changed ones.
/// Returns (reindexed_count, removed_count).
/// If no index exists yet, returns Ok((0, 0)) immediately.
pub fn ensure_fresh(root: &Path, store: &Store, search_index: &SearchIndex) -> Result<(u64, u64)> {
    let paths = store.all_file_paths()?;
    if paths.is_empty() {
        return Ok((0, 0));
    }

    let mut changed_files: Vec<ScanResult> = Vec::new();
    let mut deleted_paths: Vec<String> = Vec::new();

    for rel_path in &paths {
        let abs_path = root.join(rel_path);
        if !abs_path.exists() {
            deleted_paths.push(rel_path.clone());
            continue;
        }

        let meta = match std::fs::metadata(&abs_path) {
            Ok(m) => m,
            Err(_) => {
                deleted_paths.push(rel_path.clone());
                continue;
            }
        };

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size = meta.len();

        // Compare with stored mtime/size
        if let Some(existing) = store.get_file(rel_path)? {
            if existing.mtime == mtime && existing.size == size {
                continue; // unchanged
            }
        }

        // Derive extension from path
        let extension = Path::new(rel_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        changed_files.push(ScanResult {
            path: abs_path,
            rel_path: rel_path.clone(),
            extension,
            size,
            mtime,
        });
    }

    if changed_files.is_empty() && deleted_paths.is_empty() {
        return Ok((0, 0));
    }

    let mut search_writer = search_index.writer()?;
    let all_paths_set: HashSet<String> = paths.into_iter().collect();

    // Remove deleted files from store + search
    for path in &deleted_paths {
        if let Ok(blocks) = store.blocks_for_file(path) {
            for b in blocks {
                search_index.delete_by_symbol_id(&search_writer, &b.symbol_id)?;
            }
        }
        store.delete_file(path)?;
    }

    // Re-index changed files
    let mut reindexed = 0u64;
    for file in &changed_files {
        match reindex_file(root, store, search_index, &search_writer, &all_paths_set, file, false) {
            Ok(result) => {
                if !result.skipped && !result.error {
                    reindexed += 1;
                }
            }
            Err(e) => {
                eprintln!("warning: re-index {}: {}", file.rel_path, e);
            }
        }
    }

    let removed = deleted_paths.len() as u64;

    if reindexed > 0 || removed > 0 {
        search_writer.commit()?;
        search_index.reload()?;
        store.next_generation()?;
    }

    Ok((reindexed, removed))
}
