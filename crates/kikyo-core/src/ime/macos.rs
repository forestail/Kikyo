use crate::chord_engine::ImeMode;
use std::ffi::c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    pub static kTISPropertyInputSourceID: *mut c_void;
    pub static kTISPropertyInputSourceType: *mut c_void;
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(input_source: *mut c_void, property_key: *mut c_void) -> *mut c_void;
    fn TISSelectInputSource(input_source: *mut c_void) -> i32;
    fn TISCreateInputSourceList(properties: *mut c_void, include_all: bool) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringGetCStringPtr(string: *mut c_void, encoding: u32) -> *const i8;
    fn CFStringGetCString(theString: *mut c_void, buffer: *mut i8, bufferSize: isize, encoding: u32) -> bool;
    fn CFRelease(cf: *mut c_void);
    fn CFArrayGetCount(array: *mut c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *mut c_void, idx: isize) -> *mut c_void;
}

#[link(name = "System", kind = "dylib")]
extern "C" {
    pub static _dispatch_main_q: c_void;
    pub fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    pub fn dispatch_sync_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

fn cfstring_to_string(cf_str: *mut c_void) -> Option<String> {
    if cf_str.is_null() {
        return None;
    }
    unsafe {
        let c_ptr = CFStringGetCStringPtr(cf_str, 0x08000100); // UTF8
        if !c_ptr.is_null() {
            let cstr = std::ffi::CStr::from_ptr(c_ptr);
            return Some(cstr.to_string_lossy().into_owned());
        } else {
            let mut buf = vec![0u8; 256];
            if CFStringGetCString(cf_str, buf.as_mut_ptr() as *mut i8, buf.len() as isize, 0x08000100) {
                if let Ok(c_str) = std::ffi::CStr::from_ptr(buf.as_ptr() as *const i8).to_str() {
                    return Some(c_str.to_string());
                }
            }
        }
    }
    None
}

fn get_current_input_source_id() -> Option<String> {
    unsafe {
        let id_key = kTISPropertyInputSourceID;
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return None;
        }

        let val = TISGetInputSourceProperty(source, id_key);
        let result = cfstring_to_string(val);
        CFRelease(source);
        
        result
    }
}

pub fn is_ime_on(mode: ImeMode) -> bool {
    matches!(mode, ImeMode::Ignore | ImeMode::ForceAlpha) || is_japanese_input_active(mode)
}

pub fn get_ime_open_status() -> anyhow::Result<bool> {
    Ok(is_japanese_input_active(ImeMode::Auto))
}

static IS_JAPANESE_INPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static LAST_JAPANESE_INPUT_SOURCE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

extern "C" fn update_ime_cache_work(_ctx: *mut c_void) {
    if let Some(id) = get_current_input_source_id() {
        let mut is_jp = id.contains("Japanese") || id.contains("Kotoeri") || id.contains("ATOK");
        if is_jp && (id.contains("Roman") || id.contains("Alphanumeric")) {
            is_jp = false;
        }
        if is_jp {
            if let Ok(mut lock) = LAST_JAPANESE_INPUT_SOURCE.lock() {
                *lock = Some(id.clone());
            }
        }
        IS_JAPANESE_INPUT.store(is_jp, std::sync::atomic::Ordering::Relaxed);
    }
}

pub fn update_ime_cache_on_main_thread() {
    unsafe {
        dispatch_sync_f(
            std::ptr::addr_of!(_dispatch_main_q) as *const c_void,
            std::ptr::null_mut(),
            update_ime_cache_work,
        );
    }
}

pub fn is_japanese_input_active(mode: ImeMode) -> bool {
    if matches!(mode, ImeMode::Ignore) {
        return true;
    }
    if matches!(mode, ImeMode::ForceAlpha) {
        return false;
    }

    IS_JAPANESE_INPUT.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn is_kana_input_active(_mode: ImeMode) -> bool {
    false
}

pub fn is_composition_active() -> bool {
    false
}

extern "C" fn set_force_ime_work(ctx: *mut c_void) {
    let open = ctx as usize != 0;
    // Utilize native macOS Kana / Eisu key events to force robust IME state toggles,
    // solving Google Japanese Input's internal Romaji/Hiragana state tracking bug.
    // 0x79 is Kana (Mac 0x68), 0x7B is Eisu (Mac 0x66).
    if open {
        let _ = crate::keyboard_hook::inject_scancode(0x79, false, false);
        let _ = crate::keyboard_hook::inject_scancode(0x79, false, true);
    } else {
        let _ = crate::keyboard_hook::inject_scancode(0x7B, false, false);
        let _ = crate::keyboard_hook::inject_scancode(0x7B, false, true);
    }
}

pub fn set_force_ime_status(open: bool) {
    unsafe {
        dispatch_async_f(
            std::ptr::addr_of!(_dispatch_main_q) as *const c_void,
            open as usize as *mut c_void,
            set_force_ime_work,
        );
    }
}

