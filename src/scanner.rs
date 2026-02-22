use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::lang;

// ── ScanResult ──

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub path: PathBuf,
    pub rel_path: String,
    pub extension: String,
    pub size: u64,
    pub mtime: u64,
}

// ── Scanner ──

pub struct Scanner {
    root: PathBuf,
    no_gitignore: bool,
    no_default_ignores: bool,
    ignore_file: Option<PathBuf>,
    lang_filter: Option<String>,
    max_file_size: u64,
}

const DEFAULT_MAX_FILE_SIZE: u64 = 1_048_576; // 1MB

const BUILT_IN_IGNORE_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    ".venv",
    "__pycache__",
    "dist",
    "build",
    "target",
    "out",
    ".git",
    ".svn",
    ".hg",
];

const BUILT_IN_IGNORE_EXTS: &[&str] = &[
    "min.js", "min.css", "map", "exe", "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "ico",
    "svg", "webp", "pdf", "zip", "tar", "gz", "bz2", "xz", "7z", "wasm", "o", "a", "lib", "bin",
];

impl Scanner {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            no_gitignore: false,
            no_default_ignores: false,
            ignore_file: None,
            lang_filter: None,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }

    pub fn no_gitignore(mut self, v: bool) -> Self {
        self.no_gitignore = v;
        self
    }

    pub fn no_default_ignores(mut self, v: bool) -> Self {
        self.no_default_ignores = v;
        self
    }

    pub fn ignore_file(mut self, path: PathBuf) -> Self {
        self.ignore_file = Some(path);
        self
    }

    pub fn lang_filter(mut self, lang: String) -> Self {
        self.lang_filter = Some(lang);
        self
    }

    pub fn max_file_size(mut self, size: u64) -> Self {
        self.max_file_size = size;
        self
    }

    pub fn scan(&self) -> Result<Vec<ScanResult>> {
        let mut builder = WalkBuilder::new(&self.root);
        builder
            .hidden(false)
            .git_ignore(!self.no_gitignore)
            .git_global(false)
            .git_exclude(false)
            .follow_links(true);

        // Add .worktoolai/codeai/ignore if it exists
        let codeai_ignore = self.root.join(".worktoolai").join("codeai").join("ignore");
        if codeai_ignore.exists() {
            builder.add_ignore(&codeai_ignore);
        }

        // Add user-specified ignore file
        if let Some(ref ignore_path) = self.ignore_file {
            builder.add_ignore(ignore_path);
        }

        // Determine which extensions to accept
        let allowed_exts: Vec<&str> = if let Some(ref lang) = self.lang_filter {
            lang::config_for_extension(lang)
                .map(|c| c.extensions.to_vec())
                .or_else(|| {
                    // Try matching by language name
                    lang::all_extensions()
                        .into_iter()
                        .filter(|ext| {
                            lang::config_for_extension(ext)
                                .map(|c| c.language == lang.as_str())
                                .unwrap_or(false)
                        })
                        .collect::<Vec<_>>()
                        .into()
                })
                .unwrap_or_default()
        } else {
            lang::all_extensions()
        };

        let mut results = Vec::new();

        for entry in builder.build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip directories
            let ft = match entry.file_type() {
                Some(ft) if ft.is_file() => ft,
                _ => continue,
            };
            let _ = ft;

            let path = entry.path();

            // Built-in directory ignores
            if !self.no_default_ignores {
                if let Some(parent) = path.parent() {
                    let skip = path.ancestors().any(|a| {
                        a.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| BUILT_IN_IGNORE_DIRS.contains(&n))
                            .unwrap_or(false)
                    });
                    if skip {
                        continue;
                    }
                    let _ = parent;
                }
            }

            // Check extension
            let ext = match path.extension().and_then(|e| e.to_str()) {
                Some(e) => e.to_lowercase(),
                None => continue,
            };

            // Built-in extension ignores
            if !self.no_default_ignores {
                let full_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if BUILT_IN_IGNORE_EXTS
                    .iter()
                    .any(|ie| full_name.ends_with(&format!(".{ie}")))
                {
                    continue;
                }
            }

            // Supported extension filter
            if !allowed_exts.contains(&ext.as_str()) {
                continue;
            }

            // File metadata
            let metadata = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Size filter
            if metadata.len() > self.max_file_size {
                continue;
            }

            let mtime = metadata
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let rel_path = normalize_path(path, &self.root);

            results.push(ScanResult {
                path: path.to_path_buf(),
                rel_path,
                extension: ext,
                size: metadata.len(),
                mtime,
            });
        }

        Ok(results)
    }
}

/// Normalize a path to use '/' separators and be relative to root.
pub fn normalize_path(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy();
    s.replace('\\', "/")
}
