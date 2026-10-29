use crate::{classify, store::NewItem, AppState};
use clipboard_rs::{
    common::RustImage, Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher,
    ClipboardWatcherContext, ContentFormat,
};
use sha2::{Digest, Sha256};
use std::{fs, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

const MAX_TEXT_BYTES: usize = 1 << 20;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Handler {
    app: AppHandle,
    ctx: ClipboardContext,
}

impl Handler {
    fn capture(&self) -> Option<NewItem> {
        let state = self.app.state::<Arc<AppState>>();
        if self.ctx.has(ContentFormat::Files) {
            let files = self.ctx.get_files().ok().filter(|f| !f.is_empty())?;
            let content = files.join("\n");
            let hash = hex(&Sha256::digest(format!("file:{content}")));
            return Some(NewItem { kind: "file", category: classify::FILE, content, blob: None, hash });
        }
        if self.ctx.has(ContentFormat::Text) {
            let text = self.ctx.get_text().ok()?;
            if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
                return None;
            }
            let hash = hex(&Sha256::digest(format!("text:{text}")));
            let category = classify::classify_text(&text);
            return Some(NewItem { kind: "text", category, content: text, blob: None, hash });
        }
        if self.ctx.has(ContentFormat::Image) {
            let img = self.ctx.get_image().ok()?;
            let (w, h) = img.get_size();
            let png = img.to_png().ok()?;
            let bytes = png.get_bytes();
            let hash = hex(&Sha256::digest(bytes));
            let path = state.blob_dir.join(format!("{hash}.png"));
            if !path.exists() {
                fs::write(&path, bytes).ok()?;
            }
            return Some(NewItem {
                kind: "image",
                category: classify::IMAGE,
                content: format!("Image {w}×{h}"),
                blob: Some(path.to_string_lossy().into_owned()),
                hash,
            });
        }
        None
    }
}

impl ClipboardHandler for Handler {
    fn on_clipboard_change(&mut self) {
        let Some(item) = self.capture() else { return };
        let state = self.app.state::<Arc<AppState>>();
        if let Err(e) = state.store.upsert(item) {
            eprintln!("store error: {e}");
            return;
        }
        state.sync_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Ok(orphans) = state.store.prune(state.settings.lock().unwrap().max_items) {
            for p in orphans {
                let _ = fs::remove_file(p);
            }
        }
        let _ = self.app.emit("items-changed", ());
    }
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let Ok(ctx) = ClipboardContext::new() else { return };
        let handler = Handler { app, ctx };
        #[cfg(target_os = "macos")]
        let watcher = ClipboardWatcherContext::new_with_interval(Duration::from_millis(300));
        #[cfg(not(target_os = "macos"))]
        let watcher = ClipboardWatcherContext::new();
        if let Ok(mut w) = watcher {
            w.add_handler(handler);
            w.start_watch();
        }
    });
}
