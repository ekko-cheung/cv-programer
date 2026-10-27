use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{path::Path, sync::Mutex};

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Serialize)]
pub struct Item {
    pub id: i64,
    pub kind: String,
    pub category: String,
    pub preview: String,
    pub blob: Option<String>,
    pub created: i64,
    pub pinned: bool,
    pub sync: bool,
}

pub struct NewItem {
    pub kind: &'static str,
    pub category: &'static str,
    pub content: String,
    pub blob: Option<String>,
    pub hash: String,
}

const PREVIEW_CHARS: usize = 300;
const SELECT: &str = "SELECT id, kind, category, preview, blob, created, pinned, sync FROM items";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-512;
             CREATE TABLE IF NOT EXISTS items(
               id INTEGER PRIMARY KEY,
               uuid TEXT NOT NULL,
               kind TEXT NOT NULL,
               category TEXT NOT NULL,
               preview TEXT NOT NULL,
               content TEXT NOT NULL,
               blob TEXT,
               hash TEXT NOT NULL UNIQUE,
               created INTEGER NOT NULL,
               pinned INTEGER NOT NULL DEFAULT 0,
               sync INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_items_created ON items(created DESC);
             CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Inserts a new item, or bumps an existing one with the same hash to the top.
    pub fn upsert(&self, it: NewItem) -> rusqlite::Result<()> {
        let c = self.conn.lock().unwrap();
        let now = now_ms();
        let bumped = c.execute("UPDATE items SET created=?1 WHERE hash=?2", params![now, it.hash])?;
        if bumped == 0 {
            let preview: String = it.content.chars().take(PREVIEW_CHARS).collect();
            c.execute(
                "INSERT INTO items(uuid,kind,category,preview,content,blob,hash,created) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![uuid::Uuid::new_v4().to_string(), it.kind, it.category, preview, it.content, it.blob, it.hash, now],
            )?;
        }
        Ok(())
    }

    pub fn list(&self, category: Option<&str>, query: &str, limit: i64) -> rusqlite::Result<Vec<Item>> {
        let c = self.conn.lock().unwrap();
        let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let sql = format!(
            "{SELECT} WHERE (?1 IS NULL OR category=?1) AND content LIKE ?2 ESCAPE '\\' ORDER BY pinned DESC, created DESC LIMIT ?3"
        );
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params![category, like, limit], |r| {
            Ok(Item {
                id: r.get(0)?,
                kind: r.get(1)?,
                category: r.get(2)?,
                preview: r.get(3)?,
                blob: r.get(4)?,
                created: r.get(5)?,
                pinned: r.get::<_, i64>(6)? != 0,
                sync: r.get::<_, i64>(7)? != 0,
            })
        })?;
        rows.collect()
    }

    /// Returns (kind, full content, blob path) for an item.
    pub fn content(&self, id: i64) -> rusqlite::Result<Option<(String, String, Option<String>)>> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT kind, content, blob FROM items WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
    }

    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute("UPDATE items SET created=?1 WHERE id=?2", params![now_ms(), id])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> rusqlite::Result<Option<String>> {
        let c = self.conn.lock().unwrap();
        let blob: Option<String> = c.query_row("SELECT blob FROM items WHERE id=?1", [id], |r| r.get(0)).optional()?.flatten();
        c.execute("DELETE FROM items WHERE id=?1", [id])?;
        Ok(blob)
    }

    pub fn set_flag(&self, id: i64, column: &str, value: bool) -> rusqlite::Result<()> {
        let col = match column {
            "pinned" => "pinned",
            "sync" => "sync",
            _ => return Ok(()),
        };
        self.conn.lock().unwrap().execute(&format!("UPDATE items SET {col}=?1 WHERE id=?2"), params![value as i64, id])?;
        Ok(())
    }

    pub fn counts(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT category, COUNT(*) FROM items GROUP BY category")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Keeps pinned items plus the newest `keep` others; returns orphaned blob paths.
    pub fn prune(&self, keep: i64) -> rusqlite::Result<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let sql = "FROM items WHERE pinned=0 AND id NOT IN (SELECT id FROM items WHERE pinned=0 ORDER BY created DESC LIMIT ?1)";
        let blobs: Vec<String> = {
            let mut st = c.prepare(&format!("SELECT blob {sql} AND blob IS NOT NULL"))?;
            let v = st.query_map([keep], |r| r.get(0))?.collect::<Result<_, _>>()?;
            v
        };
        c.execute(&format!("DELETE {sql}"), [keep])?;
        Ok(blobs)
    }

    pub fn get_kv(&self, key: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_kv(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2", params![key, value])?;
        Ok(())
    }

    /// Deletes history (optionally keeping pinned items); returns orphaned blob paths.
    pub fn clear(&self, keep_pinned: bool) -> rusqlite::Result<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let cond = if keep_pinned { "WHERE pinned=0" } else { "" };
        let blobs: Vec<String> = {
            let mut st = c.prepare(&format!("SELECT blob FROM items {cond} {} blob IS NOT NULL", if keep_pinned { "AND" } else { "WHERE" }))?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            v
        };
        c.execute(&format!("DELETE FROM items {cond}"), [])?;
        Ok(blobs)
    }
}
