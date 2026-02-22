use anyhow::Result;
use std::path::PathBuf;

use crate::models::{self, ThinResponse};
use crate::store::Store;

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
    let codeai_dir = opts.root.join(".codeai");
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
    let filtered: Vec<_> = if let Some(ref kind) = opts.kind_filter {
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
                format!("{}:{}-{}:{}", b.start_line, b.start_col, b.end_line, b.end_col),
            ])
        })
        .collect();

    let resp = ThinResponse::success("outline", opts.max_bytes, items);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}
