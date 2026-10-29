//! Two-way sync with an S3-compatible bucket.
//!
//! Layout under the configured prefix: `items/<uuid>.json` (one object per item, or a
//! tombstone `{"deleted":true}` after deletion) and `blobs/<sha256>.png` for images.
use crate::{s3, settings::SYNC_CATEGORIES, store::Upload, AppState};
use ::s3::Bucket;
use serde::{Deserialize, Serialize};
use std::{fs, sync::{atomic::Ordering, Arc}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone, Default)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub deleted: usize,
    pub skipped: usize,
}

#[derive(Serialize, Clone, Default)]
pub struct Status {
    pub at: i64,
    pub error: Option<String>,
    pub report: Report,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Remote {
    uuid: String,
    deleted: bool,
    kind: String,
    category: String,
    content: String,
    hash: String,
    created: i64,
}

fn dberr(e: rusqlite::Error) -> String {
    e.to_string()
}

/// Uploads `bytes`, returning the object's ETag when the server reports one.
fn put(bucket: &Bucket, key: &str, bytes: &[u8]) -> Result<Option<String>, String> {
    let r = bucket.put_object(key, bytes).map_err(|e| format!("Upload failed: {e}"))?;
    if !(200..300).contains(&r.status_code()) {
        return Err(format!("Upload failed (HTTP {}): {}", r.status_code(), String::from_utf8_lossy(r.as_slice())));
    }
    Ok(r.headers().iter().find(|(k, _)| k.eq_ignore_ascii_case("etag")).map(|(_, v)| v.trim_matches('"').to_string()))
}

fn get(bucket: &Bucket, key: &str) -> Result<Vec<u8>, String> {
    let r = bucket.get_object(key).map_err(|e| format!("Download failed: {e}"))?;
    if r.status_code() != 200 {
        return Err(format!("Download failed (HTTP {}) for {key}", r.status_code()));
    }
    Ok(r.as_slice().to_vec())
}

fn safe_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn run(state: &AppState) -> Result<Report, String> {
    let _guard = state.sync_lock.try_lock().map_err(|_| "A sync is already running.".to_string())?;
    let cfg = state.settings.lock().unwrap().clone();
    let secret = s3::load_secret().ok_or("S3 secret key is not set (Settings → S3 storage).")?;
    let bucket = s3::bucket(&cfg.s3, &secret)?;
    let store = &state.store;
    let mut rep = Report::default();
    let item_key = |uuid: &str| s3::key(&cfg.s3, &format!("items/{uuid}.json"));
    let blob_key = |hash: &str| s3::key(&cfg.s3, &format!("blobs/{hash}.png"));

    // 1. Propagate local deletions.
    for uuid in store.pending_deletes().map_err(dberr)? {
        let body = serde_json::to_vec(&Remote { uuid: uuid.clone(), deleted: true, ..Default::default() }).map_err(|e| e.to_string())?;
        if let Some(etag) = put(&bucket, &item_key(&uuid), &body)? {
            store.set_seen(&uuid, &etag).map_err(dberr)?;
        }
        store.clear_pending_delete(&uuid).map_err(dberr)?;
        rep.deleted += 1;
    }

    // 2. Upload new items.
    let max_bytes = cfg.sync.max_blob_mb * 1024 * 1024;
    for u in store.pending_uploads(&cfg.sync.categories).map_err(dberr)? {
        let too_big = if u.kind == "image" {
            match u.blob.as_deref().map(fs::read) {
                Some(Ok(bytes)) if bytes.len() as u64 <= max_bytes => {
                    put(&bucket, &blob_key(&u.hash), &bytes)?;
                    false
                }
                _ => true,
            }
        } else {
            u.content.len() as u64 > max_bytes
        };
        if too_big {
            store.set_synced(u.id, 2).map_err(dberr)?;
            rep.skipped += 1;
            continue;
        }
        let remote = Remote { uuid: u.uuid.clone(), deleted: false, kind: u.kind, category: u.category, content: u.content, hash: u.hash, created: u.created };
        let body = serde_json::to_vec(&remote).map_err(|e| e.to_string())?;
        if let Some(etag) = put(&bucket, &item_key(&u.uuid), &body)? {
            store.set_seen(&u.uuid, &etag).map_err(dberr)?;
        }
        store.set_synced(u.id, 1).map_err(dberr)?;
        rep.uploaded += 1;
    }

    // 3. Pull changes made elsewhere.
    let prefix = s3::key(&cfg.s3, "items/");
    for page in bucket.list(prefix.clone(), None).map_err(|e| format!("List failed: {e}"))? {
        for obj in page.contents {
            let Some(uuid) = obj.key.strip_prefix(&prefix).and_then(|n| n.strip_suffix(".json")) else { continue };
            if !safe_id(uuid) {
                continue;
            }
            let etag = obj.e_tag.as_deref().unwrap_or("").trim_matches('"').to_string();
            if !etag.is_empty() && store.seen_etag(uuid).as_deref() == Some(etag.as_str()) {
                continue;
            }
            let Ok(remote) = serde_json::from_slice::<Remote>(&get(&bucket, &obj.key)?) else { continue };

            if remote.deleted {
                if let Some(blob) = store.delete_by_uuid(uuid).map_err(dberr)? {
                    if let Some(p) = blob {
                        let _ = fs::remove_file(p);
                    }
                    rep.deleted += 1;
                }
            } else if remote.kind == "text" || remote.kind == "image" {
                let mut blob = None;
                let mut ok = true;
                if remote.kind == "image" {
                    // The hash becomes a local file name, so only accept a plain sha256 hex string.
                    if remote.hash.len() == 64 && remote.hash.chars().all(|c| c.is_ascii_hexdigit()) {
                        let path = state.blob_dir.join(format!("{}.png", remote.hash));
                        if !path.exists() {
                            fs::write(&path, get(&bucket, &blob_key(&remote.hash))?).map_err(|e| e.to_string())?;
                        }
                        blob = Some(path.to_string_lossy().into_owned());
                    } else {
                        ok = false;
                    }
                }
                if ok {
                    let category = if SYNC_CATEGORIES.contains(&remote.category.as_str()) { remote.category } else { "text".into() };
                    let item = Upload { id: 0, uuid: uuid.to_string(), kind: remote.kind, category, content: remote.content, blob, hash: remote.hash, created: remote.created };
                    if store.insert_remote(&item).map_err(dberr)? {
                        rep.downloaded += 1;
                    }
                }
            }
            if !etag.is_empty() {
                store.set_seen(uuid, &etag).map_err(dberr)?;
            }
        }
    }
    Ok(rep)
}

/// Runs a sync, records its outcome for the settings page and refreshes open panels.
pub fn run_and_record(app: &AppHandle, state: &AppState) -> Result<Report, String> {
    let result = run(state);
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let mut status = Status { at, ..Default::default() };
    match &result {
        Ok(r) => {
            status.report = r.clone();
            if r.downloaded + r.deleted > 0 {
                let _ = app.emit("items-changed", ());
            }
        }
        Err(e) => status.error = Some(e.clone()),
    }
    *state.sync_status.lock().unwrap() = status;
    result
}

/// Background loop: syncs shortly after local changes and at the configured interval.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let state = app.state::<Arc<AppState>>();
            let (enabled, interval) = {
                let s = state.settings.lock().unwrap();
                (s.sync.enabled, s.sync.interval_secs)
            };
            if !enabled {
                continue;
            }
            let dirty = state.sync_dirty.swap(false, Ordering::Relaxed);
            if dirty || last.map_or(true, |t| t.elapsed().as_secs() >= interval) {
                last = Some(Instant::now());
                let _ = run_and_record(&app, &state);
            }
        }
    });
}
