use image::GenericImageView;
use kikyo_core::chord_engine::Profile;
use kikyo_core::engine::ENGINE;
use kikyo_core::{keyboard_hook, parser};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::Manager;
use tauri::WindowEvent;

static ENTRY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
const TRAY_LAYOUT_ITEM_ID_PREFIX: &str = "layout_entry::";
const DUPLICATE_LAYOUT_PATH_MESSAGE: &str = "\u{3059}\u{3067}\u{306b}\u{767b}\u{9332}\u{3055}\u{308c}\u{3066}\u{3044}\u{308b}\u{5b9a}\u{7fa9}\u{30d5}\u{30a1}\u{30a4}\u{30eb}\u{3067}\u{3059}";

fn tray_layout_item_menu_id(entry_id: &str) -> String {
    format!("{TRAY_LAYOUT_ITEM_ID_PREFIX}{entry_id}")
}

fn tray_layout_id_from_menu_id(menu_id: &str) -> Option<&str> {
    menu_id.strip_prefix(TRAY_LAYOUT_ITEM_ID_PREFIX)
}

struct AppState {
    current_yab_path: Mutex<Option<String>>,
    layout_name: Mutex<Option<String>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct LayoutEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    layout_name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    order: usize,
}

#[derive(serde::Serialize)]
struct LayoutEntriesResponse {
    entries: Vec<LayoutEntry>,
    active_layout_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Settings {
    #[serde(default, alias = "last_yab_path")]
    last_layout_path: Option<String>,
    #[serde(default)]
    layout_entries: Vec<LayoutEntry>,
    #[serde(default)]
    active_layout_id: Option<String>,
    #[serde(default)]
    profile: Option<Profile>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            last_layout_path: None,
            layout_entries: Vec::new(),
            active_layout_id: None,
            profile: None,
            enabled: true,
        }
    }
}

fn generate_layout_entry_id() -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = ENTRY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("layout-{}-{}", now_ms, seq)
}

fn fallback_alias_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "layout".to_string())
}

fn normalize_layout_path_for_compare(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        path.trim().replace('/', "\\").to_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.trim().to_string()
    }
}

fn detect_layout_name_from_file(path: &str) -> Result<String, String> {
    let layout = parser::load_yab(path).map_err(|e| e.to_string())?;
    let name = layout
        .name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback_alias_from_path(path));
    Ok(name)
}

fn preferred_entry_display_name(entry: &LayoutEntry) -> String {
    let alias = entry.alias.trim();
    if !alias.is_empty() {
        return alias.to_string();
    }

    let layout_name = entry.layout_name.trim();
    if !layout_name.is_empty() {
        return layout_name.to_string();
    }

    fallback_alias_from_path(&entry.path)
}

fn preferred_display_name_for_path(settings: &Settings, path: &str) -> Option<String> {
    if let Some(active_id) = settings.active_layout_id.as_ref() {
        if let Some(active_entry) = settings
            .layout_entries
            .iter()
            .find(|entry| &entry.id == active_id && entry.path == path)
        {
            return Some(preferred_entry_display_name(active_entry));
        }
    }

    settings
        .layout_entries
        .iter()
        .find(|entry| entry.path == path)
        .map(preferred_entry_display_name)
}

fn normalize_layout_entry(entry: &mut LayoutEntry) -> bool {
    let mut changed = false;

    let path = entry.path.trim().to_string();
    if path != entry.path {
        entry.path = path;
        changed = true;
    }

    let alias = entry.alias.trim().to_string();
    if alias != entry.alias {
        entry.alias = alias;
        changed = true;
    }

    let layout_name = entry.layout_name.trim().to_string();
    if layout_name != entry.layout_name {
        entry.layout_name = layout_name;
        changed = true;
    }

    if entry.id.trim().is_empty() {
        entry.id = generate_layout_entry_id();
        changed = true;
    }

    if entry.layout_name.trim().is_empty() {
        entry.layout_name = if !entry.alias.trim().is_empty() {
            entry.alias.clone()
        } else {
            fallback_alias_from_path(&entry.path)
        };
        changed = true;
    }

    if entry.alias.trim().is_empty() {
        entry.alias = entry.layout_name.clone();
        changed = true;
    }

    changed
}

