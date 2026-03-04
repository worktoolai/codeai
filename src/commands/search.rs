use anyhow::Result;
use std::path::PathBuf;

use crate::models::{self, ThinResponse};
use crate::search::SearchIndex;
use crate::store::Store;

use super::{validate_lang_filter, validate_nonzero};
pub struct SearchOpts {
    pub root: PathBuf,
    pub query: String,
    pub limit: usize,
    pub path_filter: Option<String>,
    pub lang_filter: Option<String>,
    pub max_bytes: u64,
    pub cursor: Option<String>,
    pub fmt: String,
}

pub fn run(opts: SearchOpts) -> Result<()> {
    if let Err(message) = validate_nonzero("limit", opts.limit as u64) {
        let resp = ThinResponse::error(
            "search",
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
            "search",
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
            let resp = ThinResponse::error(
                "search",
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
    let db_path = codeai_dir.join("index.db");
    let search_dir = codeai_dir.join("search");

    if !db_path.exists() {
        let resp = ThinResponse::error(
            "search",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "No index found. Run 'codeai index' first.".into(),
            Some(vec![serde_json::json!(["index", {}])]),
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let store = Store::open(&db_path)?;
    let search_index = SearchIndex::open(&search_dir)?;

    // Auto re-index stale files before querying
    crate::autoreindex::ensure_fresh(&opts.root, &store, &search_index)?;

    // Check for stale cursor
    if let Some(ref cursor) = opts.cursor {
        let gen = store.generation()?;
        if let Some(cursor_gen) = parse_cursor_generation(cursor) {
            if cursor_gen != gen {
                let resp = ThinResponse::error(
                    "search",
                    opts.max_bytes,
                    models::ERR_CURSOR_STALE,
                    format!(
                        "Cursor generation {cursor_gen} != current {gen}. Re-query without cursor."
                    ),
                    Some(vec![serde_json::json!(["search", {"query": opts.query}])]),
                );
                println!("{}", serde_json::to_string(&resp)?);
                return Ok(());
            }
        }
    }

    // Search
    let hits = search_index.search(
        &opts.query,
        opts.limit,
        opts.path_filter.as_deref(),
        opts.lang_filter.as_deref(),
    )?;

    // Build thin response items
    // search tuple: [symbol_id, name, path, range, score, why[], preview]
    let items: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            // Get range from store
            let range_str = store
                .get_block(&h.symbol_id)
                .ok()
                .flatten()
                .map(|b| {
                    format!(
                        "{}:{}-{}:{}",
                        b.start_line, b.start_col, b.end_line, b.end_col
                    )
                })
                .unwrap_or_default();

            serde_json::json!([
                h.symbol_id,
                h.name,
                h.path,
                range_str,
                (h.score * 10.0).round() / 10.0,
                h.matched_fields,
                h.preview.lines().take(3).collect::<Vec<_>>().join("\n"),
            ])
        })
        .collect();

    let mut resp = ThinResponse::success("search", opts.max_bytes, items);

    // Add hint for top result
    if let Some(top) = hits.first() {
        resp.h = Some(vec![
            serde_json::json!(["open", {"symbol_id": top.symbol_id}]),
        ]);
    }

    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn parse_cursor_generation(cursor: &str) -> Option<u64> {
    // Cursor format: base64-encoded JSON {"g": generation, "o": offset}
    let decoded = base64_decode(cursor)?;
    let obj: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    obj.get("g")?.as_u64()
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    // Simple base64 decode (no padding required)
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;

    for &b in input.as_bytes() {
        if b == b'=' {
            break;
        }
        let val = table.iter().position(|&c| c == b)? as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Some(output)
}
