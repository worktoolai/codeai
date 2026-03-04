pub mod edit;
pub mod graph;
pub mod index;
pub mod open;
pub mod outline;
pub mod project;
pub mod search;
pub mod write;

pub fn validate_nonzero(name: &str, value: u64) -> Result<(), String> {
    if value == 0 {
        return Err(format!("invalid {name}: expected > 0"));
    }
    Ok(())
}

pub fn validate_fmt(fmt: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&fmt) {
        return Ok(());
    }

    Err(format!(
        "unsupported fmt '{fmt}': expected {}",
        allowed.join("|")
    ))
}

pub fn validate_lang_filter(lang: &str) -> bool {
    let normalized = lang.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if crate::lang::config_for_extension(&normalized).is_some() {
        return true;
    }

    crate::lang::all_extensions().into_iter().any(|ext| {
        crate::lang::config_for_extension(ext)
            .map(|config| config.language == normalized)
            .unwrap_or(false)
    })
}
