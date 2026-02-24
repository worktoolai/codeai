use anyhow::Result;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::models;
use crate::store::Store;

use super::{validate_fmt, validate_nonzero};
const ENTRYPOINT_BASENAMES: &[&str] = &[
    "main.rs",
    "main.go",
    "main.py",
    "__main__.py",
    "index.ts",
    "index.js",
    "app.py",
    "server.js",
];

pub struct ProjectOpts {
    pub root: PathBuf,
    pub path_filter: Option<String>,
    pub max_bytes: u64,
    pub fmt: String,
}

#[derive(Debug, Clone)]
struct Entrypoint {
    path: String,
    confidence: &'static str,
}

pub fn run(opts: ProjectOpts) -> Result<()> {
    if let Err(message) = validate_fmt(&opts.fmt, &["thin"]) {
        let resp = models::ThinResponse::error(
            "project.get",
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
            "project.get",
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
            "project.get",
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
            "project.get",
            opts.max_bytes,
            models::ERR_INDEX_EMPTY,
            "index is empty — run `codeai index` first".to_string(),
            None,
        );
        println!("{}", serde_json::to_string(&resp)?);
        return Ok(());
    }

    let all_paths = store.all_file_paths()?;
    let all_imports = store.all_imports()?;

    let all_paths: Vec<String> = match &opts.path_filter {
        Some(prefix) => all_paths.into_iter().filter(|p| p.starts_with(prefix.as_str())).collect(),
        None => all_paths,
    };
    let all_imports: Vec<crate::store::ImportRow> = match &opts.path_filter {
        Some(prefix) => all_imports.into_iter().filter(|imp| {
            imp.path.starts_with(prefix.as_str())
                && imp.resolved_path.as_ref().map_or(true, |rp| rp.starts_with(prefix.as_str()))
        }).collect(),
        None => all_imports,
    };

    let (adj, rev_adj) = build_graph(&all_paths, &all_imports);
    let entrypoints = detect_entrypoints(&all_paths, &adj, &rev_adj);

    let mut reach_by_entry: HashMap<String, HashSet<String>> = HashMap::new();
    for ep in &entrypoints {
        let reach = bfs_reachable(&adj, &ep.path);
        reach_by_entry.insert(ep.path.clone(), reach);
    }

    let (entry_files, shared, orphan) = classify_files(&all_paths, &entrypoints, &reach_by_entry);

    let summary = serde_json::json!({
        "total_files": all_paths.len(),
        "entrypoints": entrypoints.len(),
        "shared": shared.len(),
        "orphan": orphan.len(),
    });

    let entry_items: Vec<serde_json::Value> = entrypoints
        .iter()
        .map(|ep| {
            let files = entry_files.get(&ep.path).cloned().unwrap_or_default();
            serde_json::json!({
                "path": ep.path,
                "confidence": ep.confidence,
                "file_count": files.len(),
                "files": files,
            })
        })
        .collect();

    let items = vec![
        summary,
        serde_json::Value::Array(entry_items),
        serde_json::Value::Array(shared.into_iter().map(serde_json::Value::String).collect()),
        serde_json::Value::Array(orphan.into_iter().map(serde_json::Value::String).collect()),
    ];

    let resp = models::ThinResponse::success("project.get", opts.max_bytes, items);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn build_graph(
    all_paths: &[String],
    all_imports: &[crate::store::ImportRow],
) -> (HashMap<String, Vec<String>>, HashMap<String, Vec<String>>) {
    let path_set: HashSet<&str> = all_paths.iter().map(|p| p.as_str()).collect();

    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut rev_adj: HashMap<String, Vec<String>> = HashMap::new();

    for path in all_paths {
        adj.entry(path.clone()).or_default();
        rev_adj.entry(path.clone()).or_default();
    }

    for imp in all_imports {
        let Some(to) = &imp.resolved_path else {
            continue;
        };

        if !path_set.contains(imp.path.as_str()) || !path_set.contains(to.as_str()) {
            continue;
        }

        adj.entry(imp.path.clone()).or_default().push(to.clone());
        rev_adj
            .entry(to.clone())
            .or_default()
            .push(imp.path.clone());
    }

    for list in adj.values_mut() {
        list.sort();
        list.dedup();
    }
    for list in rev_adj.values_mut() {
        list.sort();
        list.dedup();
    }

    (adj, rev_adj)
}

fn detect_entrypoints(
    all_paths: &[String],
    adj: &HashMap<String, Vec<String>>,
    rev_adj: &HashMap<String, Vec<String>>,
) -> Vec<Entrypoint> {
    let mut by_path: HashMap<String, (&'static str, u8)> = HashMap::new();

    for path in all_paths {
        let filename_hit = is_entrypoint_filename(path);
        let in_degree = rev_adj.get(path).map(|v| v.len()).unwrap_or(0);
        let out_degree = adj.get(path).map(|v| v.len()).unwrap_or(0);
        let graph_hit = in_degree == 0 && out_degree > 0;

        let (confidence, rank) = match (filename_hit, graph_hit) {
            (true, true) => ("both", 3),
            (true, false) => ("filename", 2),
            (false, true) => ("graph", 1),
            (false, false) => continue,
        };

        match by_path.get(path) {
            Some((_, existing_rank)) if *existing_rank >= rank => {}
            _ => {
                by_path.insert(path.clone(), (confidence, rank));
            }
        }
    }

    let mut out: Vec<Entrypoint> = by_path
        .into_iter()
        .map(|(path, (confidence, _))| Entrypoint { path, confidence })
        .collect();

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn is_entrypoint_filename(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    ENTRYPOINT_BASENAMES.contains(&basename)
}

fn bfs_reachable(adj: &HashMap<String, Vec<String>>, start: &str) -> HashSet<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    visited.insert(start.to_string());
    queue.push_back(start.to_string());

    while let Some(node) = queue.pop_front() {
        if let Some(nexts) = adj.get(&node) {
            for next in nexts {
                if visited.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
    }

    visited
}

fn classify_files(
    all_paths: &[String],
    entrypoints: &[Entrypoint],
    reach_by_entry: &HashMap<String, HashSet<String>>,
) -> (HashMap<String, Vec<String>>, Vec<String>, Vec<String>) {
    let mut entry_files: HashMap<String, Vec<String>> = entrypoints
        .iter()
        .map(|ep| (ep.path.clone(), Vec::new()))
        .collect();

    let mut shared = Vec::new();
    let mut orphan = Vec::new();

    for path in all_paths {
        let owners: Vec<&str> = entrypoints
            .iter()
            .filter_map(|ep| {
                reach_by_entry.get(&ep.path).and_then(|set| {
                    if set.contains(path) {
                        Some(ep.path.as_str())
                    } else {
                        None
                    }
                })
            })
            .collect();

        match owners.len() {
            0 => orphan.push(path.clone()),
            1 => {
                if let Some(files) = entry_files.get_mut(owners[0]) {
                    files.push(path.clone());
                }
            }
            _ => shared.push(path.clone()),
        }
    }

    for files in entry_files.values_mut() {
        files.sort();
    }
    shared.sort();
    orphan.sort();

    (entry_files, shared, orphan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_entrypoints_merge_and_sort() {
        let all_paths = vec![
            "a/main.rs".to_string(),
            "b/lib.rs".to_string(),
            "c/index.ts".to_string(),
            "d/worker.rs".to_string(),
            "e/shared.rs".to_string(),
        ];

        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        let mut rev: HashMap<String, Vec<String>> = HashMap::new();
        for p in &all_paths {
            adj.insert(p.clone(), Vec::new());
            rev.insert(p.clone(), Vec::new());
        }

        adj.get_mut("a/main.rs")
            .unwrap()
            .push("e/shared.rs".to_string());
        rev.get_mut("e/shared.rs")
            .unwrap()
            .push("a/main.rs".to_string());

        adj.get_mut("d/worker.rs")
            .unwrap()
            .push("e/shared.rs".to_string());
        rev.get_mut("e/shared.rs")
            .unwrap()
            .push("d/worker.rs".to_string());

        let eps = detect_entrypoints(&all_paths, &adj, &rev);
        let got: Vec<(String, &'static str)> =
            eps.into_iter().map(|e| (e.path, e.confidence)).collect();

        assert_eq!(
            got,
            vec![
                ("a/main.rs".to_string(), "both"),
                ("c/index.ts".to_string(), "filename"),
                ("d/worker.rs".to_string(), "graph"),
            ]
        );
    }

    #[test]
    fn test_bfs_reachable_cycle_terminates() {
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        adj.insert("a".to_string(), vec!["b".to_string()]);
        adj.insert("b".to_string(), vec!["c".to_string()]);
        adj.insert("c".to_string(), vec!["a".to_string()]);

        let reach = bfs_reachable(&adj, "a");
        assert_eq!(reach.len(), 3);
        assert!(reach.contains("a"));
        assert!(reach.contains("b"));
        assert!(reach.contains("c"));
    }

    #[test]
    fn test_classify_shared_and_orphan() {
        let all_paths = vec![
            "entry/a.rs".to_string(),
            "entry/b.rs".to_string(),
            "x.rs".to_string(),
            "y.rs".to_string(),
            "z.rs".to_string(),
        ];

        let entrypoints = vec![
            Entrypoint {
                path: "entry/a.rs".to_string(),
                confidence: "filename",
            },
            Entrypoint {
                path: "entry/b.rs".to_string(),
                confidence: "filename",
            },
        ];

        let mut reach_by_entry: HashMap<String, HashSet<String>> = HashMap::new();
        reach_by_entry.insert(
            "entry/a.rs".to_string(),
            ["entry/a.rs", "x.rs", "y.rs"]
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        );
        reach_by_entry.insert(
            "entry/b.rs".to_string(),
            ["entry/b.rs", "y.rs"]
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        );

        let (entry_files, shared, orphan) =
            classify_files(&all_paths, &entrypoints, &reach_by_entry);

        assert_eq!(shared, vec!["y.rs".to_string()]);
        assert_eq!(orphan, vec!["z.rs".to_string()]);
        assert_eq!(
            entry_files.get("entry/a.rs").unwrap(),
            &vec!["entry/a.rs".to_string(), "x.rs".to_string()]
        );
        assert_eq!(
            entry_files.get("entry/b.rs").unwrap(),
            &vec!["entry/b.rs".to_string()]
        );
    }
}
