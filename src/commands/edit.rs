use anyhow::{bail, Result};
use std::fs;
use std::io::{self, Read};

pub struct EditOpts {
    pub file: String,
    pub range: String,
    pub content: Option<String>,
    pub content_file: Option<String>,
    pub dry_run: bool,
}

/// Parse "L10-L15" or "10-15" into (start, end) 1-based inclusive
fn parse_range(range: &str) -> Result<(usize, usize)> {
    let range = range.trim();
    let parts: Vec<&str> = range.split('-').collect();
    if parts.len() != 2 {
        bail!("Invalid range format: '{}'. Expected: L10-L15 or 10-15", range);
    }
    let start: usize = parts[0].trim_start_matches('L').trim_start_matches('l').parse()
        .map_err(|_| anyhow::anyhow!("Invalid start line: '{}'", parts[0]))?;
    let end: usize = parts[1].trim_start_matches('L').trim_start_matches('l').parse()
        .map_err(|_| anyhow::anyhow!("Invalid end line: '{}'", parts[1]))?;
    if start == 0 || end == 0 {
        bail!("Line numbers are 1-based, got: {}-{}", start, end);
    }
    if start > end {
        bail!("Start line ({}) must be <= end line ({})", start, end);
    }
    Ok((start, end))
}

fn read_content(content: &Option<String>, content_file: &Option<String>) -> Result<String> {
    if let Some(c) = content {
        Ok(c.replace("\\n", "\n").replace("\\t", "\t"))
    } else if let Some(path) = content_file {
        if path == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        } else {
            Ok(fs::read_to_string(path)?)
        }
    } else {
        bail!("No content provided. Use -c or --content-file")
    }
}

pub fn run(opts: EditOpts) -> Result<()> {
    let (start, end) = parse_range(&opts.range)?;
    let original = fs::read_to_string(&opts.file)?;
    let lines: Vec<&str> = original.lines().collect();
    let total = lines.len();

    if start > total {
        bail!("Start line {} exceeds file length ({})", start, total);
    }
    let end = end.min(total);

    let new_content = read_content(&opts.content, &opts.content_file)?;

    let mut result = String::new();

    // Lines before range
    for line in &lines[..start - 1] {
        result.push_str(line);
        result.push('\n');
    }

    // New content
    result.push_str(&new_content);
    if !new_content.ends_with('\n') {
        result.push('\n');
    }

    // Lines after range
    for line in &lines[end..] {
        result.push_str(line);
        result.push('\n');
    }

    // Preserve original trailing newline behavior
    if !original.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    if opts.dry_run {
        print!("{}", result);
        return Ok(());
    }

    fs::write(&opts.file, &result)?;
    Ok(())
}
