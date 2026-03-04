use anyhow::{bail, Result};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

pub struct WriteOpts {
    pub file: String,
    pub content: Option<String>,
    pub content_file: Option<String>,
    pub dry_run: bool,
}

pub fn run(opts: WriteOpts) -> Result<()> {
    let content = if let Some(c) = opts.content {
        c.replace("\\n", "\n").replace("\\t", "\t")
    } else if let Some(path) = opts.content_file {
        if path == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            fs::read_to_string(&path)?
        }
    } else {
        bail!("No content provided. Use -c or --content-file")
    };

    if opts.dry_run {
        print!("{}", content);
        return Ok(());
    }

    if let Some(parent) = Path::new(&opts.file).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(&opts.file, &content)?;
    Ok(())
}
