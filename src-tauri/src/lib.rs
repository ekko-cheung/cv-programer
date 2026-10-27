mod classify;
mod settings;
mod store;
mod watcher;

use clipboard_rs::{common::RustImage, Clipboard, ClipboardContext, RustImageData};
use serde::Serialize;
use settings::Settings;
use std::{fs, path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use store::{Item, Store};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    pub store: Store,
    pub blob_dir: PathBuf,
    pub settings: Mutex<Settings>,
}

type State<'a> = tauri::State<'a, Arc<AppState>>;

fn err<E: ToString>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
fn list_items(state: State, category: Option<String>, query: String) -> Result<Vec<Item>, String> {
    let category = category.filter(|c| c != "all");
    state.store.list(category.as_deref(), &query, 300).map_err(err)
}

#[derive(Serialize)]
struct Count {
    category: String,
    count: i64,
}

#[tauri::command]
fn category_counts(state: State) -> Result<Vec<Count>, String> {
    let v = state.store.counts().map_err(err)?;
    Ok(v.into_iter().map(|(category, count)| Count { category, count }).collect())
}

#[tauri::command]
fn delete_item(state: State, id: i64) -> Result<(), String> {
    if let Some(p) = state.store.delete(id).map_err(err)? {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

#[tauri::command]
fn set_pinned(state: State, id: i64, value: bool) -> Result<(), String> {
    state.store.set_flag(id, "pinned", value).map_err(err)
}

#[tauri::command]
fn set_sync(state: State, id: i64, value: bool) -> Result<(), String> {
    state.store.set_flag(id, "sync", value).map_err(err)
}

#[tauri::command]
fn get_settings(state: State) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State, settings: Settings) -> Result<Settings, String> {
    let settings = settings.sanitized();
    let new_key: Shortcut = settings.hotkey.parse().map_err(|_| format!("Invalid shortcut: {}", settings.hotkey))?;
    let old = state.settings.lock().unwrap().clone();
    if old.hotkey != settings.hotkey {
        let gs = app.global_shortcut();
        let old_key: Option<Shortcut> = old.hotkey.parse().ok();
        if let Some(k) = old_key {
            let _ = gs.unregister(k);
        }
        if let Err(e) = gs.register(new_key) {
            if let Some(k) = old_key {
                let _ = gs.register(k);
            }
            return Err(format!("Cannot register shortcut (already in use?): {e}"));
        }
    }
    state.store.set_kv("settings", &serde_json::to_string(&settings).map_err(err)?).map_err(err)?;
    *state.settings.lock().unwrap() = settings.clone();
    for p in state.store.prune(settings.max_items).map_err(err)? {
        let _ = fs::remove_file(p);
    }
    Ok(settings)
}

#[tauri::command]
fn clear_history(state: State, keep_pinned: bool) -> Result<(), String> {
    for p in state.store.clear(keep_pinned).map_err(err)? {
        let _ = fs::remove_file(p);
    }
    Ok(())
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    if let Some(p) = app.get_webview_window("panel") {
        let _ = p.close();
    }
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings".into()))
        .title("Settings")
        .inner_size(560.0, 520.0)
        .min_inner_size(480.0, 420.0)
        .center()
        .focused(true)
        .build();
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.close();
    }
}

/// Writes the item back to the system clipboard, closes the panel and sends the paste keystroke.
#[tauri::command]
fn paste_item(app: AppHandle, state: State, id: i64) -> Result<(), String> {
    let (kind, content, blob) = state.store.content(id).map_err(err)?.ok_or("item not found")?;
    let ctx = ClipboardContext::new().map_err(err)?;
    match kind.as_str() {
        "image" => {
            let img = RustImageData::from_path(&blob.ok_or("missing image")?).map_err(err)?;
            ctx.set_image(img).map_err(err)?;
        }
        "file" => ctx.set_files(content.lines().map(String::from).collect()).map_err(err)?,
        _ => ctx.set_text(content).map_err(err)?,
    }
    state.store.touch(id).map_err(err)?;
    hide_panel(app.clone());
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(180));
        let _ = handle.run_on_main_thread(simulate_paste);
    });
    Ok(())
}

fn simulate_paste() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let Ok(mut e) = Enigo::new(&Settings::default()) else { return };
    let m = if cfg!(target_os = "macos") { Key::Meta } else { Key::Control };
    let _ = e.key(m, Direction::Press);
    let _ = e.key(Key::Unicode('v'), Direction::Click);
    let _ = e.key(m, Direction::Release);
}

fn toggle_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.close();
        return;
    }
    let Ok(w) = WebviewWindowBuilder::new(app, "panel", WebviewUrl::App("index.html".into()))
        .title("Clipboard")
        .inner_size(600.0, 560.0)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .center()
        .focused(true)
        .build()
    else {
        return;
    };
    // As an Accessory (tray-only) app on macOS the window doesn't become key on its own.
    let _ = w.show();
    let _ = w.set_focus();
    let win = w.clone();
    w.on_window_event(move |ev| {
        if let WindowEvent::Focused(false) = ev {
            let _ = win.close();
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _sc, ev| {
                    // Only the toggle shortcut is ever registered.
                    if ev.state() == ShortcutState::Pressed {
                        toggle_panel(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            list_items,
            category_counts,
            delete_item,
            set_pinned,
            set_sync,
            hide_panel,
            paste_item,
            get_settings,
            save_settings,
            clear_history,
            open_settings
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let dir = app.path().app_data_dir()?;
            let blob_dir = dir.join("blobs");
            fs::create_dir_all(&blob_dir)?;
            let store = Store::open(&dir.join("clipboard.db"))?;
            let mut settings: Settings = store
                .get_kv("settings")
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            settings = settings.sanitized();
            let gs = app.global_shortcut();
            if !settings.hotkey.parse::<Shortcut>().is_ok_and(|k| gs.register(k).is_ok()) {
                // Stored shortcut is invalid or taken: fall back to the default.
                settings.hotkey = settings::default_hotkey();
                gs.register(settings.hotkey.parse::<Shortcut>()?)?;
            }
            app.manage(Arc::new(AppState { store, blob_dir, settings: Mutex::new(settings) }));

            let show = MenuItem::with_id(app, "show", "Show Clipboard", true, None::<&str>)?;
            let prefs = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &prefs, &quit])?;
            let builder = TrayIconBuilder::new();
            // macOS menu bar wants a monochrome template image; Windows keeps the colored app icon.
            #[cfg(target_os = "macos")]
            let builder = builder
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true);
            #[cfg(not(target_os = "macos"))]
            let builder = builder.icon(app.default_window_icon().unwrap().clone());
            builder
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "show" => toggle_panel(app),
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;

            watcher::spawn(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, ev| {
            // Keep running in the tray when the last window closes.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = ev {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
