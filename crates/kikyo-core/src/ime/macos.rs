use crate::chord_engine::ImeMode;
use std::ffi::c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    pub static kTISPropertyInputSourceID: *mut c_void;
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

pub fn update_ime_cache_on_main_thread() {
    if let Some(id) = get_current_input_source_id() {
        let is_jp = id.contains("Japanese") || id.contains("Kotoeri") || id.contains("ATOK");
        IS_JAPANESE_INPUT.store(is_jp, std::sync::atomic::Ordering::Relaxed);
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

pub fn set_force_ime_status(open: bool) {
    unsafe {
        let id_key = kTISPropertyInputSourceID;
        if id_key.is_null() { return; }
        
        let target_substring = if open {
            "Japanese"
        } else {
            "com.apple.keylayout.ABC"
        };

        let list = TISCreateInputSourceList(std::ptr::null_mut(), false);
        if list.is_null() { return; }
        
        let count = CFArrayGetCount(list);

        for i in 0..count {
            let source = CFArrayGetValueAtIndex(list, i);
            let val = TISGetInputSourceProperty(source, id_key);
            if let Some(id) = cfstring_to_string(val) {
                if id.contains(target_substring) {
                    TISSelectInputSource(source);
                    break;
                }
            }
        }
        
        CFRelease(list);
    }
}