fn refresh_layout_entry_order(settings: &mut Settings) -> bool {
    let mut changed = false;
    for (idx, entry) in settings.layout_entries.iter_mut().enumerate() {
        if entry.order != idx {
            entry.order = idx;
            changed = true;
        }
    }
    changed
}

fn sync_last_path_with_active(settings: &mut Settings) -> bool {
    if let Some(active_id) = settings.active_layout_id.as_ref() {
        if let Some(active_entry) = settings
            .layout_entries
            .iter()
            .find(|entry| &entry.id == active_id)
        {
            if settings.last_layout_path.as_deref() != Some(active_entry.path.as_str()) {
                settings.last_layout_path = Some(active_entry.path.clone());
                return true;
            }
        }
    }
    false
}

fn migrate_settings(settings: &mut Settings) -> bool {
    let mut changed = false;

    for entry in &mut settings.layout_entries {
        if normalize_layout_entry(entry) {
            changed = true;
        }
    }

    let old_len = settings.layout_entries.len();
    settings
        .layout_entries
        .retain(|entry| !entry.path.trim().is_empty());
    if settings.layout_entries.len() != old_len {
        changed = true;
    }

    if settings.layout_entries.is_empty() {
        if let Some(path) = settings
            .last_layout_path
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            let layout_name = detect_layout_name_from_file(&path)
                .unwrap_or_else(|_| fallback_alias_from_path(&path));
            settings.layout_entries.push(LayoutEntry {
                id: generate_layout_entry_id(),
                alias: layout_name.clone(),
                layout_name,
                path,
                order: 0,
            });
            changed = true;
        }
    }

    if settings.active_layout_id.is_none() && !settings.layout_entries.is_empty() {
        settings.active_layout_id = Some(settings.layout_entries[0].id.clone());
        changed = true;
    }

    if let Some(active_id) = settings.active_layout_id.as_ref() {
        if !settings
            .layout_entries
            .iter()
            .any(|entry| &entry.id == active_id)
        {
            settings.active_layout_id = settings
                .layout_entries
                .first()
                .map(|entry| entry.id.clone());
            changed = true;
        }
    }

    if sync_last_path_with_active(settings) {
        changed = true;
    }

    if refresh_layout_entry_order(settings) {
        changed = true;
    }

    changed
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
    Settings::default()
}

fn load_settings_with_migration(app: &tauri::AppHandle) -> Settings {
    let mut settings = load_settings(app);
    if migrate_settings(&mut settings) {
        save_settings(app, &settings);
    }
    settings
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

fn sanitize_profile_for_save(mut profile: Profile) -> Profile {
    // Keep only user-facing settings; derived layout data is re-built on load.
    profile.thumb_keys = None;
    profile.trigger_keys.clear();
    profile.target_keys = None;
    profile
}

fn update_tray_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let layout_name = app.state::<AppState>().layout_name.lock().unwrap().clone();
    let enabled = ENGINE.lock().is_enabled();
    update_tray_menu_with_state(app, layout_name, enabled)
}

