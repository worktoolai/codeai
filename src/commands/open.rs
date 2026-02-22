use anyhow::Result;
use std::path::PathBuf;

use crate::models::{self, parse_symbol_id, ThinResponse};
use crate::store::Store;

pub struct OpenOpts {
    pub root: PathBuf,
    pub symbol: Option<String>,
    pub symbols: Option<Vec<String>>,
    pub range: Option<String>,
    pub preview_lines: usize,
    pub max_bytes: u64,
    pub fmt: String,
}

pub fn run(opts: OpenOpts) -> Result<()> {
    let codeai_dir = opts.root.join(".worktoolai").join("codeai");
    let db_path = codeai_dir.join("index.db");

    if opts.range.is_some() {
        return run_range_open(&opts);
    }

    if !db_path.exists() {
        let resp = ThinResponse::error(
            "open",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "No index found. Run 'codeai index' first.".into(),
            Some(vec![serde_json::json!(["index", {}])]),
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let store = Store::open(&db_path)?;

    if let Some(ref symbol_id) = opts.symbol {
        return run_single_open(&opts, &store, symbol_id);
    }

    if let Some(ref symbols) = opts.symbols {
        return run_batch_open(&opts, &store, symbols);
    }

    let resp = ThinResponse::error(
        "open",
        opts.max_bytes,
        "INVALID_ARGS",
        "Provide --symbol, --symbols, or --range".into(),
        None,
    );
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn run_single_open(opts: &OpenOpts, store: &Store, symbol_id: &str) -> Result<()> {
    let block = match store.get_block(symbol_id)? {
        Some(b) => b,
        None => {
            // Stale ID fallback: parse symbol_id and search
            if let Some((path, kind, name, _)) = parse_symbol_id(symbol_id) {
                let candidates = store.find_blocks(&path, &kind, &name)?;
                match candidates.len() {
                    0 => {
                        let resp = ThinResponse::error(
                            "open",
                            opts.max_bytes,
                            models::ERR_SYMBOL_NOT_FOUND,
                            format!("symbol_id '{symbol_id}' not found in current index"),
                            Some(vec![serde_json::json!(["search", {"query": name}])]),
                        );
                        println!("{}", serde_json::to_string(&resp)?);
                        return Ok(());
                    }
                    1 => candidates.into_iter().next().unwrap(),
                    _ => {
                        let candidate_ids: Vec<_> = candidates.iter().map(|c| c.symbol_id.clone()).collect();
                        let resp = ThinResponse::error(
                            "open",
                            opts.max_bytes,
                            models::ERR_SYMBOL_AMBIGUOUS,
                            format!("Multiple candidates for '{symbol_id}'"),
                            Some(vec![serde_json::json!(["open", {"symbols": candidate_ids}])]),
                        );
                        println!("{}", serde_json::to_string(&resp)?);
                        return Ok(());
                    }
                }
            } else {
                let resp = ThinResponse::error(
                    "open",
                    opts.max_bytes,
                    models::ERR_SYMBOL_NOT_FOUND,
                    format!("symbol_id '{symbol_id}' not found and cannot be parsed"),
                    Some(vec![serde_json::json!(["search", {"query": symbol_id}])]),
                );
                println!("{}", serde_json::to_string(&resp)?);
                return Ok(());
            }
        }
    };

    // Read the actual file content
    let file_path = opts.root.join(&block.path);
    let content = match std::fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(_) => {
            let resp = ThinResponse::error(
                "open",
                opts.max_bytes,
                models::ERR_FILE_NOT_FOUND,
                format!("File '{}' not found on disk", block.path),
                Some(vec![serde_json::json!(["search", {"query": block.name}])]),
            );
            println!("{}", serde_json::to_string(&resp)?);
            return Ok(());
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let start = block.start_line as usize;
    let end = (block.end_line as usize + 1).min(lines.len());

    if start >= lines.len() {
        let resp = ThinResponse::error(
            "open",
            opts.max_bytes,
            models::ERR_RANGE_OUT_OF_BOUNDS,
            format!("Block range {}:{} exceeds file length {}", block.start_line, block.end_line, lines.len()),
            Some(vec![serde_json::json!(["outline", {"path": block.path}])]),
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let block_content = &lines[start..end];
    let limited: Vec<&str> = block_content.iter().take(opts.preview_lines).copied().collect();
    let content_str = limited.join("\n");

    let range_str = format!("{}:{}-{}:{}", block.start_line, block.start_col, block.end_line, block.end_col);

    // open tuple: [symbol_id, name, path, range, signature?, doc?, content_or_preview]
    let item = serde_json::json!([
        block.symbol_id,
        block.name,
        block.path,
        range_str,
        block.signature,
        block.doc,
        content_str,
    ]);

    let resp = ThinResponse::success("open", opts.max_bytes, vec![item]);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn run_batch_open(opts: &OpenOpts, store: &Store, symbols: &[String]) -> Result<()> {
    let mut items = Vec::new();
    let mut remaining = Vec::new();
    let mut total_bytes: u64 = 0;

    for symbol_id in symbols {
        let block = match store.get_block(symbol_id)? {
            Some(b) => b,
            None => {
                remaining.push(symbol_id.clone());
                continue;
            }
        };

        let file_path = opts.root.join(&block.path);
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => {
                remaining.push(symbol_id.clone());
                continue;
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let start = block.start_line as usize;
        let end = (block.end_line as usize + 1).min(lines.len());

        if start >= lines.len() {
            remaining.push(symbol_id.clone());
            continue;
        }

        let block_content = &lines[start..end];
        let limited: Vec<&str> = block_content.iter().take(opts.preview_lines).copied().collect();
        let content_str = limited.join("\n");
        let content_bytes = content_str.len() as u64;

        // Check budget
        if total_bytes + content_bytes > opts.max_bytes {
            remaining.push(symbol_id.clone());
            // Add remaining symbols
            continue;
        }

        total_bytes += content_bytes;

        let range_str = format!("{}:{}-{}:{}", block.start_line, block.start_col, block.end_line, block.end_col);

        items.push(serde_json::json!([
            block.symbol_id,
            block.name,
            block.path,
            range_str,
            block.signature,
            block.doc,
            content_str,
        ]));
    }

    let mut resp = ThinResponse::success("open", opts.max_bytes, items);
    resp.m.byte_count = total_bytes;

    if !remaining.is_empty() {
        resp.m.truncated = true;
        resp.remaining = Some(remaining);
    }

    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn run_range_open(opts: &OpenOpts) -> Result<()> {
    let range_str = opts.range.as_ref().unwrap();

    // Parse range: path:L:C-L:C
    let (path, range) = match range_str.rsplit_once(':') {
        Some(_) => parse_range_arg(range_str)?,
        None => {
            let resp = ThinResponse::error(
                "open",
                opts.max_bytes,
                "INVALID_ARGS",
                format!("Invalid range format: '{range_str}'. Expected: path:L:C-L:C"),
                None,
            );
            println!("{}", serde_json::to_string(&resp)?);
            return Ok(());
        }
    };

    let file_path = opts.root.join(&path);
    let content = match std::fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(_) => {
            let resp = ThinResponse::error(
                "open",
                opts.max_bytes,
                models::ERR_FILE_NOT_FOUND,
                format!("File '{path}' not found on disk"),
                None,
            );
            println!("{}", serde_json::to_string(&resp)?);
            return Ok(());
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let (start_line, end_line) = range;

    if start_line >= lines.len() {
        let resp = ThinResponse::error(
            "open",
            opts.max_bytes,
            models::ERR_RANGE_OUT_OF_BOUNDS,
            format!("Start line {} exceeds file length {}", start_line, lines.len()),
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let end = (end_line + 1).min(lines.len());
    let content_str = lines[start_line..end].join("\n");

    let item = serde_json::json!([
        null,
        null,
        path,
        format!("{start_line}:0-{end_line}:0"),
        null,
        null,
        content_str,
    ]);

    let resp = ThinResponse::success("open", opts.max_bytes, vec![item]);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

/// Parse "path:L:C-L:C" → (path, (start_line, end_line))
fn parse_range_arg(s: &str) -> Result<(String, (usize, usize))> {
    // Find the range part by looking for the pattern L:C-L:C at the end
    // Strategy: split from the right, looking for the dash-separated range
    // Find the range part by looking for the pattern at the end.
    // Path can contain colons, so we search for the numeric range pattern.

    // Try to find pattern: digits:digits-digits:digits at the end
    if let Some(idx) = s.rfind(|c: char| !c.is_ascii_digit() && c != ':' && c != '-') {
        let path = &s[..=idx];
        let range_part = &s[idx + 1..];

        // But we need the colon before the range
        let path = path.trim_end_matches(':');
        let range_part = range_part.trim_start_matches(':');

        if let Some((start_part, end_part)) = range_part.split_once('-') {
            let start_line: usize = start_part
                .split(':')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let end_line: usize = end_part
                .split(':')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(start_line);

            return Ok((path.to_string(), (start_line, end_line)));
        }
    }

    anyhow::bail!("Cannot parse range: {s}")
}
