use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

// ── Helpers ──

#[allow(deprecated)]
fn codeai() -> Command {
    Command::cargo_bin("codeai").unwrap()
}

/// Create a temp project with sample source files for testing.
fn setup_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    // Go file
    fs::create_dir_all(root.join("services/payment")).unwrap();
    fs::write(
        root.join("services/payment/validate.go"),
        r#"package payment

// ValidatePayment checks if a payment request is valid.
// It returns an error if validation fails.
func ValidatePayment(amount int, currency string) error {
    if amount <= 0 {
        return fmt.Errorf("payment validation failed: invalid amount %d", amount)
    }
    if currency == "" {
        return fmt.Errorf("payment validation failed: missing currency")
    }
    return nil
}

// ProcessPayment executes the payment after validation.
func ProcessPayment(id string, amount int) error {
    err := ValidatePayment(amount, "USD")
    if err != nil {
        return err
    }
    return nil
}
"#,
    )
    .unwrap();

    // Python file
    fs::create_dir_all(root.join("utils")).unwrap();
    fs::write(
        root.join("utils/helpers.py"),
        r#"def parse_config(path: str) -> dict:
    """Parse a configuration file and return a dictionary."""
    with open(path) as f:
        return json.load(f)

def validate_email(email: str) -> bool:
    """Check if an email address is valid."""
    return "@" in email and "." in email

class ConfigManager:
    """Manages application configuration."""
    def __init__(self, config_path: str):
        self.config = parse_config(config_path)

    def get(self, key: str, default=None):
        return self.config.get(key, default)
"#,
    )
    .unwrap();

    // Rust file
    fs::write(
        root.join("utils/math.rs"),
        r#"/// Compute the greatest common divisor.
pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Compute the least common multiple.
pub fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}

struct Calculator {
    history: Vec<f64>,
}

impl Calculator {
    fn new() -> Self {
        Self { history: Vec::new() }
    }

    fn add(&mut self, val: f64) {
        self.history.push(val);
    }
}
"#,
    )
    .unwrap();

    // JavaScript file
    fs::write(
        root.join("utils/format.js"),
        r#"/**
 * Format a currency amount with symbol.
 */
function formatCurrency(amount, currency) {
    return `${currency} ${amount.toFixed(2)}`;
}

const parseDate = (dateStr) => {
    return new Date(dateStr);
};

class Formatter {
    constructor(locale) {
        this.locale = locale;
    }

    format(value) {
        return value.toLocaleString(this.locale);
    }
}

module.exports = { formatCurrency, parseDate, Formatter };
"#,
    )
    .unwrap();

    dir
}

fn parse_response(output: &[u8]) -> serde_json::Value {
    serde_json::from_slice(output).expect("output should be valid JSON")
}

fn get_items(resp: &serde_json::Value) -> &Vec<serde_json::Value> {
    resp["i"].as_array().expect("response should have items")
}

fn git(args: &[&str], cwd: &std::path::Path) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn setup_git_repo(root: &std::path::Path) {
    git(&["init"], root);
    git(&["config", "user.email", "test@example.com"], root);
    git(&["config", "user.name", "Test User"], root);
    git(&["add", "."], root);
    git(&["commit", "-m", "initial"], root);
}

// ── Tests ──

#[test]
fn test_index_creates_blocks() {
    let dir = setup_project();

    let output = codeai()
        .arg("index")
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let resp = parse_response(&output.stdout);
    assert_eq!(resp["v"], 1);
    assert_eq!(resp["m"][0], "index");

    let info = &resp["i"][0];
    let indexed_files = info["indexed_files"].as_u64().unwrap();
    let indexed_blocks = info["indexed_blocks"].as_u64().unwrap();
    let total_blocks = info["total_blocks"].as_u64().unwrap();

    assert!(
        indexed_files >= 4,
        "should index at least 4 files, got {indexed_files}"
    );
    assert!(
        indexed_blocks >= 10,
        "should find at least 10 blocks, got {indexed_blocks}"
    );
    assert_eq!(indexed_blocks, total_blocks);
}

#[test]
fn test_index_incremental_skips_unchanged() {
    let dir = setup_project();
    setup_git_repo(dir.path());

    // First index
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Second index — nothing changed
    let output = codeai()
        .arg("index")
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let info = &resp["i"][0];
    assert_eq!(
        info["indexed_files"].as_u64().unwrap(),
        0,
        "no files should be re-indexed"
    );
    assert!(
        info["total_blocks"].as_u64().unwrap() > 0,
        "total blocks should persist"
    );
}

