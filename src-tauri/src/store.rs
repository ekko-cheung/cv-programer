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

/// An item as exchanged with the remote bucket.
pub struct Upload {
    pub id: i64,
    pub uuid: String,
    pub kind: String,
    pub category: String,
    pub content: String,
    pub blob: Option<String>,
    pub hash: String,
    pub created: i64,
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
             CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS pending_deletes(uuid TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS seen_remote(uuid TEXT PRIMARY KEY, etag TEXT NOT NULL);",
        )?;
        // synced: 0 = not uploaded, 1 = uploaded (or pulled), 2 = skipped (too large / unreadable).
        let _ = conn.execute("ALTER TABLE items ADD COLUMN synced INTEGER NOT NULL DEFAULT 0", []);
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
        let row: Option<(Option<String>, i64, String)> = c
            .query_row("SELECT blob, synced, uuid FROM items WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((blob, synced, uuid)) = row else { return Ok(None) };
        if synced == 1 {
            // Tell the bucket (and other devices) about the deletion on the next sync.
            c.execute("INSERT OR IGNORE INTO pending_deletes(uuid) VALUES(?1)", [uuid])?;
        }
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

    // ---- sync support ----

    pub fn pending_uploads(&self, categories: &[String]) -> rusqlite::Result<Vec<Upload>> {
        let c = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, uuid, kind, category, content, blob, hash, created FROM items WHERE synced=0 AND kind!='file' AND (sync=1",
        );
        if !categories.is_empty() {
            sql.push_str(&format!(" OR category IN ({})", vec!["?"; categories.len()].join(",")));
        }
        sql.push(')');
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(rusqlite::params_from_iter(categories.iter()), |r| {
            Ok(Upload {
                id: r.get(0)?,
                uuid: r.get(1)?,
                kind: r.get(2)?,
                category: r.get(3)?,
                content: r.get(4)?,
                blob: r.get(5)?,
                hash: r.get(6)?,
                created: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn set_synced(&self, id: i64, state: i64) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute("UPDATE items SET synced=?1 WHERE id=?2", params![state, id])?;
        Ok(())
    }

    /// Adds an item pulled from the bucket. Returns false if it (or identical content) already exists.
    pub fn insert_remote(&self, it: &Upload) -> rusqlite::Result<bool> {
        let c = self.conn.lock().unwrap();
        let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM items WHERE uuid=?1)", [&it.uuid], |r| r.get(0))?;
        if exists {
            return Ok(false);
        }
        let preview: String = it.content.chars().take(PREVIEW_CHARS).collect();
        let n = c.execute(
            "INSERT OR IGNORE INTO items(uuid,kind,category,preview,content,blob,hash,created,sync,synced) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,1)",
            params![it.uuid, it.kind, it.category, preview, it.content, it.blob, it.hash, it.created],
        )?;
        Ok(n > 0)
    }

    /// Removes an item because it was deleted elsewhere. Outer `Some` means a row existed.
    pub fn delete_by_uuid(&self, uuid: &str) -> rusqlite::Result<Option<Option<String>>> {
        let c = self.conn.lock().unwrap();
        let row: Option<Option<String>> = c.query_row("SELECT blob FROM items WHERE uuid=?1", [uuid], |r| r.get(0)).optional()?;
        if row.is_some() {
            c.execute("DELETE FROM items WHERE uuid=?1", [uuid])?;
        }
        Ok(row)
    }

    pub fn pending_deletes(&self) -> rusqlite::Result<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let mut st = c.prepare("SELECT uuid FROM pending_deletes")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn clear_pending_delete(&self, uuid: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM pending_deletes WHERE uuid=?1", [uuid])?;
        Ok(())
    }

    pub fn seen_etag(&self, uuid: &str) -> Option<String> {
        let c = self.conn.lock().unwrap();
        c.query_row("SELECT etag FROM seen_remote WHERE uuid=?1", [uuid], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_seen(&self, uuid: &str, etag: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO seen_remote(uuid,etag) VALUES(?1,?2) ON CONFLICT(uuid) DO UPDATE SET etag=?2",
            params![uuid, etag],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> Store {
        let p = std::env::temp_dir().join(format!("cvp-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        Store::open(&p).unwrap()
    }

    fn text(content: &str, category: &'static str) -> NewItem {
        NewItem { kind: "text", category, content: content.into(), blob: None, hash: format!("h-{content}") }
    }

    #[test]
    fn uploads_follow_categories_and_flags() {
        let s = temp_store("uploads");
        s.upsert(text("a code", "code")).unwrap();
        s.upsert(text("a note", "text")).unwrap();
        s.upsert(NewItem { kind: "file", category: "file", content: "/x".into(), blob: None, hash: "hf".into() }).unwrap();

        let cats = vec!["code".to_string()];
        let up = s.pending_uploads(&cats).unwrap();
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].content, "a code");

        // Flagging an item with ☁ includes it; files are never uploaded.
        let note = s.list(Some("text"), "", 10).unwrap()[0].id;
        s.set_flag(note, "sync", true).unwrap();
        assert_eq!(s.pending_uploads(&cats).unwrap().len(), 2);
        let file = s.list(Some("file"), "", 10).unwrap()[0].id;
        s.set_flag(file, "sync", true).unwrap();
        assert_eq!(s.pending_uploads(&[]).unwrap().len(), 1);

        s.set_synced(up[0].id, 1).unwrap();
        assert_eq!(s.pending_uploads(&cats).unwrap().len(), 1);
    }

    #[test]
    fn remote_insert_dedups_and_deletes_become_tombstones() {
        let s = temp_store("remote");
        let remote = Upload { id: 0, uuid: "u1".into(), kind: "text".into(), category: "text".into(), content: "hi".into(), blob: None, hash: "hh".into(), created: 1 };
        assert!(s.insert_remote(&remote).unwrap());
        assert!(!s.insert_remote(&remote).unwrap(), "same uuid is ignored");
        let same_hash = Upload { uuid: "u2".into(), ..remote };
        assert!(!s.insert_remote(&same_hash).unwrap(), "same content is ignored");

        // A pulled item is marked synced, so deleting it queues a tombstone.
        let id = s.list(None, "", 10).unwrap()[0].id;
        s.delete(id).unwrap();
        assert_eq!(s.pending_deletes().unwrap(), vec!["u1".to_string()]);

        // An item that was never uploaded leaves nothing to propagate.
        s.upsert(text("local only", "text")).unwrap();
        let id = s.list(None, "", 10).unwrap()[0].id;
        s.delete(id).unwrap();
        assert_eq!(s.pending_deletes().unwrap().len(), 1);
    }

    #[test]
    fn remote_delete_removes_local_without_tombstone() {
        let s = temp_store("tomb");
        let remote = Upload { id: 0, uuid: "u9".into(), kind: "text".into(), category: "text".into(), content: "bye".into(), blob: None, hash: "hb".into(), created: 1 };
        s.insert_remote(&remote).unwrap();
        assert!(s.delete_by_uuid("u9").unwrap().is_some());
        assert!(s.delete_by_uuid("u9").unwrap().is_none());
        assert!(s.pending_deletes().unwrap().is_empty());
    }
}
