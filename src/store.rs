use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

// ── Data types ──

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub path: String,
    pub mtime: u64,
    pub size: u64,
    pub content_hash: String,
    pub language: Option<String>,
    pub parse_error: bool,
}

#[derive(Debug, Clone)]
pub struct BlockRow {
    pub symbol_id: String,
    pub path: String,
    pub language: String,
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub preview: String,
}

#[derive(Debug, Clone)]
pub struct ImportRow {
    pub path: String,
    pub raw_import: String,
    pub resolved_path: Option<String>,
    pub kind: String,
}

// ── Store ──

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Self { conn };
        store.create_tables()?;
        Ok(store)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                language TEXT,
                parse_error INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS blocks (
                symbol_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                language TEXT NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                start_col INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                end_col INTEGER NOT NULL,
                signature TEXT,
                doc TEXT,
                preview TEXT NOT NULL,
                FOREIGN KEY (path) REFERENCES files(path) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_blocks_path ON blocks(path);
            CREATE INDEX IF NOT EXISTS idx_blocks_name ON blocks(name);
            CREATE INDEX IF NOT EXISTS idx_blocks_kind ON blocks(kind);

            CREATE TABLE IF NOT EXISTS imports (
                path TEXT NOT NULL,
                raw_import TEXT NOT NULL,
                resolved_path TEXT,
                kind TEXT NOT NULL DEFAULT 'module',
                UNIQUE(path, raw_import)
            );
            CREATE INDEX IF NOT EXISTS idx_imports_path ON imports(path);
            CREATE INDEX IF NOT EXISTS idx_imports_resolved ON imports(resolved_path);",
        )?;
        Ok(())
    }

    // ── Generation ──

    pub fn generation(&self) -> Result<u64> {
        let val: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'generation'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(val.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn next_generation(&self) -> Result<u64> {
        let gen = self.generation()? + 1;
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('generation', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            params![gen.to_string()],
        )?;
        Ok(gen)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("get_meta")
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn last_indexed_head(&self) -> Result<Option<String>> {
        self.get_meta("git_head")
    }

    pub fn set_last_indexed_head(&self, head: &str) -> Result<()> {
        self.set_meta("git_head", head)
    }

    // ── Files ──

    pub fn get_file(&self, path: &str) -> Result<Option<FileMeta>> {
        self.conn
            .query_row(
                "SELECT path, mtime, size, content_hash, language, parse_error FROM files WHERE path = ?1",
                params![path],
                |row| {
                    Ok(FileMeta {
                        path: row.get(0)?,
                        mtime: row.get(1)?,
                        size: row.get(2)?,
                        content_hash: row.get(3)?,
                        language: row.get(4)?,
                        parse_error: row.get::<_, i32>(5)? != 0,
                    })
                },
            )
            .optional()
            .context("get_file")
    }

    pub fn upsert_file(&self, file: &FileMeta) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, mtime, size, content_hash, language, parse_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
                mtime = ?2, size = ?3, content_hash = ?4, language = ?5, parse_error = ?6",
            params![
                file.path,
                file.mtime,
                file.size,
                file.content_hash,
                file.language,
                file.parse_error as i32,
            ],
        )?;
        Ok(())
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM blocks WHERE path = ?1", params![path])?;
        self.conn
            .execute("DELETE FROM imports WHERE path = ?1", params![path])?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(row?);
        }
        Ok(paths)
    }

    // ── Blocks ──

    pub fn replace_blocks(&self, path: &str, blocks: &[BlockRow]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM blocks WHERE path = ?1", params![path])?;
        let mut stmt = tx.prepare(
            "INSERT INTO blocks (symbol_id, path, language, kind, name,
             start_line, start_col, end_line, end_col, signature, doc, preview)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for b in blocks {
            stmt.execute(params![
                b.symbol_id,
                b.path,
                b.language,
                b.kind,
                b.name,
                b.start_line,
                b.start_col,
                b.end_line,
                b.end_col,
                b.signature,
                b.doc,
                b.preview,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    pub fn get_block(&self, symbol_id: &str) -> Result<Option<BlockRow>> {
        self.conn
            .query_row(
                "SELECT symbol_id, path, language, kind, name,
                 start_line, start_col, end_line, end_col, signature, doc, preview
                 FROM blocks WHERE symbol_id = ?1",
                params![symbol_id],
                |row| {
                    Ok(BlockRow {
                        symbol_id: row.get(0)?,
                        path: row.get(1)?,
                        language: row.get(2)?,
                        kind: row.get(3)?,
                        name: row.get(4)?,
                        start_line: row.get(5)?,
                        start_col: row.get(6)?,
                        end_line: row.get(7)?,
                        end_col: row.get(8)?,
                        signature: row.get(9)?,
                        doc: row.get(10)?,
                        preview: row.get(11)?,
                    })
                },
            )
            .optional()
            .context("get_block")
    }

    pub fn find_blocks(&self, path: &str, kind: &str, name: &str) -> Result<Vec<BlockRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT symbol_id, path, language, kind, name,
             start_line, start_col, end_line, end_col, signature, doc, preview
             FROM blocks WHERE path = ?1 AND kind = ?2 AND name = ?3
             ORDER BY start_line",
        )?;
        let rows = stmt.query_map(params![path, kind, name], |row| {
            Ok(BlockRow {
                symbol_id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                kind: row.get(3)?,
                name: row.get(4)?,
                start_line: row.get(5)?,
                start_col: row.get(6)?,
                end_line: row.get(7)?,
                end_col: row.get(8)?,
                signature: row.get(9)?,
                doc: row.get(10)?,
                preview: row.get(11)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn blocks_for_file(&self, path: &str) -> Result<Vec<BlockRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT symbol_id, path, language, kind, name,
             start_line, start_col, end_line, end_col, signature, doc, preview
             FROM blocks WHERE path = ?1
             ORDER BY start_line",
        )?;
        let rows = stmt.query_map(params![path], |row| {
            Ok(BlockRow {
                symbol_id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                kind: row.get(3)?,
                name: row.get(4)?,
                start_line: row.get(5)?,
                start_col: row.get(6)?,
                end_line: row.get(7)?,
                end_col: row.get(8)?,
                signature: row.get(9)?,
                doc: row.get(10)?,
                preview: row.get(11)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn block_count(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM blocks", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM blocks;
             DELETE FROM imports;
             DELETE FROM files;
             DELETE FROM meta;",
        )?;
        Ok(())
    }

    // ── Imports ──

    pub fn replace_imports(&self, path: &str, rows: &[ImportRow]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM imports WHERE path = ?1", params![path])?;
        let mut stmt = tx.prepare(
            "INSERT INTO imports (path, raw_import, resolved_path, kind)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for r in rows {
            stmt.execute(params![r.path, r.raw_import, r.resolved_path, r.kind])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    pub fn imports_for_file(&self, path: &str) -> Result<Vec<ImportRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, raw_import, resolved_path, kind
             FROM imports WHERE path = ?1",
        )?;
        let rows = stmt.query_map(params![path], |row| {
            Ok(ImportRow {
                path: row.get(0)?,
                raw_import: row.get(1)?,
                resolved_path: row.get(2)?,
                kind: row.get(3)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn all_imports(&self) -> Result<Vec<ImportRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, raw_import, resolved_path, kind FROM imports")?;
        let rows = stmt.query_map([], |row| {
            Ok(ImportRow {
                path: row.get(0)?,
                raw_import: row.get(1)?,
                resolved_path: row.get(2)?,
                kind: row.get(3)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn import_count(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}
