use anyhow::Result;
use std::path::PathBuf;

use crate::models::{self, ThinResponse};
use crate::store::Store;

use super::validate_nonzero;
pub struct OutlineOpts {
    pub root: PathBuf,
    pub path: String,
    pub kind_filter: Option<String>,
    pub limit: usize,
    pub max_bytes: u64,
    pub cursor: Option<String>,
    pub fmt: String,
}

pub fn run(opts: OutlineOpts) -> Result<()> {
    if let Err(message) = validate_nonzero("limit", opts.limit as u64) {
        let resp = ThinResponse::error(
            "outline",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    if let Err(message) = validate_nonzero("max-bytes", opts.max_bytes) {
        let resp = ThinResponse::error(
            "outline",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let kind_filter = if let Some(ref kind) = opts.kind_filter {
        match kind.parse::<models::BlockKind>() {
            Ok(parsed) => Some(parsed.to_string()),
            Err(_) => {
                let resp = ThinResponse::error(
                    "outline",
                    opts.max_bytes,
                    models::ERR_PARSE_ERROR,
                    format!("invalid kind '{kind}'"),
                    None,
                );
                println!("{}", serde_json::to_string(&resp)?);
                return Ok(());
            }
        }
    } else {
        None
    };

    let codeai_dir = opts.root.join(".worktoolai").join("codeai");
    let db_path = codeai_dir.join("index.db");

    if !db_path.exists() {
        let resp = ThinResponse::error(
            "outline",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "No index found. Run 'codeai index' first.".into(),
            Some(vec![serde_json::json!(["index", {}])]),
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let store = Store::open(&db_path)?;

    // Auto re-index stale files before querying
    let search_dir = codeai_dir.join("search");
    if let Ok(search_index) = crate::search::SearchIndex::open(&search_dir) {
        if let Err(e) = crate::autoreindex::ensure_fresh(&opts.root, &store, &search_index) {
            eprintln!("warning: auto re-index failed: {e}");
        }
    }

    // Normalize the path
    let rel_path = opts.path.replace('\\', "/");
    let rel_path = rel_path.trim_start_matches("./");

    let blocks = store.blocks_for_file(rel_path)?;

    if blocks.is_empty() {
        // Check if the file exists in DB at all
        if store.get_file(rel_path)?.is_none() {
            let resp = ThinResponse::error(
                "outline",
                opts.max_bytes,
                models::ERR_FILE_NOT_FOUND,
                format!("File '{rel_path}' not found in index."),
                Some(vec![serde_json::json!(["search", {"query": rel_path}])]),
            );
            println!("{}", serde_json::to_string(&resp)?);
            return Ok(());
        }
    }

    // Filter by kind if requested
    let filtered: Vec<_> = if let Some(ref kind) = kind_filter {
        blocks.iter().filter(|b| b.kind == *kind).collect()
    } else {
        blocks.iter().collect()
    };

    // Apply limit
    let limited: Vec<_> = filtered.into_iter().take(opts.limit).collect();

    // outline tuple: [symbol_id, name, kind, path, range]
    let items: Vec<serde_json::Value> = limited
        .iter()
        .map(|b| {
            serde_json::json!([
                b.symbol_id,
                b.name,
                b.kind,
                b.path,
                format!(
                    "{}:{}-{}:{}",
                    b.start_line, b.start_col, b.end_line, b.end_col
                ),
            ])
        })
        .collect();

    let resp = ThinResponse::success("outline", opts.max_bytes, items);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}
