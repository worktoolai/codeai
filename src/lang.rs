use tree_sitter::Language;

// ── DocStyle ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocStyle {
    Docstring,
    LineComment,
    RustDoc,
    BlockComment,
    Generic,
}

// ── LangConfig ──

#[derive(Debug, Clone)]
pub struct LangConfig {
    pub language: &'static str,
    pub extensions: &'static [&'static str],
    pub function_nodes: &'static [&'static str],
    pub class_nodes: &'static [&'static str],
    pub doc_style: DocStyle,
}

// ── Language definitions ──

pub static GO: LangConfig = LangConfig {
    language: "go",
    extensions: &["go"],
    function_nodes: &["function_declaration", "method_declaration"],
    class_nodes: &["type_declaration"],
    doc_style: DocStyle::LineComment,
};

pub static RUST: LangConfig = LangConfig {
    language: "rust",
    extensions: &["rs"],
    function_nodes: &["function_item"],
    class_nodes: &["impl_item", "struct_item", "enum_item", "trait_item"],
    doc_style: DocStyle::RustDoc,
};

pub static PYTHON: LangConfig = LangConfig {
    language: "python",
    extensions: &["py"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_definition"],
    doc_style: DocStyle::Docstring,
};

pub static TYPESCRIPT: LangConfig = LangConfig {
    language: "typescript",
    extensions: &["ts"],
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static TSX: LangConfig = LangConfig {
    language: "tsx",
    extensions: &["tsx"],
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static JAVASCRIPT: LangConfig = LangConfig {
    language: "javascript",
    extensions: &["js", "mjs", "cjs"],
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
    class_nodes: &["class_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static JSX: LangConfig = LangConfig {
    language: "jsx",
    extensions: &["jsx"],
    function_nodes: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
    ],
    class_nodes: &["class_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static JAVA: LangConfig = LangConfig {
    language: "java",
    extensions: &["java"],
    function_nodes: &["method_declaration"],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static KOTLIN: LangConfig = LangConfig {
    language: "kotlin",
    extensions: &["kt", "kts"],
    function_nodes: &["function_declaration"],
    class_nodes: &["class_declaration", "object_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static C: LangConfig = LangConfig {
    language: "c",
    extensions: &["c", "h"],
    function_nodes: &["function_definition"],
    class_nodes: &["struct_specifier"],
    doc_style: DocStyle::BlockComment,
};

pub static CPP: LangConfig = LangConfig {
    language: "cpp",
    extensions: &["cpp", "cc", "cxx", "hpp", "hxx"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_specifier", "namespace_definition"],
    doc_style: DocStyle::BlockComment,
};

pub static CSHARP: LangConfig = LangConfig {
    language: "csharp",
    extensions: &["cs"],
    function_nodes: &["method_declaration"],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static SWIFT: LangConfig = LangConfig {
    language: "swift",
    extensions: &["swift"],
    function_nodes: &["function_declaration"],
    class_nodes: &["class_declaration", "protocol_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static SCALA: LangConfig = LangConfig {
    language: "scala",
    extensions: &["scala", "sc"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_definition", "object_definition", "trait_definition"],
    doc_style: DocStyle::BlockComment,
};

pub static RUBY: LangConfig = LangConfig {
    language: "ruby",
    extensions: &["rb"],
    function_nodes: &["method"],
    class_nodes: &["class", "module"],
    doc_style: DocStyle::Generic,
};

pub static PHP: LangConfig = LangConfig {
    language: "php",
    extensions: &["php"],
    function_nodes: &["function_definition", "method_declaration"],
    class_nodes: &["class_declaration"],
    doc_style: DocStyle::BlockComment,
};

pub static BASH: LangConfig = LangConfig {
    language: "bash",
    extensions: &["sh", "bash"],
    function_nodes: &["function_definition"],
    class_nodes: &[],
    doc_style: DocStyle::Generic,
};

pub static HCL: LangConfig = LangConfig {
    language: "hcl",
    extensions: &["tf", "hcl"],
    function_nodes: &[],
    class_nodes: &["block"],
    doc_style: DocStyle::Generic,
};

static ALL_CONFIGS: &[&LangConfig] = &[
    &GO,
    &RUST,
    &PYTHON,
    &TYPESCRIPT,
    &TSX,
    &JAVASCRIPT,
    &JSX,
    &JAVA,
    &KOTLIN,
    &C,
    &CPP,
    &CSHARP,
    &SWIFT,
    &SCALA,
    &RUBY,
    &PHP,
    &BASH,
    &HCL,
];

// ── Lookup functions ──

/// Get LangConfig by file extension (without dot).
pub fn config_for_extension(ext: &str) -> Option<&'static LangConfig> {
    let ext_lower = ext.to_lowercase();
    ALL_CONFIGS
        .iter()
        .find(|c| c.extensions.contains(&ext_lower.as_str()))
        .copied()
}

/// Get tree-sitter Language for a file extension.
pub fn ts_language_for_extension(ext: &str) -> Option<Language> {
    let ext_lower = ext.to_lowercase();
    match ext_lower.as_str() {
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "js" | "mjs" | "cjs" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "rb" => Some(tree_sitter_ruby::LANGUAGE.into()),
        "sh" | "bash" => Some(tree_sitter_bash::LANGUAGE.into()),
        // Languages without grammar crates in Cargo.toml: kt, kts, cs, swift, scala, sc, php, tf, hcl
        _ => None,
    }
}

/// All supported extensions.
pub fn all_extensions() -> Vec<&'static str> {
    ALL_CONFIGS
        .iter()
        .flat_map(|c| c.extensions.iter().copied())
        .collect()
}

/// Check if an extension is supported.
pub fn is_supported_extension(ext: &str) -> bool {
    config_for_extension(ext).is_some()
}