fn update_tray_menu_with_state(
    app: &tauri::AppHandle,
    layout_name: Option<String>,
    enabled: bool,
) -> tauri::Result<()> {
    let settings = load_settings_with_migration(app);
    let active_layout_id = settings.active_layout_id.clone();
    let active_name = active_layout_id
        .as_ref()
        .and_then(|active_id| {
            settings
                .layout_entries
                .iter()
                .find(|entry| &entry.id == active_id)
        })
        .map(preferred_entry_display_name);
    let name_text = layout_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or(active_name)
        .unwrap_or_else(|| "配列定義なし".to_string());

    let menu = Menu::new(app)?;
    if settings.layout_entries.is_empty() {
        let item_empty =
            MenuItem::with_id(app, "layout_name", "配列定義なし", false, None::<&str>)?;
        menu.append(&item_empty)?;
    } else {
        for entry in &settings.layout_entries {
            let display_name = preferred_entry_display_name(entry);
            let item = CheckMenuItem::with_id(
                app,
                tray_layout_item_menu_id(&entry.id),
                display_name,
                true,
                active_layout_id.as_deref() == Some(entry.id.as_str()),
                None::<&str>,
            )?;
            menu.append(&item)?;
        }
    }

    // Separator
    let sep1 = PredefinedMenuItem::separator(app)?;
    menu.append(&sep1)?;

    // Reload & Settings
    let item_reload = MenuItem::with_id(app, "reload", "配列定義再読み込み", true, None::<&str>)?;
    let item_settings = MenuItem::with_id(app, "show", "設定", true, None::<&str>)?;
    menu.append(&item_reload)?;
    menu.append(&item_settings)?;

    // Separator
    let sep2 = PredefinedMenuItem::separator(app)?;
    menu.append(&sep2)?;

    // Toggle
    let toggle_text = if enabled { "一時停止" } else { "再開" };
    let item_toggle = MenuItem::with_id(app, "toggle", toggle_text, true, None::<&str>)?;
    menu.append(&item_toggle)?;

    // Separator
    let sep3 = PredefinedMenuItem::separator(app)?;
    menu.append(&sep3)?;

    // Quit
    let item_quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    menu.append(&item_quit)?;

    if let Some(tray) = app.tray_by_id("kikyo-tray") {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(format!("Kikyo: {}", name_text)))?;

        let icon_bytes = include_bytes!("../icons/128x128.png");
        match image::load_from_memory(icon_bytes) {
            Ok(mut img) => {
                let (width, height) = img.dimensions();

                if !enabled {
                    // Draw a red diagonal line
                    // Simple algorithm: line thickness = 10% of width
                    let thickness = (width as i32) / 10;
                    let mut rgba_img = img.to_rgba8();

                    for x in 0..width {
                        for y in 0..height {
                            // Check if point (x, y) is close to the diagonal x=y
                            let dist = (x as i32 - y as i32).abs();
                            if dist < thickness / 2 {
                                // Set to Red (255, 0, 0, 255)
                                rgba_img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
                            }
                        }
                    }
                    img = image::DynamicImage::ImageRgba8(rgba_img);
                }

                let rgba_bytes = img.into_rgba8().into_raw();
                let icon = Image::new(&rgba_bytes, width, height);
                if let Err(e) = tray.set_icon(Some(icon)) {
                    tracing::error!("Failed to set tray icon: {}", e);
                } else {
                    tracing::info!("Tray icon updated successfully");
                }
            }
            Err(e) => tracing::error!("Failed to load icon from memory: {}", e),
        }
    } else {
        tracing::warn!("Tray 'kikyo-tray' not found");
    }

    Ok(())
}

fn update_window_title(app: &tauri::AppHandle, layout_name: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let title_text = if let Some(name) = layout_name {
            format!("桔梗 - {}", name)
        } else {
            "桔梗 - 配列定義なし".to_string()
        };
        let _ = window.set_title(&title_text);
    }
}

fn apply_layout_from_path(
    app: &tauri::AppHandle,
    state: &AppState,
    path: &str,
    display_name: Option<String>,
) -> Result<String, String> {
    let layout = parser::load_yab(path).map_err(|e| e.to_string())?;
    let stats = format!("Loaded {} sections", layout.sections.len());
    let parser_name = layout
        .name
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback_alias_from_path(path));
    ENGINE.lock().load_layout(layout);
    keyboard_hook::refresh_runtime_flags_from_engine();

    let resolved_display_name = display_name
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or(parser_name);

    *state.current_yab_path.lock().unwrap() = Some(path.to_string());
    *state.layout_name.lock().unwrap() = Some(resolved_display_name.clone());
    let enabled = ENGINE.lock().is_enabled();
    let _ = update_tray_menu_with_state(app, Some(resolved_display_name.clone()), enabled);
    update_window_title(app, Some(resolved_display_name.as_str()));
    Ok(stats)
}

