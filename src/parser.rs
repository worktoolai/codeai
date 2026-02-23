use anyhow::{Context, Result};
use std::collections::HashSet;
use tree_sitter::{Language, Node, Parser};

use crate::lang::{ImportExtractor, ImportNodeConfig};

const PREVIEW_LINES: usize = 20;
const MAX_STRINGS: usize = 20;
const MAX_STRING_LEN: usize = 200;

fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    &text[..end]
}

// ── ExtractedBlock ──

#[derive(Debug, Clone)]
pub struct ExtractedBlock {
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub preview: String,
    pub strings: Vec<String>,
}

// ── Node kind → BlockKind mapping ──

fn map_node_kind(node_kind: &str) -> &str {
    match node_kind {
        "function_declaration" | "function_definition" | "function_item" => "function",
        "method_declaration" | "method_definition" | "method" => "method",
        "arrow_function" => "function",
        "class_declaration" | "class_definition" | "class_specifier" | "class" => "class",
        "struct_item" | "struct_specifier" => "struct",
        "interface_declaration" => "interface",
        "trait_item" | "trait_definition" => "trait",
        "enum_item" => "enum",
        "impl_item" => "impl",
        "module" | "namespace_definition" => "module",
        "object_declaration" | "object_definition" => "object",
        "protocol_declaration" => "protocol",
        "type_declaration" => "class",
        "block" => "block",
        other => other,
    }
}

// ── Main extraction ──

pub fn extract_blocks(
    source: &[u8],
    _rel_path: &str,
    _language: &str,
    ts_language: Language,
    function_nodes: &[&str],
    class_nodes: &[&str],
) -> Result<Vec<ExtractedBlock>> {
    let mut parser = Parser::new();
    parser.set_language(&ts_language).context("set_language")?;

    let tree = parser
        .parse(source, None)
        .context("tree-sitter parse failed")?;

    let source_text = String::from_utf8_lossy(source);
    let lines: Vec<&str> = source_text.lines().collect();

    let all_target_nodes: Vec<&str> = function_nodes
        .iter()
        .chain(class_nodes.iter())
        .copied()
        .collect();

    let mut blocks = Vec::new();
    collect_blocks(
        tree.root_node(),
        source,
        &lines,
        &all_target_nodes,
        class_nodes,
        &mut blocks,
        0,
    );

    Ok(blocks)
}

fn collect_blocks(
    node: Node,
    source: &[u8],
    lines: &[&str],
    target_nodes: &[&str],
    class_nodes: &[&str],
    blocks: &mut Vec<ExtractedBlock>,
    depth: usize,
) {
    // Only go 2 levels deep (top-level + methods inside classes)
    if depth > 2 {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        if target_nodes.contains(&kind) {
            if let Some(block) = extract_one_block(child, source, lines) {
                blocks.push(block);
            }

            // If it's a class-like node, recurse into it for methods
            if class_nodes.contains(&kind) {
                collect_blocks(
                    child,
                    source,
                    lines,
                    target_nodes,
                    class_nodes,
                    blocks,
                    depth + 1,
                );
            }
        } else {
            // Recurse into non-target nodes to find nested targets
            collect_blocks(
                child,
                source,
                lines,
                target_nodes,
                class_nodes,
                blocks,
                depth + 1,
            );
        }
    }
}

fn extract_one_block(node: Node, source: &[u8], lines: &[&str]) -> Option<ExtractedBlock> {
    let kind_str = map_node_kind(node.kind());
    let name = extract_name(node, source);
    let start = node.start_position();
    let end = node.end_position();

    let signature = extract_signature(node, source);
    let doc = extract_doc(node, source);
    let preview = extract_preview(start.row, end.row, lines);
    let strings = extract_strings(node, source);

    Some(ExtractedBlock {
        kind: kind_str.to_string(),
        name,
        start_line: start.row as u32,
        start_col: start.column as u32,
        end_line: end.row as u32,
        end_col: end.column as u32,
        signature,
        doc,
        preview,
        strings,
    })
}

// ── Name extraction ──

