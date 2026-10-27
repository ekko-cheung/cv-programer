mod classify;
mod store;
mod watcher;

use clipboard_rs::{common::RustImage, Clipboard, ClipboardContext, RustImageData};
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use store::{Item, Store};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct AppState {
    pub store: Store,
    pub blob_dir: PathBuf,
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
    let mods = if cfg!(target_os = "macos") { Modifiers::SUPER | Modifiers::SHIFT } else { Modifiers::CONTROL | Modifiers::SHIFT };
    let hotkey = Shortcut::new(Some(mods), Code::KeyV);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, ev| {
                    if *sc == hotkey && ev.state() == ShortcutState::Pressed {
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
            paste_item
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let dir = app.path().app_data_dir()?;
            let blob_dir = dir.join("blobs");
            fs::create_dir_all(&blob_dir)?;
            let store = Store::open(&dir.join("clipboard.db"))?;
            app.manage(Arc::new(AppState { store, blob_dir }));

            app.global_shortcut().register(hotkey)?;

            let show = MenuItem::with_id(app, "show", "Show Clipboard", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "show" => toggle_panel(app),
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
