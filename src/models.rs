use serde::{Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

// ── Error codes ──

pub const ERR_SYMBOL_NOT_FOUND: &str = "SYMBOL_NOT_FOUND";
pub const ERR_SYMBOL_AMBIGUOUS: &str = "SYMBOL_AMBIGUOUS";
pub const ERR_FILE_NOT_FOUND: &str = "FILE_NOT_FOUND";
pub const ERR_RANGE_OUT_OF_BOUNDS: &str = "RANGE_OUT_OF_BOUNDS";
pub const ERR_PARSE_ERROR: &str = "PARSE_ERROR";
pub const ERR_INDEX_EMPTY: &str = "INDEX_EMPTY";
pub const ERR_INDEX_BUSY: &str = "INDEX_BUSY";
pub const ERR_BUDGET_EXCEEDED: &str = "BUDGET_EXCEEDED";
pub const ERR_CURSOR_STALE: &str = "CURSOR_STALE";
pub const ERR_UNSUPPORTED_LANGUAGE: &str = "UNSUPPORTED_LANGUAGE";
pub const ERR_ENCODING_ERROR: &str = "ENCODING_ERROR";
pub const ERR_PERMISSION_DENIED: &str = "PERMISSION_DENIED";

// ── Range ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}-{}:{}",
            self.start_line, self.start_col, self.end_line, self.end_col
        )
    }
}

impl FromStr for Range {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (start, end) = s.split_once('-').ok_or("missing '-'")?;
        let (sl, sc) = start.split_once(':').ok_or("missing ':' in start")?;
        let (el, ec) = end.split_once(':').ok_or("missing ':' in end")?;
        Ok(Range {
            start_line: sl.parse().map_err(|e| format!("{e}"))?,
            start_col: sc.parse().map_err(|e| format!("{e}"))?,
            end_line: el.parse().map_err(|e| format!("{e}"))?,
            end_col: ec.parse().map_err(|e| format!("{e}"))?,
        })
    }
}

impl Serialize for Range {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ── BlockKind ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    Impl,
    Module,
    Namespace,
    Block,
    Object,
    Protocol,
}

impl fmt::Display for BlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::Impl => "impl",
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Block => "block",
            Self::Object => "object",
            Self::Protocol => "protocol",
        };
        f.write_str(s)
    }
}

impl FromStr for BlockKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" | "func" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            "class" => Ok(Self::Class),
            "struct" => Ok(Self::Struct),
            "interface" => Ok(Self::Interface),
            "trait" => Ok(Self::Trait),
            "enum" => Ok(Self::Enum),
            "impl" => Ok(Self::Impl),
            "module" => Ok(Self::Module),
            "namespace" => Ok(Self::Namespace),
            "block" => Ok(Self::Block),
            "object" => Ok(Self::Object),
            "protocol" => Ok(Self::Protocol),
            _ => Err(format!("unknown block kind: {s}")),
        }
    }
}

// ── SymbolId helpers ──

/// Build a symbol_id string: `path#kind#name` or `path#kind#name#N`
pub fn build_symbol_id(path: &str, kind: &BlockKind, name: &str, occurrence: Option<u32>) -> String {
    match occurrence {
        Some(idx) => format!("{path}#{kind}#{name}#{idx}"),
        None => format!("{path}#{kind}#{name}"),
    }
}

/// Parse a symbol_id back into (path, kind_str, name, occurrence).
pub fn parse_symbol_id(id: &str) -> Option<(String, String, String, Option<u32>)> {
    let parts: Vec<&str> = id.splitn(4, '#').collect();
    match parts.len() {
        3 => Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string(), None)),
        4 => {
            let occ = parts[3].parse::<u32>().ok();
            // If the 4th part isn't a number, treat it as part of name (shouldn't happen)
            if occ.is_some() {
                Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string(), occ))
            } else {
                Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string(), None))
            }
        }
        _ => None,
    }
}

// ── Block ──

