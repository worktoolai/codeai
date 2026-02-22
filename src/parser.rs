use anyhow::{Context, Result};
use tree_sitter::{Language, Node, Parser};

const PREVIEW_LINES: usize = 20;
const MAX_STRINGS: usize = 20;
const MAX_STRING_LEN: usize = 200;

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
            let truncated = if text.len() > MAX_STRING_LEN {
                &text[..MAX_STRING_LEN]
            } else {
                text
            };
            strings.push(truncated.to_string());
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_string_nodes(child, source, strings);
    }
}
