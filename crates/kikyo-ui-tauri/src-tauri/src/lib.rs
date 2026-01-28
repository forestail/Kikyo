use kikyo_core::chord_engine::Profile;
use kikyo_core::engine::ENGINE;
use kikyo_core::{keyboard_hook, parser};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

#[tauri::command]
fn load_yab(path: String) -> Result<String, String> {
    let layout = parser::load_yab(&path).map_err(|e| e.to_string())?;
    let stats = format!("Loaded {} sections", layout.sections.len());
    ENGINE.lock().load_layout(layout);
    Ok(stats)
}

#[tauri::command]
fn set_enabled(enabled: bool) {
    ENGINE.lock().set_enabled(enabled);
}

#[tauri::command]
fn get_enabled() -> bool {
    true
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
        .invoke_handler(tauri::generate_handler![
            load_yab,
            set_enabled,
            get_enabled,
            get_profile,
            set_profile
        ])
        .setup(|app| {
            // Setup Tray
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show_i = MenuItem::with_id(app, "show", "Open Settings", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

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
                    _ => {}
                })
                .icon(app.default_window_icon().unwrap().clone())
                .build(app)?;

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