#[test]
fn test_index_incremental_detects_change() {
    let dir = setup_project();
    setup_git_repo(dir.path());

    // First index
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Modify a file
    fs::write(
        dir.path().join("utils/helpers.py"),
        r#"def parse_config(path: str) -> dict:
    """Parse a configuration file and return a dictionary."""
    with open(path) as f:
        return json.load(f)

def new_function():
    """A brand new function."""
    pass
"#,
    )
    .unwrap();

    // Re-index
    let output = codeai()
        .arg("index")
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let info = &resp["i"][0];
    assert_eq!(
        info["indexed_files"].as_u64().unwrap(),
        1,
        "only modified file should be re-indexed"
    );
}

#[test]
fn test_index_full_reindex() {
    let dir = setup_project();
    setup_git_repo(dir.path());

    // First index
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Full reindex
    let output = codeai()
        .args(["index", "--full"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let info = &resp["i"][0];
    assert!(
        info["indexed_files"].as_u64().unwrap() >= 4,
        "full reindex should re-index all files"
    );
}

#[test]
fn test_index_incremental_detects_deleted_file_via_git() {
    let dir = setup_project();
    setup_git_repo(dir.path());

    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Ensure symbol is searchable before deletion
    let before = codeai()
        .args(["search", "ValidatePayment"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let before_resp = parse_response(&before.stdout);
    assert!(
        !get_items(&before_resp).is_empty(),
        "symbol should exist before delete"
    );

    fs::remove_file(dir.path().join("services/payment/validate.go")).unwrap();

    let idx = codeai()
        .arg("index")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let idx_resp = parse_response(&idx.stdout);
    let idx_info = &idx_resp["i"][0];
    assert_eq!(
        idx_info["indexed_files"].as_u64().unwrap(),
        0,
        "delete-only change should not reindex files"
    );

    let after = codeai()
        .args(["search", "ValidatePayment"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let after_resp = parse_response(&after.stdout);
    let after_items = get_items(&after_resp);
    assert!(
        !after_items
            .iter()
            .any(|i| i[2].as_str().unwrap_or("").contains("validate.go")),
        "deleted file should be removed from search index"
    );
}

#[test]
fn test_index_incremental_detects_rename_via_git() {
    let dir = setup_project();
    setup_git_repo(dir.path());

    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    fs::rename(
        dir.path().join("utils/helpers.py"),
        dir.path().join("utils/helpers2.py"),
    )
    .unwrap();

    let idx = codeai()
        .arg("index")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let idx_resp = parse_response(&idx.stdout);
    let idx_info = &idx_resp["i"][0];
    assert_eq!(
        idx_info["indexed_files"].as_u64().unwrap(),
        1,
        "rename should index new path once"
    );

    let old_outline = codeai()
        .args(["outline", "utils/helpers.py"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let old_resp = parse_response(&old_outline.stdout);
    assert_eq!(old_resp["e"]["code"].as_str().unwrap(), "FILE_NOT_FOUND");

    let new_outline = codeai()
        .args(["outline", "utils/helpers2.py"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let new_resp = parse_response(&new_outline.stdout);
    assert!(
        !get_items(&new_resp).is_empty(),
        "renamed file should be indexed at new path"
    );
}

#[test]
fn test_search_finds_by_name() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["search", "ValidatePayment", "--limit", "5"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    assert!(!items.is_empty(), "search should find results");
    // First result should be ValidatePayment
    assert_eq!(items[0][1].as_str().unwrap(), "ValidatePayment");
    assert!(items[0][0].as_str().unwrap().contains("validate.go"));
}

#[test]
fn test_search_finds_by_string_literal() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["search", "payment validation failed"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    assert!(
        !items.is_empty(),
        "should find blocks containing the error string"
    );
    // Should find the validate.go file
    let paths: Vec<&str> = items.iter().filter_map(|i| i[2].as_str()).collect();
    assert!(paths.iter().any(|p| p.contains("validate.go")));
}

#[test]
fn test_search_with_path_filter() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["search", "parse", "--path", "utils"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    // All results should be in utils/
    for item in items {
        let path = item[2].as_str().unwrap();
        assert!(
            path.contains("utils"),
            "result path should contain 'utils': {path}"
        );
    }
}

#[test]
fn test_search_empty_index() {
    let dir = TempDir::new().unwrap();

    let output = codeai()
        .args(["search", "anything"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    assert_eq!(resp["e"]["code"].as_str().unwrap(), "INDEX_EMPTY");
}

#[test]
fn test_search_has_hints() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["search", "gcd"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);
    assert!(!items.is_empty());

    // Should have hints
    let hints = resp["h"].as_array();
    assert!(hints.is_some(), "search response should include hints");
    assert_eq!(hints.unwrap()[0][0].as_str().unwrap(), "open");
}

#[test]
fn test_outline_lists_blocks() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["outline", "services/payment/validate.go"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    // Should find ValidatePayment and ProcessPayment
    let names: Vec<&str> = items.iter().filter_map(|i| i[1].as_str()).collect();
    assert!(
        names.contains(&"ValidatePayment"),
        "should contain ValidatePayment: {names:?}"
    );
    assert!(
        names.contains(&"ProcessPayment"),
        "should contain ProcessPayment: {names:?}"
    );

    // outline tuple: [symbol_id, name, kind, path, range]
    for item in items {
        assert!(item[0].is_string(), "symbol_id should be string");
        assert!(item[1].is_string(), "name should be string");
        assert!(item[2].is_string(), "kind should be string");
        assert!(item[3].is_string(), "path should be string");
        assert!(item[4].is_string(), "range should be string");
    }
}

#[test]
fn test_outline_file_not_found() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["outline", "nonexistent/file.go"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    assert_eq!(resp["e"]["code"].as_str().unwrap(), "FILE_NOT_FOUND");
}

#[test]
fn test_outline_kind_filter() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["outline", "utils/helpers.py", "--kind", "class"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    for item in items {
        assert_eq!(
            item[2].as_str().unwrap(),
            "class",
            "kind filter should only return classes"
        );
    }
}

#[test]
fn test_open_single_symbol() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // First get the symbol_id from outline
    let outline_out = codeai()
        .args(["outline", "services/payment/validate.go"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let outline_resp = parse_response(&outline_out.stdout);
    let symbol_id = outline_resp["i"][0][0].as_str().unwrap();

    // Open it
    let output = codeai()
        .args(["open", "--symbol", symbol_id])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    assert_eq!(resp["m"][0], "open");

    let items = get_items(&resp);
    assert_eq!(items.len(), 1);

    // open tuple: [symbol_id, name, path, range, signature, doc, content]
    let item = &items[0];
    assert_eq!(item[0].as_str().unwrap(), symbol_id);
    assert!(item[2].as_str().unwrap().contains("validate.go"));
    // Content should contain actual source code
    let content = item[6].as_str().unwrap();
    assert!(
        content.contains("func"),
        "content should contain source code"
    );
}

#[test]
fn test_open_symbol_not_found() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["open", "--symbol", "nonexistent.go#function#foo"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    assert_eq!(resp["e"]["code"].as_str().unwrap(), "SYMBOL_NOT_FOUND");
    // Should have recovery hints
    assert!(resp["e"]["recovery"].is_array());
}

#[test]
fn test_open_batch() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Get symbol IDs from outline
    let outline_out = codeai()
        .args(["outline", "services/payment/validate.go"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let outline_resp = parse_response(&outline_out.stdout);
    let outline_items = get_items(&outline_resp);

    // Get first two function symbol IDs
    let ids: Vec<&str> = outline_items
        .iter()
        .filter(|i| i[2].as_str() == Some("function"))
        .take(2)
        .filter_map(|i| i[0].as_str())
        .collect();

    assert!(ids.len() >= 2, "should have at least 2 functions");

    let symbols_arg = ids.join(",");
    let output = codeai()
        .args(["open", "--symbols", &symbols_arg])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);
    assert_eq!(items.len(), 2, "batch open should return 2 items");
}

#[test]
fn test_open_range() {
    let dir = setup_project();

    let output = codeai()
        .args(["open", "--range", "services/payment/validate.go:4:0-12:0"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);
    assert_eq!(items.len(), 1);

    let content = items[0][6].as_str().unwrap();
    assert!(
        content.contains("ValidatePayment"),
        "range should capture function body"
    );
}

#[test]
fn test_open_range_file_not_found() {
    let dir = TempDir::new().unwrap();

    let output = codeai()
        .args(["open", "--range", "nonexistent.go:0:0-10:0"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let resp = parse_response(&output.stdout);
    assert_eq!(resp["e"]["code"].as_str().unwrap(), "FILE_NOT_FOUND");
}

#[test]
fn test_output_schema_version() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // All commands should include v=1
    for args in [vec!["search", "gcd"], vec!["outline", "utils/math.rs"]] {
        let output = codeai()
            .args(&args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        let resp = parse_response(&output.stdout);
        assert_eq!(resp["v"], 1, "schema version should be 1 for {args:?}");
    }
}

#[test]
fn test_meta_tuple_format() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["search", "gcd"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let meta = resp["m"].as_array().unwrap();

    // m = [cmd, max_bytes, byte_count, truncated(0/1), next_cursor|null]
    assert_eq!(meta.len(), 5, "meta tuple should have 5 elements");
    assert_eq!(meta[0].as_str().unwrap(), "search");
    assert!(meta[1].is_u64(), "max_bytes should be u64");
    assert!(meta[2].is_u64(), "byte_count should be u64");
    assert!(meta[3].as_u64().unwrap() <= 1, "truncated should be 0 or 1");
    assert!(
        meta[4].is_null(),
        "next_cursor should be null when not paginating"
    );
}

#[test]
fn test_multilang_index() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Verify each language file was indexed
    for (path, expected_name) in [
        ("services/payment/validate.go", "ValidatePayment"),
        ("utils/helpers.py", "parse_config"),
        ("utils/math.rs", "gcd"),
        ("utils/format.js", "formatCurrency"),
    ] {
        let output = codeai()
            .args(["outline", path])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let resp = parse_response(&output.stdout);
        let items = get_items(&resp);
        let names: Vec<&str> = items.iter().filter_map(|i| i[1].as_str()).collect();
        assert!(
            names.contains(&expected_name),
            "{path} should contain {expected_name}, got: {names:?}"
        );
    }
}

#[test]
fn test_js_arrow_function_name() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["outline", "utils/format.js"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);
    let names: Vec<&str> = items.iter().filter_map(|i| i[1].as_str()).collect();

    assert!(
        names.contains(&"parseDate"),
        "arrow function assigned to const should have name 'parseDate', got: {names:?}"
    );
}

#[test]
fn test_python_class_and_methods() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    let output = codeai()
        .args(["outline", "utils/helpers.py"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let items = get_items(&resp);

    let kinds_names: Vec<(&str, &str)> = items
        .iter()
        .filter_map(|i| Some((i[2].as_str()?, i[1].as_str()?)))
        .collect();

    assert!(
        kinds_names
            .iter()
            .any(|(k, n)| *k == "class" && *n == "ConfigManager"),
        "should find class ConfigManager: {kinds_names:?}"
    );
    assert!(
        kinds_names
            .iter()
            .any(|(k, n)| *k == "function" && *n == "parse_config"),
        "should find function parse_config: {kinds_names:?}"
    );
}

#[test]
fn test_search_then_open_roundtrip() {
    let dir = setup_project();
    codeai()
        .arg("index")
        .current_dir(dir.path())
        .assert()
        .success();

    // Search
    let search_out = codeai()
        .args(["search", "gcd"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let search_resp = parse_response(&search_out.stdout);
    let items = get_items(&search_resp);
    assert!(!items.is_empty());

    let symbol_id = items[0][0].as_str().unwrap();

    // Open using the symbol_id from search
    let open_out = codeai()
        .args(["open", "--symbol", symbol_id])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let open_resp = parse_response(&open_out.stdout);
    let open_items = get_items(&open_resp);
    assert_eq!(open_items.len(), 1);

    let content = open_items[0][6].as_str().unwrap();
    assert!(
        content.contains("gcd"),
        "opened content should contain 'gcd'"
    );
}

#[test]
fn test_gitignore_respected() {
    let dir = setup_project();
    let root = dir.path();

    // ignore crate requires .git/ to recognize .gitignore
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".gitignore"), "utils/\n").unwrap();

    let output = codeai().arg("index").current_dir(root).output().unwrap();

    let resp = parse_response(&output.stdout);
    let info = &resp["i"][0];

    // Only validate.go should be indexed (utils/ is ignored)
    assert_eq!(info["indexed_files"].as_u64().unwrap(), 1);
}

#[test]
fn test_no_gitignore_flag() {
    let dir = setup_project();
    let root = dir.path();

    // ignore crate requires .git/ to recognize .gitignore
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".gitignore"), "utils/\n").unwrap();

    let output = codeai()
        .args(["index", "--no-gitignore"])
        .current_dir(root)
        .output()
        .unwrap();

    let resp = parse_response(&output.stdout);
    let info = &resp["i"][0];

    // All files should be indexed despite .gitignore
    assert!(info["indexed_files"].as_u64().unwrap() >= 4);
}
