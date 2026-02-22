#![allow(dead_code)]

mod commands;
mod lang;
mod models;
mod parser;
mod scanner;
mod search;
mod store;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "codeai", about = "Agent-first code exploration")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index the codebase
    Index {
        /// Full reindex (delete existing index first)
        #[arg(long)]
        full: bool,

        /// Filter by path
        #[arg(long)]
        path: Option<String>,

        /// Filter by language
        #[arg(long)]
        lang: Option<String>,

        /// Disable .gitignore respect
        #[arg(long)]
        no_gitignore: bool,

        /// Disable built-in ignore patterns
        #[arg(long)]
        no_default_ignores: bool,

        /// Additional ignore file
        #[arg(long)]
        ignore_file: Option<PathBuf>,

        /// Max output bytes
        #[arg(long, default_value = "12000")]
        max_bytes: u64,

        /// Output format: thin, json, lines
        #[arg(long, default_value = "thin")]
        fmt: String,
    },

    /// Search for code blocks
    Search {
        /// Search query
        query: String,

        /// Max results
        #[arg(long, default_value = "10")]
        limit: usize,

        /// Filter by path prefix
        #[arg(long)]
        path: Option<String>,

        /// Filter by language
        #[arg(long)]
        lang: Option<String>,

        /// Max output bytes
        #[arg(long, default_value = "12000")]
        max_bytes: u64,

        /// Pagination cursor
        #[arg(long)]
        cursor: Option<String>,

        /// Output format
        #[arg(long, default_value = "thin")]
        fmt: String,
    },

    /// List blocks in a file
    Outline {
        /// File path (project-relative)
        path: String,

        /// Filter by block kind
        #[arg(long)]
        kind: Option<String>,

        /// Max results
        #[arg(long, default_value = "100")]
        limit: usize,

        /// Max output bytes
        #[arg(long, default_value = "12000")]
        max_bytes: u64,

        /// Pagination cursor
        #[arg(long)]
        cursor: Option<String>,

        /// Output format
        #[arg(long, default_value = "thin")]
        fmt: String,
    },

    /// Open (read) code blocks
    Open {
        /// Single symbol ID
        #[arg(long)]
        symbol: Option<String>,

        /// Multiple symbol IDs (comma-separated)
        #[arg(long, value_delimiter = ',')]
        symbols: Option<Vec<String>>,

        /// Range: path:L:C-L:C
        #[arg(long)]
        range: Option<String>,

        /// Preview lines per block
        #[arg(long, default_value = "80")]
        preview_lines: usize,

        /// Max output bytes
        #[arg(long, default_value = "16000")]
        max_bytes: u64,

        /// Output format
        #[arg(long, default_value = "thin")]
        fmt: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let result = match cli.command {
        Commands::Index {
            full,
            path,
            lang,
            no_gitignore,
            no_default_ignores,
            ignore_file,
            max_bytes,
            fmt,
        } => commands::index::run(commands::index::IndexOpts {
            root,
            full,
            path_filter: path,
            lang_filter: lang,
            no_gitignore,
            no_default_ignores,
            ignore_file,
            max_bytes,
            fmt,
        }),

        Commands::Search {
            query,
            limit,
            path,
            lang,
            max_bytes,
            cursor,
            fmt,
        } => commands::search::run(commands::search::SearchOpts {
            root,
            query,
            limit,
            path_filter: path,
            lang_filter: lang,
            max_bytes,
            cursor,
            fmt,
        }),

        Commands::Outline {
            path,
            kind,
            limit,
            max_bytes,
            cursor,
            fmt,
        } => commands::outline::run(commands::outline::OutlineOpts {
            root,
            path,
            kind_filter: kind,
            limit,
            max_bytes,
            cursor,
            fmt,
        }),

        Commands::Open {
            symbol,
            symbols,
            range,
            preview_lines,
            max_bytes,
            fmt,
        } => commands::open::run(commands::open::OpenOpts {
            root,
            symbol,
            symbols,
            range,
            preview_lines,
            max_bytes,
            fmt,
        }),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
