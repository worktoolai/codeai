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

// ── Import extraction config ──

#[derive(Debug, Clone, Copy)]
pub struct ImportNodeConfig {
    pub node_type: &'static str,
    pub extractor: ImportExtractor,
}

#[derive(Debug, Clone, Copy)]
pub enum ImportExtractor {
    Field(&'static str),
    StringField(&'static str),
    GoImport,
    JavaImport,
    CInclude,
    RubyRequire,
    BashSource,
}

// ── LangConfig ──

#[derive(Debug, Clone)]
pub struct LangConfig {
    pub language: &'static str,
    pub extensions: &'static [&'static str],
    pub function_nodes: &'static [&'static str],
    pub class_nodes: &'static [&'static str],
    pub doc_style: DocStyle,
    pub import_nodes: &'static [ImportNodeConfig],
}

// ── Language definitions ──

pub static GO: LangConfig = LangConfig {
    language: "go",
    extensions: &["go"],
    function_nodes: &["function_declaration", "method_declaration"],
    class_nodes: &["type_declaration"],
    doc_style: DocStyle::LineComment,
    import_nodes: &[ImportNodeConfig {
        node_type: "import_declaration",
        extractor: ImportExtractor::GoImport,
    }],
};

pub static RUST: LangConfig = LangConfig {
    language: "rust",
    extensions: &["rs"],
    function_nodes: &["function_item"],
    class_nodes: &["impl_item", "struct_item", "enum_item", "trait_item"],
    doc_style: DocStyle::RustDoc,
    import_nodes: &[ImportNodeConfig {
        node_type: "use_declaration",
        extractor: ImportExtractor::Field("argument"),
    }],
};

pub static PYTHON: LangConfig = LangConfig {
    language: "python",
    extensions: &["py"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_definition"],
    doc_style: DocStyle::Docstring,
    import_nodes: &[
        ImportNodeConfig {
            node_type: "import_statement",
            extractor: ImportExtractor::Field("name"),
        },
        ImportNodeConfig {
            node_type: "import_from_statement",
            extractor: ImportExtractor::Field("module_name"),
        },
    ],
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
    import_nodes: &[ImportNodeConfig {
        node_type: "import_statement",
        extractor: ImportExtractor::StringField("source"),
    }],
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
    import_nodes: &[ImportNodeConfig {
        node_type: "import_statement",
        extractor: ImportExtractor::StringField("source"),
    }],
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
    import_nodes: &[ImportNodeConfig {
        node_type: "import_statement",
        extractor: ImportExtractor::StringField("source"),
    }],
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
    import_nodes: &[ImportNodeConfig {
        node_type: "import_statement",
        extractor: ImportExtractor::StringField("source"),
    }],
};

pub static JAVA: LangConfig = LangConfig {
    language: "java",
    extensions: &["java"],
    function_nodes: &["method_declaration"],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[ImportNodeConfig {
        node_type: "import_declaration",
        extractor: ImportExtractor::JavaImport,
    }],
};

pub static KOTLIN: LangConfig = LangConfig {
    language: "kotlin",
    extensions: &["kt", "kts"],
    function_nodes: &["function_declaration"],
    class_nodes: &["class_declaration", "object_declaration"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[],
};

pub static C: LangConfig = LangConfig {
    language: "c",
    extensions: &["c", "h"],
    function_nodes: &["function_definition"],
    class_nodes: &["struct_specifier"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[ImportNodeConfig {
        node_type: "preproc_include",
        extractor: ImportExtractor::CInclude,
    }],
};

pub static CPP: LangConfig = LangConfig {
    language: "cpp",
    extensions: &["cpp", "cc", "cxx", "hpp", "hxx"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_specifier", "namespace_definition"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[ImportNodeConfig {
        node_type: "preproc_include",
        extractor: ImportExtractor::CInclude,
    }],
};

pub static CSHARP: LangConfig = LangConfig {
    language: "csharp",
    extensions: &["cs"],
    function_nodes: &["method_declaration"],
    class_nodes: &["class_declaration", "interface_declaration"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[],
};

pub static SWIFT: LangConfig = LangConfig {
    language: "swift",
    extensions: &["swift"],
    function_nodes: &["function_declaration"],
    class_nodes: &["class_declaration", "protocol_declaration"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[],
};

pub static SCALA: LangConfig = LangConfig {
    language: "scala",
    extensions: &["scala", "sc"],
    function_nodes: &["function_definition"],
    class_nodes: &["class_definition", "object_definition", "trait_definition"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[],
};

pub static RUBY: LangConfig = LangConfig {
    language: "ruby",
    extensions: &["rb"],
    function_nodes: &["method"],
    class_nodes: &["class", "module"],
    doc_style: DocStyle::Generic,
    import_nodes: &[ImportNodeConfig {
        node_type: "call",
        extractor: ImportExtractor::RubyRequire,
    }],
};

pub static PHP: LangConfig = LangConfig {
    language: "php",
    extensions: &["php"],
    function_nodes: &["function_definition", "method_declaration"],
    class_nodes: &["class_declaration"],
    doc_style: DocStyle::BlockComment,
    import_nodes: &[],
};

pub static BASH: LangConfig = LangConfig {
    language: "bash",
    extensions: &["sh", "bash"],
    function_nodes: &["function_definition"],
    class_nodes: &[],
    doc_style: DocStyle::Generic,
    import_nodes: &[ImportNodeConfig {
        node_type: "command",
        extractor: ImportExtractor::BashSource,
    }],
};

pub static HCL: LangConfig = LangConfig {
    language: "hcl",
    extensions: &["tf", "hcl"],
    function_nodes: &[],
    class_nodes: &["block"],
    doc_style: DocStyle::Generic,
    import_nodes: &[],
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
