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

pub const SYNC_CATEGORIES: [&str; 4] = ["text", "code", "link", "image"];

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SyncConfig {
    pub enabled: bool,
    /// Items in these categories are synced automatically; others only when flagged with ☁.
    pub categories: Vec<String>,
    /// How often to poll the bucket for remote changes.
    pub interval_secs: u64,
    /// Larger images are skipped.
    pub max_blob_mb: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self { enabled: false, categories: Vec::new(), interval_secs: 300, max_blob_mb: 10 }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Appearance {
    /// Translucent, blurred panel background (needs a transparent window, applied when the panel opens).
    pub glass: bool,
    /// Surface opacity over the blur, 30–95 (%).
    pub glass_opacity: u32,
    /// CSS font-family list tried before the system UI font; empty keeps the default.
    pub font_ui: String,
    /// Same, for code and shortcut labels.
    pub font_mono: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { glass: false, glass_opacity: 70, font_ui: String::new(), font_mono: String::new() }
    }
}

/// Keeps only characters that can appear in a font-family list, so the value can't break out of a CSS declaration.
fn clean_font(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || " ,'\"-_.".contains(*c)).take(200).collect::<String>().trim().to_string()
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Global shortcut accelerator, e.g. "Super+Shift+V".
    pub hotkey: String,
    /// Unpinned items kept in history.
    pub max_items: i64,
    pub s3: S3Config,
    pub sync: SyncConfig,
    pub appearance: Appearance,
}

pub fn default_hotkey() -> String {
    if cfg!(target_os = "macos") { "Super+Shift+V" } else { "Control+Shift+V" }.into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: default_hotkey(), max_items: 2000, s3: S3Config::default(), sync: SyncConfig::default(), appearance: Appearance::default() }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.max_items = self.max_items.clamp(MIN_ITEMS, MAX_ITEMS);
        self.appearance.glass_opacity = self.appearance.glass_opacity.clamp(30, 95);
        self.appearance.font_ui = clean_font(&self.appearance.font_ui);
        self.appearance.font_mono = clean_font(&self.appearance.font_mono);
        self.sync.interval_secs = self.sync.interval_secs.clamp(60, 3600);
        self.sync.max_blob_mb = self.sync.max_blob_mb.clamp(1, 100);
        self.sync.categories.retain(|c| SYNC_CATEGORIES.contains(&c.as_str()));
        self.sync.categories.dedup();
        self
    }
}
