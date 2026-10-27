use serde::{Deserialize, Serialize};

pub const MIN_ITEMS: i64 = 50;
pub const MAX_ITEMS: i64 = 20_000;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// Global shortcut accelerator, e.g. "Super+Shift+V".
    pub hotkey: String,
    /// Unpinned items kept in history.
    pub max_items: i64,
}

pub fn default_hotkey() -> String {
    if cfg!(target_os = "macos") { "Super+Shift+V" } else { "Control+Shift+V" }.into()
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: default_hotkey(), max_items: 2000 }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.max_items = self.max_items.clamp(MIN_ITEMS, MAX_ITEMS);
        self
    }
}