fn activate_layout_entry_by_id(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
) -> Result<String, String> {
    let mut settings = load_settings_with_migration(app);
    let entry = settings
        .layout_entries
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| "Layout entry not found".to_string())?;

    let display_name = preferred_entry_display_name(&entry);
    let stats = apply_layout_from_path(app, state, &entry.path, Some(display_name))?;
    settings.active_layout_id = Some(entry.id);
    settings.last_layout_path = Some(entry.path);
    save_settings(app, &settings);
    let _ = update_tray_menu(app);
    Ok(stats)
}

#[tauri::command]
fn load_yab(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    path: String,
) -> Result<String, String> {
    let mut settings = load_settings_with_migration(&app);
    settings.last_layout_path = Some(path.clone());
    settings.active_layout_id = settings
        .layout_entries
        .iter()
        .find(|entry| entry.path == path.as_str())
        .map(|entry| entry.id.clone());
    let display_name = preferred_display_name_for_path(&settings, &path);
    let stats = apply_layout_from_path(&app, &state, &path, display_name)?;
    save_settings(&app, &settings);
    let _ = update_tray_menu(&app);
    Ok(stats)
}

#[tauri::command]
fn set_enabled(_app: tauri::AppHandle, enabled: bool) {
    ENGINE.lock().set_enabled(enabled);
}

#[tauri::command]
fn get_enabled() -> bool {
    ENGINE.lock().is_enabled()
}

#[tauri::command]
fn get_profile() -> Profile {
    let profile = ENGINE.lock().get_profile();
    // Remove layout-derived fields so JSON serialization works for UI.
    sanitize_profile_for_save(profile)
}

#[tauri::command]
fn set_profile(app: tauri::AppHandle, profile: Profile) {
    ENGINE.lock().set_profile(profile.clone());
    keyboard_hook::refresh_runtime_flags_from_engine();
    let mut settings = load_settings_with_migration(&app);
    settings.profile = Some(sanitize_profile_for_save(profile));
    save_settings(&app, &settings);
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn get_layout_entries(app: tauri::AppHandle) -> LayoutEntriesResponse {
    let settings = load_settings_with_migration(&app);
    LayoutEntriesResponse {
        entries: settings.layout_entries,
        active_layout_id: settings.active_layout_id,
    }
}

#[tauri::command]
fn create_layout_entry_from_path(
    app: tauri::AppHandle,
    path: String,
) -> Result<LayoutEntry, String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("Path is empty".to_string());
    }

    let mut settings = load_settings_with_migration(&app);
    let normalized = normalize_layout_path_for_compare(&path);
    if settings.layout_entries.iter().any(|entry| {
        normalize_layout_path_for_compare(&entry.path) == normalized
    }) {
        return Err(DUPLICATE_LAYOUT_PATH_MESSAGE.to_string());
    }
    let layout_name = detect_layout_name_from_file(&path)?;
    let entry = LayoutEntry {
        id: generate_layout_entry_id(),
        alias: layout_name.clone(),
        layout_name,
        path,
        order: settings.layout_entries.len(),
    };
    settings.layout_entries.push(entry.clone());
    let _ = refresh_layout_entry_order(&mut settings);
    if settings.active_layout_id.is_none() {
        settings.active_layout_id = Some(entry.id.clone());
        let _ = sync_last_path_with_active(&mut settings);
    }
    save_settings(&app, &settings);
    let _ = update_tray_menu(&app);
    Ok(entry)
}