#[derive(Debug, Clone)]
pub struct Block {
    pub symbol_id: String,
    pub language: String,
    pub kind: BlockKind,
    pub name: String,
    pub path: String,
    pub range: Range,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub preview: String,
    pub strings: Vec<String>,
}

// ── Output types (Thin JSON) ──

#[derive(Debug, Clone, Serialize)]
pub struct ThinResponse {
    pub v: u32,
    pub m: Meta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<ErrorBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<Vec<String>>,
}

impl ThinResponse {
    pub fn success(cmd: &str, max_bytes: u64, items: Vec<serde_json::Value>) -> Self {
        let byte_count = serde_json::to_string(&items).map(|s| s.len() as u64).unwrap_or(0);
        Self {
            v: 1,
            m: Meta {
                cmd: cmd.to_string(),
                max_bytes,
                byte_count,
                truncated: false,
                next_cursor: None,
            },
            i: Some(items),
            h: None,
            e: None,
            remaining: None,
        }
    }

    pub fn error(cmd: &str, max_bytes: u64, code: &str, message: String, recovery: Option<Vec<serde_json::Value>>) -> Self {
        Self {
            v: 1,
            m: Meta {
                cmd: cmd.to_string(),
                max_bytes,
                byte_count: 0,
                truncated: false,
                next_cursor: None,
            },
            i: None,
            h: None,
            e: Some(ErrorBody {
                code: code.to_string(),
                message,
                recovery,
            }),
            remaining: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Meta {
    pub cmd: String,
    pub max_bytes: u64,
    pub byte_count: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

impl Serialize for Meta {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(5))?;
        seq.serialize_element(&self.cmd)?;
        seq.serialize_element(&self.max_bytes)?;
        seq.serialize_element(&self.byte_count)?;
        seq.serialize_element(&(if self.truncated { 1u8 } else { 0u8 }))?;
        seq.serialize_element(&self.next_cursor)?;
        seq.end()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_range_display_parse() {
        let r = Range { start_line: 10, start_col: 0, end_line: 50, end_col: 1 };
        assert_eq!(r.to_string(), "10:0-50:1");
        assert_eq!(Range::from_str("10:0-50:1").unwrap(), r);
    }

    #[test]
    fn test_block_kind_roundtrip() {
        for kind in &[
            BlockKind::Function, BlockKind::Method, BlockKind::Class,
            BlockKind::Struct, BlockKind::Interface, BlockKind::Trait,
        ] {
            let s = kind.to_string();
            assert_eq!(&BlockKind::from_str(&s).unwrap(), kind);
        }
    }

    #[test]
    fn test_symbol_id_build_parse() {
        let id = build_symbol_id("src/main.rs", &BlockKind::Function, "main", None);
        assert_eq!(id, "src/main.rs#function#main");
        let parsed = parse_symbol_id(&id).unwrap();
        assert_eq!(parsed, ("src/main.rs".into(), "function".into(), "main".into(), None));

        let id2 = build_symbol_id("lib.rs", &BlockKind::Method, "new", Some(1));
        assert_eq!(id2, "lib.rs#method#new#1");
        let parsed2 = parse_symbol_id(&id2).unwrap();
        assert_eq!(parsed2, ("lib.rs".into(), "method".into(), "new".into(), Some(1)));
    }

    #[test]
    fn test_meta_serializes_as_tuple() {
        let m = Meta {
            cmd: "search".into(),
            max_bytes: 12000,
            byte_count: 8000,
            truncated: false,
            next_cursor: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#"["search",12000,8000,0,null]"#);
    }

    #[test]
    fn test_thin_response_error() {
        let resp = ThinResponse::error("open", 16000, ERR_SYMBOL_NOT_FOUND, "not found".into(), None);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["v"], 1);
        assert_eq!(json["e"]["code"], "SYMBOL_NOT_FOUND");
        assert!(json.get("i").is_none());
    }
}
