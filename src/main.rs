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
#[command(
    name = "codeai",
    about = "Agent-first code exploration",
    after_help = r#"Workflow: index → search/outline → open
  index                  build/update block index (auto-skips unchanged files)
  search QUERY           full-text + semantic search across blocks
  outline PATH           list blocks in a file (functions, structs, etc.)
  open --symbol ID       read block content by symbol ID
Symbol ID: path#kind#name (e.g. "src/main.rs#function#main")
Block kinds: function, method, class, struct, interface, trait, enum, impl, module, namespace, block, object, protocol
Languages: go, rust, python, typescript, tsx, javascript, jsx, java, kotlin, c, cpp, csharp, swift, scala, ruby, php, bash, hcl
Output: --fmt thin (default, compact JSON) | json (pretty) | lines (one per line)
Exit: 0=ok, 1=error"#
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index the codebase
    #[command(after_help = r#"  codeai index                       # incremental index (skips unchanged)
  codeai index --full                # full reindex from scratch
  codeai index --lang rust           # only Rust files
  codeai index --path src/           # only files under src/"#)]
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
    #[command(after_help = r#"  codeai search "parse"              # search all blocks
  codeai search "validate" --lang go # only Go blocks
  codeai search "auth" --limit 5     # limit results
  codeai search "error" --path src/  # only in src/"#)]
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
    #[command(after_help = r#"  codeai outline src/main.rs                  # all blocks
  codeai outline src/main.rs --kind function  # functions only
  codeai outline src/main.rs --kind struct    # structs only
Kinds: function, method, class, struct, interface, trait, enum, impl, module, namespace, block, object, protocol"#)]
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

    /// Open (read) code blocks by symbol ID
    #[command(after_help = r#"  codeai open --symbol "src/main.rs#function#main"
  codeai open --symbols "src/a.rs#function#foo,src/b.rs#struct#Bar"
  codeai open --range "src/main.rs:10:0-25:0"
Symbol ID format: path#kind#name or path#kind#name#N (N=occurrence index)
  obtained from: search results (i[][0]), outline results (i[][0])"#)]
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