#[tauri::command]
fn update_layout_entry(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
    alias: String,
    path: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("Path is empty".to_string());
    }

    let mut settings = load_settings_with_migration(&app);
    let is_active = settings.active_layout_id.as_deref() == Some(id.as_str());
    let mut active_display_name: Option<String> = None;
    {
        let entry = settings
            .layout_entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Layout entry not found".to_string())?;

        let path_changed = entry.path != path;
        entry.path = path;
        if path_changed {
            entry.layout_name = detect_layout_name_from_file(&entry.path)
                .unwrap_or_else(|_| fallback_alias_from_path(&entry.path));
        }

        let alias = alias.trim().to_string();
        entry.alias = if alias.is_empty() {
            entry.layout_name.clone()
        } else {
            alias
        };

        if is_active {
            active_display_name = Some(preferred_entry_display_name(entry));
        }
    }

    let _ = sync_last_path_with_active(&mut settings);
    save_settings(&app, &settings);

    if let Some(display_name) = active_display_name {
        *state.layout_name.lock().unwrap() = Some(display_name.clone());
        update_window_title(&app, Some(display_name.as_str()));
    }
    let _ = update_tray_menu(&app);

    Ok(())
}

#[tauri::command]
fn delete_layout_entry(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let mut settings = load_settings_with_migration(&app);
    let old_len = settings.layout_entries.len();
    settings.layout_entries.retain(|entry| entry.id != id);
    if settings.layout_entries.len() == old_len {
        return Err("Layout entry not found".to_string());
    }

    if settings.active_layout_id.as_deref() == Some(id.as_str()) {
        settings.active_layout_id = settings
            .layout_entries
            .first()
            .map(|entry| entry.id.clone());
    }

    let _ = refresh_layout_entry_order(&mut settings);
    let _ = sync_last_path_with_active(&mut settings);
    save_settings(&app, &settings);
    let _ = update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
fn reorder_layout_entries(app: tauri::AppHandle, ordered_ids: Vec<String>) -> Result<(), String> {
    let mut settings = load_settings_with_migration(&app);
    if ordered_ids.len() != settings.layout_entries.len() {
        return Err("Invalid number of layout ids".to_string());
    }

    let mut by_id: HashMap<String, LayoutEntry> = HashMap::new();
    for entry in settings.layout_entries.drain(..) {
        by_id.insert(entry.id.clone(), entry);
    }

    let mut reordered = Vec::with_capacity(ordered_ids.len());
    for id in ordered_ids {
        let entry = by_id
            .remove(&id)
            .ok_or_else(|| "Unknown layout id".to_string())?;
        reordered.push(entry);
    }

    if !by_id.is_empty() {
        return Err("Some layout ids are missing in order payload".to_string());
    }

    settings.layout_entries = reordered;
    let _ = refresh_layout_entry_order(&mut settings);
    save_settings(&app, &settings);
    let _ = update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
fn activate_layout_entry(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    id: String,
) -> Result<String, String> {
    activate_layout_entry_by_id(&app, &state, id.as_str())
}

#[cfg(test)]
mod tests {
    use super::{normalize_layout_path_for_compare, Settings};

    #[test]
    fn settings_default_enabled_is_true() {
        assert!(Settings::default().enabled);
    }

    #[test]
    fn settings_deserialize_without_enabled_defaults_to_true() {
        let parsed: Settings = serde_json::from_str("{}").expect("settings json");
        assert!(parsed.enabled);
    }

    #[test]
    fn settings_deserialize_legacy_last_yab_path_into_last_layout_path() {
        let parsed: Settings =
            serde_json::from_str(r#"{"last_yab_path":"C:\\layouts\\legacy.yab"}"#)
                .expect("legacy settings json");
        assert_eq!(
            parsed.last_layout_path.as_deref(),
            Some(r"C:\layouts\legacy.yab")
        );
    }

    #[test]
    fn settings_serialize_uses_last_layout_path_key() {
        let mut settings = Settings::default();
        settings.last_layout_path = Some("layout.yab".to_string());
        let value = serde_json::to_value(settings).expect("serialize settings");
        assert_eq!(
            value.get("last_layout_path").and_then(|v| v.as_str()),
            Some("layout.yab")
        );
        assert!(value.get("last_yab_path").is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn normalize_layout_path_for_compare_is_case_and_slash_insensitive_on_windows() {
        let a = normalize_layout_path_for_compare(r"C:/Layouts/Test.yab");
        let b = normalize_layout_path_for_compare(r"c:\layouts\test.yab");
        assert_eq!(a, b);
    }
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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(AppState {
            current_yab_path: Mutex::new(None),
            layout_name: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            load_yab,
            get_layout_entries,
            create_layout_entry_from_path,
            update_layout_entry,
            delete_layout_entry,
            reorder_layout_entries,
            activate_layout_entry,
            set_enabled,
            get_enabled,
            get_profile,
            set_profile,
            get_app_version
        ])
        .setup(|app| {
            // Setup Tray with initial menu
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit_i])?;

            let _tray = TrayIconBuilder::with_id("kikyo-tray")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    let event_id = event.id.as_ref();
                    match event_id {
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
                                let settings = load_settings_with_migration(app);
                                let display_name =
                                    preferred_display_name_for_path(&settings, &path);
                                match apply_layout_from_path(app, &state, &path, display_name) {
                                    Ok(_) => tracing::info!("Reloaded config from tray"),
                                    Err(e) => tracing::error!("Failed to reload config: {}", e),
                                }
                            }
                        }
                        "toggle" => {
                            let current = ENGINE.lock().is_enabled();
                            ENGINE.lock().set_enabled(!current);
                            let _ = update_tray_menu(app);
                            let _ = app.emit("enabled-state-changed", !current);
                        }
                        _ => {
                            if let Some(layout_id) = tray_layout_id_from_menu_id(event_id) {
                                let state = app.state::<AppState>();
                                match activate_layout_entry_by_id(app, &state, layout_id) {
                                    Ok(_) => {
                                        tracing::info!("Activated layout from tray: {}", layout_id)
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to activate layout from tray ({}): {}",
                                            layout_id,
                                            e
                                        );
                                        let _ = update_tray_menu(app);
                                    }
                                }
                            }
                        }
                    }
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

            // Load settings (profile first, then layout)
            let settings = load_settings_with_migration(app.handle());
            ENGINE.lock().set_enabled(settings.enabled);
            if let Some(profile) = settings.profile.as_ref() {
                ENGINE.lock().set_profile(profile.clone());
                keyboard_hook::refresh_runtime_flags_from_engine();
            }
            let startup_path = settings
                .active_layout_id
                .as_ref()
                .and_then(|active_id| {
                    settings
                        .layout_entries
                        .iter()
                        .find(|entry| &entry.id == active_id)
                        .map(|entry| entry.path.clone())
                })
                .or_else(|| settings.last_layout_path.clone());

            if let Some(path) = startup_path {
                let display_name = preferred_display_name_for_path(&settings, &path);
                let app_state = app.state::<AppState>();
                let _ = apply_layout_from_path(app.handle(), &app_state, &path, display_name);
            }

            // Update to correct initial state
            update_tray_menu(app.handle())?;

            // Initial Window Title Update
            {
                let layout_name = app.state::<AppState>().layout_name.lock().unwrap().clone();
                update_window_title(app.handle(), layout_name.as_deref());
            }

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
                let mut settings = load_settings_with_migration(&handle_for_cb);
                settings.enabled = enabled;
                save_settings(&handle_for_cb, &settings);
                let _ = handle_for_cb.emit("enabled-state-changed", enabled);
                let layout_name = handle_for_cb
                    .state::<AppState>()
                    .layout_name
                    .lock()
                    .unwrap()
                    .clone();
                let _ = update_tray_menu_with_state(&handle_for_cb, layout_name, enabled);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
