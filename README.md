# codeai

**Stop wasting tokens reading entire files. Read only the blocks you need.**

AI coding agents today burn through context windows at an alarming rate. They `cat` entire files to find one function, grep blindly across codebases, and re-read the same code because they lost track of where things are. Every wasted token is latency, cost, and a step closer to context overflow.

**codeai** fixes this with a single idea: **treat code blocks — not files — as the unit of exploration.**

```
# Don't know the function name? Search by what it does.
$ codeai search "payment validation failed"

# Read only that function, not the whole file.
$ codeai open --symbol "services/payment/validate.go#func#ValidatePayment"

# Need the full picture? See what it calls, in one shot.
$ codeai open --symbol "..." --with=callees
```

## The Problem

Watch any AI agent explore a codebase. You'll see the same anti-patterns:

| Pattern | Waste |
|---------|-------|
| `cat src/service.go` (800 lines) to find one function | ~90% of tokens are noise |
| Name is fuzzy → grep fails → agent retries with variations | 3-5 round trips wasted |
| Found the function, now needs callees → reads 4 more files | Context explodes |
| File changed since last turn → stale line numbers → error → retry | Silent failures |
| Error response is unstructured → agent can't auto-recover | Stuck in retry loops |

The root cause: **every existing tool treats files as the atomic unit**. But agents think in *symbols* — functions, classes, methods. The mismatch forces them to over-read.

## How codeai Solves It

| Principle | Implementation |
|-----------|---------------|
| **Search before read** | BM25 + fuzzy search over block-level index. Find functions by name, doc, error strings, or path — even with partial recall. |
| **Block-level addressing** | Every function/class gets a stable `symbol_id`. Open just that block, not the file. |
| **Batch reads** | `open --symbols id1,id2,id3` — read 3 functions in one call. One round trip, not three. |
| **Bounded output** | Every response respects `--max-bytes`. No surprise 50KB dumps. Truncation is explicit, cursor-based. |
| **Structured errors** | Errors include machine-readable codes and recovery hints. Agent knows exactly what to do next. |
| **Stable IDs** | `symbol_id` survives code edits (as long as the function isn't renamed/deleted). No more stale line numbers. |
| **Incremental sync** | Only re-indexes changed files. Sub-second updates after edits. |

## Quick Start

```bash
# Build
cargo build --release

# Index your project (run from project root)
codeai index

# Explore
codeai search "authenticate user"
codeai outline src/auth/handler.go
codeai open --symbol "src/auth/handler.go#function#AuthenticateUser"
```

## Commands

### `codeai index`

Parse and index the codebase. Incremental by default.

```bash
codeai index                    # Incremental (only changed files)
codeai index --full             # Full rebuild
codeai index --lang rust        # Only index Rust files
codeai index --no-gitignore     # Include gitignored files
```

### `codeai search <query>`

Find blocks by name, content, doc comments, or string literals.

```bash
codeai search "validate payment"
codeai search "TODO" --path "src/services/"
codeai search "connection refused" --limit 5
```

Output (Thin JSON):
```json
{
  "v": 1,
  "m": ["search", 12000, 843, 0, null],
  "i": [
    ["src/payment/validate.go#func#ValidatePayment", "ValidatePayment", "src/payment/validate.go", "34:0-128:1", 12.4, ["doc","str"], "func ValidatePayment..."]
  ],
  "h": [["open", {"symbol_id": "src/payment/validate.go#func#ValidatePayment"}]]
}
```

### `codeai outline <path>`

List all blocks in a file — the code equivalent of a table of contents.

```bash
codeai outline src/auth/handler.go
codeai outline src/models.py --kind class
```

### `codeai open`

Read specific blocks. The core operation.

```bash
# Single block
codeai open --symbol "src/auth/handler.go#function#Login"

# Batch: compare multiple blocks in one call
codeai open --symbols "id1,id2,id3" --max-bytes 32000

# Raw range (no index needed)
codeai open --range "src/main.rs:10:0-50:0"
```

## Output Format

All output is **Thin JSON** — tuple-based minimal JSON designed for agents:

- **`v`**: Schema version (currently `1`)
- **`m`**: Meta tuple `[cmd, max_bytes, byte_count, truncated, next_cursor]`
- **`i`**: Items (tuples, format varies by command)
- **`h`**: Hints (suggested next actions)
- **`e`**: Error (when applicable, with `code`, `message`, `recovery`)

Errors are structured so agents can auto-recover:

```json
{
  "v": 1,
  "e": {
    "code": "SYMBOL_NOT_FOUND",
    "message": "symbol_id 'old_id' not found in current index",
    "recovery": [["search", {"query": "ValidatePayment"}]]
  }
}
```

## Symbol IDs

Blocks are addressed by human-readable, stable IDs:

```
<path>#<kind>#<name>
src/auth/handler.go#function#Login
utils/helpers.py#class#ConfigManager
```

IDs survive code edits — they don't include line numbers. If a function moves but keeps its name, the same ID still works. If an ID goes stale, `open` automatically attempts fallback resolution and returns recovery hints on failure.

## Supported Languages

11 languages with Tree-sitter grammars embedded (zero-config):

Go, Rust, Python, TypeScript/TSX, JavaScript/JSX, Java, C, C++, Ruby, Bash

7 more with config ready (grammar crates pending):

Kotlin, C#, Swift, Scala, PHP, HCL/Terraform

## Agent Integration

codeai is designed as an MCP tool or CLI tool for AI agents. The typical loop:

```
1. search  →  find candidate blocks (even with fuzzy recall)
2. open    →  read just those blocks
3. open --with=callees  →  expand context if needed
4. repeat with refined search if needed
```

Every response is bounded, structured, and includes hints for the next action. No surprises, no context blowups.

## Development

```bash
# Build
cargo build

# Run tests (unit + 25 CLI integration tests)
cargo test

# Run only CLI integration tests
cargo test --test cli

# Index codeai's own source
cargo run -- index
cargo run -- search "extract blocks"
```

## Design

See [codeai-design.md](codeai-design.md) for the full design document.

## License

MIT
