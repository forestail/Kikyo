use kikyo_core::chord_engine::Profile;
use kikyo_core::engine::ENGINE;
use kikyo_core::{keyboard_hook, parser};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

struct AppState {
    current_yab_path: Mutex<Option<String>>,
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

    Ok(stats)
}

#[tauri::command]
fn set_enabled(app: tauri::AppHandle, enabled: bool) {
    ENGINE.lock().set_enabled(enabled);
    let _ = update_tray_menu(&app);
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
                    }
                    _ => {}
                })
                .icon(app.default_window_icon().unwrap().clone())
                .build(app)?;

            // Update to correct initial state
            update_tray_menu(app.handle())?;

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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
