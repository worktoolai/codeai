use anyhow::Result;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use crate::models;
use crate::store::Store;

use super::{validate_fmt, validate_nonzero};
pub struct GraphOpts {
    pub root: PathBuf,
    pub path: String,
    pub depth: usize,
    pub limit: usize,
    pub offset: usize,
    pub external: bool,
    pub max_bytes: u64,
    pub fmt: String,
}

struct Edge {
    from: String,
    to: Option<String>,
    raw_import: String,
    kind: String,
}

struct GraphResult {
    entry: String,
    edges: Vec<Edge>,
    file_count: usize,
    cycle_count: usize,
    external_count: usize,
    max_depth: usize,
}

pub fn run(opts: GraphOpts) -> Result<()> {
    if let Err(message) = validate_fmt(&opts.fmt, &["tree", "thin"]) {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    if let Err(message) = validate_nonzero("depth", opts.depth as u64) {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    if let Err(message) = validate_nonzero("limit", opts.limit as u64) {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    if let Err(message) = validate_nonzero("max-bytes", opts.max_bytes) {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_PARSE_ERROR,
            message,
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let codeai_dir = opts.root.join(".worktoolai").join("codeai");
    let db_path = codeai_dir.join("index.db");

    if !db_path.exists() {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "index not found — run `codeai index` first".to_string(),
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let store = Store::open(&db_path)?;

    if store.block_count()? == 0 {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "index is empty — run `codeai index` first".to_string(),
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    // Normalize path
    let entry_path = opts
        .path
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();

    // Check file exists in index
    if store.get_file(&entry_path)?.is_none() {
        let resp = models::ThinResponse::error(
            "graph",
            opts.max_bytes,
            models::ERR_FILE_NOT_FOUND,
            format!("file not in index: {entry_path}"),
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let graph = build_graph(&store, &entry_path, opts.depth, opts.external)?;

    match opts.fmt.as_str() {
        "thin" => print_thin(&graph, &opts)?,
        "tree" => print_tree(&graph, &opts)?,
        _ => unreachable!("fmt is pre-validated"),
    }

    Ok(())
}

fn build_graph(
    store: &Store,
    entry: &str,
    max_depth: usize,
    include_external: bool,
) -> Result<GraphResult> {
    let mut edges = Vec::new();
    let mut visited = HashSet::new();
    let mut files = HashSet::new();
    let mut cycle_count = 0usize;
    let mut external_count = 0usize;
    let mut real_depth = 0usize;

    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((entry.to_string(), 0));
    visited.insert(entry.to_string());
    files.insert(entry.to_string());

    while let Some((path, depth)) = queue.pop_front() {
        if depth > real_depth {
            real_depth = depth;
        }

        let imports = store.imports_for_file(&path)?;

        for imp in &imports {
            match &imp.resolved_path {
                Some(resolved) => {
                    if visited.contains(resolved) {
                        cycle_count += 1;
                        edges.push(Edge {
                            from: path.clone(),
                            to: Some(resolved.clone()),
                            raw_import: imp.raw_import.clone(),
                            kind: "cycle".to_string(),
                        });
                    } else {
                        edges.push(Edge {
                            from: path.clone(),
                            to: Some(resolved.clone()),
                            raw_import: imp.raw_import.clone(),
                            kind: imp.kind.clone(),
                        });
                        files.insert(resolved.clone());
                        if depth + 1 < max_depth {
                            visited.insert(resolved.clone());
                            queue.push_back((resolved.clone(), depth + 1));
                        }
                    }
                }
                None => {
                    external_count += 1;
                    if include_external {
                        edges.push(Edge {
                            from: path.clone(),
                            to: None,
                            raw_import: imp.raw_import.clone(),
                            kind: "external".to_string(),
                        });
                    }
                }
            }
        }
    }

    Ok(GraphResult {
        entry: entry.to_string(),
        edges,
        file_count: files.len(),
        cycle_count,
        external_count,
        max_depth: real_depth,
    })
}

fn print_tree(graph: &GraphResult, opts: &GraphOpts) -> Result<()> {
    let mut output = String::new();
    let mut visited = HashSet::new();
    let mut node_count = 0usize;

    // Collect edges by source
    let children = edges_by_source(&graph.edges);

    render_tree_node(
        &graph.entry,
        "",
        true,
        &children,
        &mut visited,
        &mut output,
        &mut node_count,
        0,
        opts.depth,
        opts.limit,
        opts.offset,
    );

    // Summary line
    let internal_edges = graph.edges.iter().filter(|e| e.to.is_some()).count();
    output.push_str(&format!(
        "\n{} files, {} edges, {} cycles (--limit {} --offset {})\n",
        graph.file_count, internal_edges, graph.cycle_count, opts.limit, opts.offset,
    ));

    print!("{output}");
    Ok(())
}

struct TreeChild {
    target: Option<String>,
    raw_import: String,
    kind: String,
}

fn edges_by_source(edges: &[Edge]) -> std::collections::HashMap<String, Vec<TreeChild>> {
    let mut map: std::collections::HashMap<String, Vec<TreeChild>> =
        std::collections::HashMap::new();
    for e in edges {
        map.entry(e.from.clone()).or_default().push(TreeChild {
            target: e.to.clone(),
            raw_import: e.raw_import.clone(),
            kind: e.kind.clone(),
        });
    }
    map
}

#[allow(clippy::too_many_arguments)]
fn render_tree_node(
    path: &str,
    prefix: &str,
    is_root: bool,
    children: &std::collections::HashMap<String, Vec<TreeChild>>,
    visited: &mut HashSet<String>,
    output: &mut String,
    node_count: &mut usize,
    depth: usize,
    max_depth: usize,
    limit: usize,
    offset: usize,
) {
    if is_root {
        output.push_str(path);
        output.push('\n');
    }

    visited.insert(path.to_string());

    if depth >= max_depth {
        return;
    }

    let empty = Vec::new();
    let kids = children.get(path).unwrap_or(&empty);

    for (i, child) in kids.iter().enumerate() {
        if *node_count >= offset + limit {
            return;
        }

        let is_last = i == kids.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        if *node_count >= offset {
            match &child.target {
                Some(target) => {
                    if child.kind == "cycle" || visited.contains(target) {
                        output.push_str(&format!("{prefix}{connector}{target} (cycle)\n"));
                    } else {
                        output.push_str(&format!("{prefix}{connector}{target}\n"));
                        render_tree_node(
                            target,
                            &format!("{prefix}{child_prefix}"),
                            false,
                            children,
                            visited,
                            output,
                            node_count,
                            depth + 1,
                            max_depth,
                            limit,
                            offset,
                        );
                    }
                }
                None => {
                    output.push_str(&format!("{prefix}{connector}[ext] {}\n", child.raw_import));
                }
            }
        }

        *node_count += 1;
    }
}

fn print_thin(graph: &GraphResult, opts: &GraphOpts) -> Result<()> {
    let internal_edges = graph.edges.iter().filter(|e| e.to.is_some()).count();

    let summary = serde_json::json!({
        "entry": graph.entry,
        "files": graph.file_count,
        "edges": internal_edges,
        "cycles": graph.cycle_count,
        "external": graph.external_count,
        "depth": graph.max_depth,
    });

    // Build edge tuples with pagination
    let edge_tuples: Vec<serde_json::Value> = graph
        .edges
        .iter()
        .skip(opts.offset)
        .take(opts.limit)
        .map(|e| serde_json::json!([e.from, e.to, e.raw_import, e.kind,]))
        .collect();

    let truncated = opts.offset + opts.limit < graph.edges.len();
    let byte_count = serde_json::to_string(&edge_tuples)
        .map(|s| s.len() as u64)
        .unwrap_or(0);

    let resp = models::ThinResponse {
        v: 1,
        m: models::Meta {
            cmd: "graph".to_string(),
            max_bytes: opts.max_bytes,
            byte_count,
            truncated,
            next_cursor: None,
        },
        i: Some(vec![summary, serde_json::Value::Array(edge_tuples)]),
        h: None,
        e: None,
        remaining: None,
    };

    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}