fn extract_name(node: Node, source: &[u8]) -> String {
    // Try direct "name" field
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(text) = name_node.utf8_text(source) {
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }

    // Arrow function: check parent for variable_declarator/pair/assignment
    if node.kind() == "arrow_function" || node.kind() == "function" {
        if let Some(parent) = node.parent() {
            match parent.kind() {
                "variable_declarator" | "public_field_definition" => {
                    if let Some(name_node) = parent.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            return text.to_string();
                        }
                    }
                }
                "pair" => {
                    if let Some(key_node) = parent.child_by_field_name("key") {
                        if let Ok(text) = key_node.utf8_text(source) {
                            return text.to_string();
                        }
                    }
                }
                "assignment_expression" => {
                    if let Some(left_node) = parent.child_by_field_name("left") {
                        if let Ok(text) = left_node.utf8_text(source) {
                            return text.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    "<anon>".to_string()
}

// ── Signature extraction ──

fn extract_signature(node: Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    // Take up to the first '{', ':', or newline
    let sig = text.lines().next().unwrap_or(text);

    // If the first line has '{', take up to it
    let sig = if let Some(pos) = sig.find('{') {
        sig[..pos].trim()
    } else {
        sig.trim()
    };

    if sig.is_empty() {
        None
    } else {
        Some(sig.to_string())
    }
}

// ── Doc extraction ──

fn extract_doc(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut sibling = node.prev_sibling();

    while let Some(sib) = sibling {
        let kind = sib.kind();
        if kind.contains("comment") {
            if let Ok(text) = sib.utf8_text(source) {
                comments.push(text.to_string());
            }
            sibling = sib.prev_sibling();
        } else {
            break;
        }
    }

    if comments.is_empty() {
        return None;
    }

    comments.reverse();
    Some(comments.join("\n"))
}

// ── Preview extraction ──

fn extract_preview(start_line: usize, end_line: usize, lines: &[&str]) -> String {
    let end = (start_line + PREVIEW_LINES)
        .min(end_line + 1)
        .min(lines.len());
    if start_line >= lines.len() {
        return String::new();
    }
    lines[start_line..end].join("\n")
}

// ── String literal extraction ──

fn extract_strings(node: Node, source: &[u8]) -> Vec<String> {
    let mut strings = Vec::new();
    collect_string_nodes(node, source, &mut strings);
    strings
}

fn collect_string_nodes(node: Node, source: &[u8], strings: &mut Vec<String>) {
    if strings.len() >= MAX_STRINGS {
        return;
    }

    let kind = node.kind();
    if matches!(
        kind,
        "string_literal"
            | "interpreted_string_literal"
            | "string"
            | "template_string"
            | "string_content"
            | "raw_string_literal"
    ) {
        if let Ok(text) = node.utf8_text(source) {
            strings.push(truncate_utf8(text, MAX_STRING_LEN).to_string());
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_string_nodes(child, source, strings);
    }
}

// ── Import extraction ──

#[derive(Debug, Clone)]
pub struct ExtractedImport {
    pub raw_import: String,
    pub kind: String,
}

pub fn extract_imports(
    source: &[u8],
    ts_language: Language,
    import_nodes: &[ImportNodeConfig],
) -> Result<Vec<ExtractedImport>> {
    if import_nodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut parser = Parser::new();
    parser.set_language(&ts_language).context("set_language")?;

    let tree = parser
        .parse(source, None)
        .context("tree-sitter parse failed")?;

    let mut imports = Vec::new();
    collect_imports(tree.root_node(), source, import_nodes, &mut imports);
    Ok(imports)
}

fn collect_imports(
    node: Node,
    source: &[u8],
    import_nodes: &[ImportNodeConfig],
    imports: &mut Vec<ExtractedImport>,
) {
    let kind = node.kind();

    for config in import_nodes {
        if kind == config.node_type {
            match config.extractor {
                ImportExtractor::Field(field) => {
                    extract_field_import(node, source, field, imports);
                }
                ImportExtractor::StringField(field) => {
                    extract_string_field_import(node, source, field, imports);
                }
                ImportExtractor::GoImport => {
                    extract_go_imports(node, source, imports);
                }
                ImportExtractor::JavaImport => {
                    extract_java_import(node, source, imports);
                }
                ImportExtractor::CInclude => {
                    extract_c_include(node, source, imports);
                }
                ImportExtractor::RubyRequire => {
                    extract_ruby_require(node, source, imports);
                }
                ImportExtractor::BashSource => {
                    extract_bash_source(node, source, imports);
                }
            }
            return;
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports(child, source, import_nodes, imports);
    }
}

fn extract_field_import(
    node: Node,
    source: &[u8],
    field: &str,
    imports: &mut Vec<ExtractedImport>,
) {
    if let Some(arg_node) = node.child_by_field_name(field) {
        if let Ok(text) = arg_node.utf8_text(source) {
            let raw = text.trim().to_string();
            if !raw.is_empty() {
                imports.push(ExtractedImport {
                    raw_import: raw,
                    kind: "module".to_string(),
                });
            }
        }
    }
}

fn extract_string_field_import(
    node: Node,
    source: &[u8],
    field: &str,
    imports: &mut Vec<ExtractedImport>,
) {
    if let Some(src_node) = node.child_by_field_name(field) {
        if let Ok(text) = src_node.utf8_text(source) {
            let raw = text.trim_matches(|c| c == '\'' || c == '"').to_string();
            if !raw.is_empty() {
                imports.push(ExtractedImport {
                    raw_import: raw,
                    kind: "module".to_string(),
                });
            }
        }
    }
}

fn extract_go_imports(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" || child.kind() == "import_spec_list" {
            extract_go_import_specs(child, source, imports);
        }
    }
}

fn extract_go_import_specs(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    if node.kind() == "import_spec" {
        if let Some(path_node) = node.child_by_field_name("path") {
            if let Ok(text) = path_node.utf8_text(source) {
                let raw = text.trim_matches('"').to_string();
                if !raw.is_empty() {
                    imports.push(ExtractedImport {
                        raw_import: raw,
                        kind: "module".to_string(),
                    });
                }
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_go_import_specs(child, source, imports);
    }
}

fn extract_java_import(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    if let Ok(text) = node.utf8_text(source) {
        let raw = text
            .trim()
            .trim_start_matches("import")
            .trim_start_matches("static")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        if !raw.is_empty() {
            imports.push(ExtractedImport {
                raw_import: raw,
                kind: "module".to_string(),
            });
        }
    }
}

fn extract_c_include(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    if let Some(path_node) = node.child_by_field_name("path") {
        if let Ok(text) = path_node.utf8_text(source) {
            let raw = text
                .trim_matches(|c| c == '"' || c == '<' || c == '>')
                .to_string();
            if !raw.is_empty() {
                imports.push(ExtractedImport {
                    raw_import: raw,
                    kind: "include".to_string(),
                });
            }
        }
    }
}

fn extract_ruby_require(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    // Only match call nodes where the method is "require" or "require_relative"
    if let Some(method_node) = node.child_by_field_name("method") {
        if let Ok(method_name) = method_node.utf8_text(source) {
            if method_name != "require" && method_name != "require_relative" {
                return;
            }
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut cursor = args.walk();
                for child in args.children(&mut cursor) {
                    if child.kind().contains("string") {
                        if let Ok(text) = child.utf8_text(source) {
                            let raw = text.trim_matches(|c| c == '\'' || c == '"').to_string();
                            if !raw.is_empty() {
                                imports.push(ExtractedImport {
                                    raw_import: raw,
                                    kind: "require".to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

fn extract_bash_source(node: Node, source: &[u8], imports: &mut Vec<ExtractedImport>) {
    // Match command nodes where the command name is "source" or "."
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            if name != "source" && name != "." {
                return;
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "word" || child.kind() == "string" {
                    if child.id() == name_node.id() {
                        continue;
                    }
                    if let Ok(text) = child.utf8_text(source) {
                        let raw = text.trim_matches(|c| c == '\'' || c == '"').to_string();
                        if !raw.is_empty() {
                            imports.push(ExtractedImport {
                                raw_import: raw,
                                kind: "source".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
}

// ── Import path resolution ──

pub fn resolve_import(
    raw_import: &str,
    source_path: &str,
    language: &str,
    all_paths: &HashSet<String>,
) -> Option<String> {
    match language {
        "rust" => resolve_rust_import(raw_import, all_paths),
        "go" => resolve_go_import(raw_import, all_paths),
        "python" => resolve_python_import(raw_import, all_paths),
        "typescript" | "tsx" | "javascript" | "jsx" => {
            resolve_js_import(raw_import, source_path, all_paths)
        }
        "java" => resolve_java_import_path(raw_import, all_paths),
        "c" | "cpp" => resolve_c_import(raw_import, source_path, all_paths),
        "ruby" => resolve_relative_import(raw_import, source_path, &["rb"], all_paths),
        "bash" => resolve_relative_import(raw_import, source_path, &["sh", "bash"], all_paths),
        _ => None,
    }
}

fn resolve_rust_import(raw: &str, all_paths: &HashSet<String>) -> Option<String> {
    // Only handle crate-internal imports
    let path_part = if let Some(rest) = raw.strip_prefix("crate::") {
        rest
    } else if raw.starts_with("super::") || raw.starts_with("self::") {
        // TODO: relative imports need source_path context — skip for now
        return None;
    } else {
        // External crate
        return None;
    };

    // Strip trailing ::{...} or ::* or ::SomeType
    let module_path = path_part
        .split("::{")
        .next()
        .unwrap_or(path_part)
        .split("::*")
        .next()
        .unwrap_or(path_part);

    // Try: last segment might be a type/fn, so try both with and without it
    let segments: Vec<&str> = module_path.split("::").collect();

    // Try full path first, then progressively shorter
    for end in (1..=segments.len()).rev() {
        let path_str = format!("src/{}", segments[..end].join("/"));

        // Try as file
        let file_path = format!("{path_str}.rs");
        if all_paths.contains(&file_path) {
            return Some(file_path);
        }

        // Try as directory with mod.rs
        let mod_path = format!("{path_str}/mod.rs");
        if all_paths.contains(&mod_path) {
            return Some(mod_path);
        }
    }

    None
}

fn resolve_go_import(raw: &str, all_paths: &HashSet<String>) -> Option<String> {
    // Try matching last segments against project directories
    let segments: Vec<&str> = raw.split('/').collect();
    for start in 0..segments.len() {
        let candidate = segments[start..].join("/");
        // Check if any file is in this directory
        for path in all_paths {
            if path.starts_with(&candidate) && path.ends_with(".go") {
                return Some(path.clone());
            }
        }
    }
    None
}

fn resolve_python_import(raw: &str, all_paths: &HashSet<String>) -> Option<String> {
    let path_str = raw.replace('.', "/");

    let file = format!("{path_str}.py");
    if all_paths.contains(&file) {
        return Some(file);
    }

    let init = format!("{path_str}/__init__.py");
    if all_paths.contains(&init) {
        return Some(init);
    }

    None
}

fn resolve_js_import(raw: &str, source_path: &str, all_paths: &HashSet<String>) -> Option<String> {
    if !raw.starts_with('.') {
        return None; // external package
    }

    let source_dir = source_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let resolved = normalize_relative_path(source_dir, raw);

    let extensions = &[
        "",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        "/index.ts",
        "/index.tsx",
        "/index.js",
        "/index.jsx",
    ];
    for ext in extensions {
        let candidate = format!("{resolved}{ext}");
        if all_paths.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn resolve_java_import_path(raw: &str, all_paths: &HashSet<String>) -> Option<String> {
    // java.util.List → java/util/List.java
    let path_str = raw.replace('.', "/");
    let file = format!("{path_str}.java");
    if all_paths.contains(&file) {
        return Some(file);
    }
    // Try under src/main/java/
    let src_file = format!("src/main/java/{path_str}.java");
    if all_paths.contains(&src_file) {
        return Some(src_file);
    }
    None
}

fn resolve_c_import(raw: &str, source_path: &str, all_paths: &HashSet<String>) -> Option<String> {
    // Try relative to source file
    let source_dir = source_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let relative = if source_dir.is_empty() {
        raw.to_string()
    } else {
        format!("{source_dir}/{raw}")
    };

    if all_paths.contains(&relative) {
        return Some(relative);
    }

    // Try from root
    if all_paths.contains(raw) {
        return Some(raw.to_string());
    }

    None
}

fn resolve_relative_import(
    raw: &str,
    source_path: &str,
    extensions: &[&str],
    all_paths: &HashSet<String>,
) -> Option<String> {
    let source_dir = source_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let resolved = normalize_relative_path(source_dir, raw);

    if all_paths.contains(&resolved) {
        return Some(resolved);
    }

    for ext in extensions {
        let candidate = format!("{resolved}.{ext}");
        if all_paths.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn normalize_relative_path(base_dir: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches("./");

    if !rel.starts_with("../") {
        return if base_dir.is_empty() {
            rel.to_string()
        } else {
            format!("{base_dir}/{rel}")
        };
    }

    let mut parts: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };

    let mut remaining = rel;
    while let Some(rest) = remaining.strip_prefix("../") {
        parts.pop();
        remaining = rest;
    }

    if parts.is_empty() {
        remaining.to_string()
    } else {
        format!("{}/{remaining}", parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_utf8;

    #[test]
    fn truncate_utf8_keeps_char_boundary() {
        let text = format!("{}═suffix", "a".repeat(199));
        let truncated = truncate_utf8(&text, 200);

        assert_eq!(truncated, "a".repeat(199));
    }

    #[test]
    fn truncate_utf8_includes_char_on_boundary() {
        let text = format!("{}═suffix", "a".repeat(199));
        let truncated = truncate_utf8(&text, 202);

        assert_eq!(truncated, format!("{}═", "a".repeat(199)));
    }
}
