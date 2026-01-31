use kikyo_core::chord_engine::Profile;
use kikyo_core::engine::ENGINE;
use kikyo_core::{keyboard_hook, parser};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::Manager;
use tauri::WindowEvent;

struct AppState {
    current_yab_path: Mutex<Option<String>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Settings {
    last_yab_path: Option<String>,
}

fn get_settings_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("settings.json"))
        .ok()
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    if let Some(path) = get_settings_path(app) {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(settings) = serde_json::from_str(&content) {
                    return settings;
                }
            }
        }
    }
    Settings {
        last_yab_path: None,
    }
}

fn save_settings(app: &tauri::AppHandle, settings: &Settings) {
    if let Some(path) = get_settings_path(app) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(content) = serde_json::to_string(settings) {
            let _ = fs::write(path, content);
        }
    }
}

fn update_tray_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let (layout_name, enabled) = {
        let engine = ENGINE.lock();
        (engine.get_layout_name(), engine.is_enabled())
    };

    let name_text = layout_name.unwrap_or_else(|| "未読み込み".to_string());
    // {LayoutName} (Disabled/Label)
    let item_name = MenuItem::with_id(app, "layout_name", &name_text, false, None::<&str>)?;

    // Separator
    let sep1 = PredefinedMenuItem::separator(app)?;

    // Reload & Settings
    let item_reload = MenuItem::with_id(app, "reload", "設定再読み込み", true, None::<&str>)?;
    let item_settings = MenuItem::with_id(app, "show", "設定", true, None::<&str>)?;

    // Separator
    let sep2 = PredefinedMenuItem::separator(app)?;

    // Toggle
    let toggle_text = if enabled { "一時停止" } else { "再開" };
    let item_toggle = MenuItem::with_id(app, "toggle", toggle_text, true, None::<&str>)?;

    // Separator
    let sep3 = PredefinedMenuItem::separator(app)?;

    // Quit
    let item_quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &item_name,
            &sep1,
            &item_reload,
            &item_settings,
            &sep2,
            &item_toggle,
            &sep3,
            &item_quit,
        ],
    )?;

    if let Some(tray) = app.tray_by_id("kikyo-tray") {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(format!("Kikyo: {}", name_text)))?;
    }

    Ok(())
}

#[tauri::command]
fn load_yab(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    path: String,
) -> Result<String, String> {
    let layout = parser::load_yab(&path).map_err(|e| e.to_string())?;
    let stats = format!("Loaded {} sections", layout.sections.len());
    ENGINE.lock().load_layout(layout);

    *state.current_yab_path.lock().unwrap() = Some(path.clone());
    let _ = update_tray_menu(&app);

    // Save settings
    let settings = Settings {
        last_yab_path: Some(path),
    };
    save_settings(&app, &settings);

    Ok(stats)
}

#[tauri::command]
fn set_enabled(app: tauri::AppHandle, enabled: bool) {
    ENGINE.lock().set_enabled(enabled);
    let _ = update_tray_menu(&app);
    let _ = app.emit("enabled-state-changed", enabled);
}

#[tauri::command]
fn get_enabled() -> bool {
    ENGINE.lock().is_enabled()
}

#[tauri::command]
fn get_profile() -> Profile {
    ENGINE.lock().get_profile()
}

#[tauri::command]
fn set_profile(profile: Profile) {
    ENGINE.lock().set_profile(profile);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState {
            current_yab_path: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            load_yab,
            set_enabled,
            get_enabled,
            get_profile,
            set_profile
        ])
        .setup(|app| {
            // Setup Tray with initial menu
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit_i])?;

            let _tray = TrayIconBuilder::with_id("kikyo-tray")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        std::process::exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "reload" => {
                        let state = app.state::<AppState>();
                        let path_opt = state.current_yab_path.lock().unwrap().clone();
                        if let Some(path) = path_opt {
                            match parser::load_yab(&path) {
                                Ok(layout) => {
                                    ENGINE.lock().load_layout(layout);
                                    let _ = update_tray_menu(app);
                                    tracing::info!("Reloaded config from tray");
                                }
                                Err(e) => {
                                    tracing::error!("Failed to reload config: {}", e);
                                }
                            }
                        }
                    }
                    "toggle" => {
                        let current = ENGINE.lock().is_enabled();
                        ENGINE.lock().set_enabled(!current);
                        let _ = update_tray_menu(app);
                        let _ = app.emit("enabled-state-changed", !current);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } => {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .icon(app.default_window_icon().unwrap().clone())
                .build(app)?;

            // Load settings
            let settings = load_settings(app.handle());
            if let Some(path) = settings.last_yab_path {
                if let Ok(layout) = parser::load_yab(&path) {
                    ENGINE.lock().load_layout(layout);
                    *app.state::<AppState>().current_yab_path.lock().unwrap() = Some(path);
                }
            }

            // Update to correct initial state
            update_tray_menu(app.handle())?;

            // Prepare Window Event for close
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = window_clone.hide();
                    }
                    _ => {}
                });
            }

            // Spawn Hook Thread
            std::thread::spawn(|| {
                tracing::info!("Hook thread started");
                match keyboard_hook::install_hook() {
                    Ok(_) => {
                        keyboard_hook::run_event_loop();
                    }
                    Err(e) => {
                        tracing::error!("Failed to install hook: {}", e);
                    }
                }
            });

            // Register callback for Engine state changes
            let handle_for_cb = app.handle().clone();
            ENGINE.lock().set_on_enabled_change(move |enabled| {
                let _ = handle_for_cb.emit("enabled-state-changed", enabled);
                let _ = update_tray_menu(&handle_for_cb);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
