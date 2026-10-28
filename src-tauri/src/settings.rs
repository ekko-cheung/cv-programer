use serde::{Deserialize, Serialize};

pub const MIN_ITEMS: i64 = 50;
pub const MAX_ITEMS: i64 = 20_000;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct S3Config {
    /// Empty means AWS S3 for `region`; otherwise an S3-compatible endpoint (R2, MinIO, OSS…).
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub access_key: String,
    /// Use `endpoint/bucket/key` URLs instead of `bucket.endpoint` (needed by MinIO and some others).
    pub path_style: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Global shortcut accelerator, e.g. "Super+Shift+V".
    pub hotkey: String,
    /// Unpinned items kept in history.
    pub max_items: i64,
    pub s3: S3Config,
}

pub fn default_hotkey() -> String {
    if cfg!(target_os = "macos") { "Super+Shift+V" } else { "Control+Shift+V" }.into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: default_hotkey(), max_items: 2000, s3: S3Config::default() }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.max_items = self.max_items.clamp(MIN_ITEMS, MAX_ITEMS);
        self
    }
}
