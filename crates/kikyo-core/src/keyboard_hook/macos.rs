use crate::engine::ENGINE;
use crate::types::{InputEvent, KeyAction};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info};

#[repr(C)] pub struct __CGEvent(c_void);
pub type CGEventRef = *mut __CGEvent;

#[repr(C)] pub struct __CGEventSource(c_void);
pub type CGEventSourceRef = *mut __CGEventSource;

#[repr(C)] pub struct __CFMachPort(c_void);
pub type CFMachPortRef = *mut __CFMachPort;

#[repr(C)] pub struct __CFRunLoopSource(c_void);
pub type CFRunLoopSourceRef = *mut __CFRunLoopSource;

pub type CGEventTapProxy = *mut c_void;
pub type CGEventType = u32;
pub type CGEventField = u32;
pub type CGEventFlags = u64;

pub type CGEventTapCallBack = extern "C" fn(
    proxy: CGEventTapProxy,
    type_: CGEventType,
    event: CGEventRef,
    userInfo: *mut c_void,
) -> CGEventRef;

pub const kCGSessionEventTap: u32 = 1;
pub const kCGHIDEventTap: u32 = 0;
pub const kCGHeadInsertEventTap: u32 = 0;
pub const kCGEventTapOptionDefault: u32 = 0;

pub const kCGEventKeyDown: u32 = 10;
pub const kCGEventKeyUp: u32 = 11;
pub const kCGEventFlagsChanged: u32 = 12;

pub const kCGEventTapDisabledByTimeout: u32 = 0xFFFF;
pub const kCGEventTapDisabledByUserInput: u32 = 0xFFFE;

pub const kCGKeyboardEventKeycode: u32 = 9;
pub const kCGEventFlagMaskAlphaShift: u64 = 0x00010000;
pub const kCGEventFlagMaskShift: u64 = 0x00020000;
pub const kCGEventFlagMaskControl: u64 = 0x00040000;
pub const kCGEventFlagMaskAlternate: u64 = 0x00080000;
pub const kCGEventFlagMaskCommand: u64 = 0x00100000;

const INJECTED_EXTRA_INFO: u32 = 0xDEADBEEF;

pub const kCGEventSourceStateHIDSystemState: i32 = 1;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        eventsOfInterest: u64,
        callback: CGEventTapCallBack,
        userInfo: *mut c_void,
    ) -> CFMachPortRef;

    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);

    pub fn CGEventCreateKeyboardEvent(
        source: *mut c_void,
        virtualKey: u16,
        keyDown: bool,
    ) -> CGEventRef;
    
    pub fn CGEventKeyboardSetUnicodeString(
        event: CGEventRef,
        stringLength: usize,
        unicodeString: *const u16,
    );

    pub fn CGEventSourceKeyState(stateID: i32, key: u16) -> bool;

    pub fn CGEventPost(tap: u32, event: CGEventRef);
    pub fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: CGEventField) -> i64;
    pub fn CGEventSetIntegerValueField(event: CGEventRef, field: CGEventField, value: i64);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub fn CFMachPortCreateRunLoopSource(
        allocator: *mut c_void,
        port: CFMachPortRef,
        order: c_int,
    ) -> CFRunLoopSourceRef;

    pub fn CFRunLoopAddSource(rl: *mut c_void, source: CFRunLoopSourceRef, mode: *mut c_void);
    pub fn CFRunLoopGetCurrent() -> *mut c_void;
    pub fn CFRunLoopRun();
    pub fn CFRunLoopStop(rl: *mut c_void);
    pub fn CFRetain(cf: *mut c_void);
    pub fn CFRelease(cf: *mut c_void);
    
    // In Rust we can just use the pointer from dlsym or we can link it
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXIsProcessTrusted() -> bool;
    pub fn AXIsProcessTrustedWithOptions(options: *mut c_void) -> bool;
}

#[derive(Clone, Copy, Debug)]
struct HookEvent {
    sc: u16,
    ext: bool,
    up: bool,
    shift: bool,
    vk: u32,
    is_suspend: bool,
    is_settings: bool,
    is_switch_layout: bool,
    mac_kc: u16,
}

// Wrapper for c_void pointer to implement Send/Sync
#[derive(Clone, Copy)]
struct RunLoopPtr(*mut c_void);
unsafe impl Send for RunLoopPtr {}
unsafe impl Sync for RunLoopPtr {}

#[derive(Clone, Copy)]
struct TapPortPtr(CFMachPortRef);
unsafe impl Send for TapPortPtr {}
unsafe impl Sync for TapPortPtr {}

lazy_static::lazy_static! {
    static ref HOOK_QUEUE: (Sender<HookEvent>, Receiver<HookEvent>) = crossbeam_channel::bounded(1024);
    static ref RUN_LOOP_PTR: Mutex<Option<RunLoopPtr>> = Mutex::new(None);
    static ref TAP_PORT: Mutex<Option<TapPortPtr>> = Mutex::new(None);
}

static ENGINE_ENABLED: AtomicBool = AtomicBool::new(true);

static ALT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_SHIFT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_SHIFT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_CTRL_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_CTRL_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_WIN_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_WIN_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);

static SUSPEND_SHORTCUT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SETTINGS_SHORTCUT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SWITCH_LAYOUT_SHORTCUT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn encode_shortcut(s: &Option<crate::types::ShortcutKey>) -> u64 {
    match s {
        Some(sc) => {
            let mut val = sc.vkey as u64;
            if sc.ctrl {
                val |= 1 << 32;
            }
            if sc.shift {
                val |= 1 << 33;
            }
            if sc.alt {
                val |= 1 << 34;
            }
            if sc.win {
                val |= 1 << 35;
            }
            val | (1 << 63)
        }
        None => 0,
    }
}

pub fn refresh_runtime_flags_from_engine() {
    let engine = ENGINE.lock();
    ENGINE_ENABLED.store(engine.is_enabled(), Ordering::Relaxed);
    SUSPEND_SHORTCUT.store(
        encode_shortcut(&engine.get_suspend_shortcut()),
        Ordering::Relaxed,
    );
    SETTINGS_SHORTCUT.store(
        encode_shortcut(&engine.get_settings_shortcut()),
        Ordering::Relaxed,
    );
    SWITCH_LAYOUT_SHORTCUT.store(
        encode_shortcut(&engine.get_switch_layout_shortcut()),
        Ordering::Relaxed,
    );

    let enabled = engine.is_enabled();
    if !enabled {
        ALT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_SHIFT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_SHIFT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_CTRL_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_CTRL_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_WIN_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_WIN_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        return;
    }

    ALT_NEEDS_HANDLING.store(engine.needs_alt_handling(), Ordering::Relaxed);
    LEFT_SHIFT_NEEDS_HANDLING.store(engine.needs_left_shift_handling(), Ordering::Relaxed);
    RIGHT_SHIFT_NEEDS_HANDLING.store(engine.needs_right_shift_handling(), Ordering::Relaxed);
    LEFT_CTRL_NEEDS_HANDLING.store(engine.needs_left_ctrl_handling(), Ordering::Relaxed);
    RIGHT_CTRL_NEEDS_HANDLING.store(engine.needs_right_ctrl_handling(), Ordering::Relaxed);
    LEFT_WIN_NEEDS_HANDLING.store(engine.needs_left_win_handling(), Ordering::Relaxed);
    RIGHT_WIN_NEEDS_HANDLING.store(engine.needs_right_win_handling(), Ordering::Relaxed);
}

fn mac_to_win(kc: u16) -> (u16, u32, bool) {
    // Basic mapping for JIS keyboard
    match kc {
        0x00 => (0x1E, 0x41, false), // A
        0x0B => (0x30, 0x42, false), // B
        0x08 => (0x2E, 0x43, false), // C
        0x02 => (0x20, 0x44, false), // D
        0x0E => (0x12, 0x45, false), // E
        0x03 => (0x21, 0x46, false), // F
        0x05 => (0x22, 0x47, false), // G
        0x04 => (0x23, 0x48, false), // H
        0x22 => (0x17, 0x49, false), // I
        0x26 => (0x24, 0x4A, false), // J
        0x28 => (0x25, 0x4B, false), // K
        0x25 => (0x26, 0x4C, false), // L
        0x2E => (0x32, 0x4D, false), // M
        0x2D => (0x31, 0x4E, false), // N
        0x1F => (0x18, 0x4F, false), // O
        0x23 => (0x19, 0x50, false), // P
        0x0C => (0x10, 0x51, false), // Q
        0x0F => (0x13, 0x52, false), // R
        0x01 => (0x1F, 0x53, false), // S
        0x11 => (0x14, 0x54, false), // T
        0x20 => (0x16, 0x55, false), // U
        0x09 => (0x2F, 0x56, false), // V
        0x0D => (0x11, 0x57, false), // W
        0x07 => (0x2D, 0x58, false), // X
        0x10 => (0x15, 0x59, false), // Y
        0x06 => (0x2C, 0x5A, false), // Z
        
        0x12 => (0x02, 0x31, false), // 1
        0x13 => (0x03, 0x32, false), // 2
        0x14 => (0x04, 0x33, false), // 3
        0x15 => (0x05, 0x34, false), // 4
        0x17 => (0x06, 0x35, false), // 5
        0x16 => (0x07, 0x36, false), // 6
        0x1A => (0x08, 0x37, false), // 7
        0x1C => (0x09, 0x38, false), // 8
        0x19 => (0x0A, 0x39, false), // 9
        0x1D => (0x0B, 0x30, false), // 0
        
        0x1B => (0x0C, 0xBD, false), // -
        0x18 => (0x0D, 0xBB, false), // ^
        0x5D => (0x7D, 0xDC, false), // Yen
        
        0x21 => (0x1A, 0xC0, false), // @
        0x1E => (0x1B, 0xDB, false), // [
        
        0x29 => (0x27, 0xBA, false), // ;
        0x27 => (0x28, 0xDE, false), // :
        0x2A => (0x2B, 0xDD, false), // ]
        
        0x2B => (0x33, 0xBC, false), // ,
        0x2F => (0x34, 0xBE, false), // .
        0x2C => (0x35, 0xBF, false), // /
        0x5E => (0x73, 0xE2, false), // Underscore
        
        0x38 => (0x2A, 0x10, false), // LShift
        0x3C => (0x36, 0x10, true),  // RShift
        0x3B => (0x1D, 0x11, false), // LCtrl
        0x3E => (0x1D, 0x11, true),  // RCtrl
        0x3A => (0x38, 0x12, false), // LAlt
        0x3D => (0x38, 0x12, true),  // RAlt
        0x37 => (0x5B, 0x5B, true),  // LCmd -> LWin
        0x36 => (0x5C, 0x5C, true),  // RCmd -> RWin
        
        0x31 => (0x39, 0x20, false), // Space
        0x24 => (0x1C, 0x0D, false), // Return
        0x33 => (0x0E, 0x08, false), // Backspace
        0x30 => (0x0F, 0x09, false), // Tab
        0x35 => (0x01, 0x1B, false), // Esc
        
        0x66 => (0x7B, 0xEB, false), // Eisu (Muhenkan)
        0x68 => (0x79, 0x1C, false), // Kana (Henkan)
        
        0x7E => (0x48, 0x26, true), // Up
        0x7D => (0x50, 0x28, true), // Down
        0x7B => (0x4B, 0x25, true), // Left
        0x7C => (0x4D, 0x27, true), // Right
        _ => (0, 0, false),
    }
}

fn win_to_mac(sc: u16, ext: bool) -> u16 {
    // Reverse mapping for injection
    match (sc, ext) {
        (0x1E, _) => 0x00, (0x30, _) => 0x0B, (0x2E, _) => 0x08, (0x20, _) => 0x02,
        (0x12, _) => 0x0E, (0x21, _) => 0x03, (0x22, _) => 0x05, (0x23, _) => 0x04,
        (0x17, _) => 0x22, (0x24, _) => 0x26, (0x25, _) => 0x28, (0x26, _) => 0x25,
        (0x32, _) => 0x2E, (0x31, _) => 0x2D, (0x18, _) => 0x1F, (0x19, _) => 0x23,
        (0x10, _) => 0x0C, (0x13, _) => 0x0F, (0x1F, _) => 0x01, (0x14, _) => 0x11,
        (0x16, _) => 0x20, (0x2F, _) => 0x09, (0x11, _) => 0x0D, (0x2D, _) => 0x07,
        (0x15, _) => 0x10, (0x2C, _) => 0x06,
        
        (0x02, _) => 0x12, (0x03, _) => 0x13, (0x04, _) => 0x14, (0x05, _) => 0x15,
        (0x06, _) => 0x17, (0x07, _) => 0x16, (0x08, _) => 0x1A, (0x09, _) => 0x1C,
        (0x0A, _) => 0x19, (0x0B, _) => 0x1D,
        
        (0x0C, _) => 0x1B, (0x0D, _) => 0x18, (0x7D, _) => 0x5D, (0x1A, _) => 0x21,
        (0x1B, _) => 0x1E, (0x27, _) => 0x29, (0x28, _) => 0x27, (0x2B, _) => 0x2A,
        (0x33, _) => 0x2B, (0x34, _) => 0x2F, (0x35, _) => 0x2C, (0x73, _) => 0x5E,
        
        (0x2A, false) => 0x38, (0x36, false) => 0x3C,
        (0x1D, false) => 0x3B, (0x1D, true) => 0x3E,
        (0x38, false) => 0x3A, (0x38, true) => 0x3D,
        (0x5B, true)  => 0x37, (0x5C, true) => 0x36,
        
        (0x39, _) => 0x31, (0x1C, false) => 0x24, (0x0E, _) => 0x33,
        (0x0F, _) => 0x30, (0x01, _) => 0x35,
        
        (0x7B, _) => 0x66, (0x79, _) => 0x68,
        
        (0x48, true) => 0x7E, (0x50, true) => 0x7D, (0x4B, true) => 0x7B, (0x4D, true) => 0x7C,
        _ => 0xFFFF,
    }
}

extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    type_: CGEventType,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if type_ == kCGEventTapDisabledByTimeout || type_ == kCGEventTapDisabledByUserInput {
            if let Some(tap_ptr) = *TAP_PORT.lock() {
                unsafe { CGEventTapEnable(tap_ptr.0, true) };
            }
            return event;
        }

        if type_ != kCGEventKeyDown && type_ != kCGEventKeyUp && type_ != kCGEventFlagsChanged {
            return event;
        }

        let magic = unsafe { CGEventGetIntegerValueField(event, 42) }; // kCGEventSourceUserData
        if magic == INJECTED_EXTRA_INFO as i64 {
            return event;
        }

        let kc = unsafe { CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) } as u16;
        let flags = unsafe { CGEventGetFlags(event) };
        let shift = (flags & kCGEventFlagMaskShift) != 0;
        
        let (sc, vk, ext) = mac_to_win(kc);
        
        if sc == 0 {
            return event;
        }

        let mut is_shift_kc = false;
        let mut is_ctrl_kc = false;
        let mut is_alt_kc = false;
        let mut is_win_kc = false;
        let up = match type_ {
            kCGEventKeyUp => true,
            kCGEventKeyDown => {
                crate::ime::update_ime_cache_on_main_thread();
                false
            },
            kCGEventFlagsChanged => {
                crate::ime::update_ime_cache_on_main_thread();
                // If the key is shift, and shift flag is not present, it's an UP event
                if kc == 0x38 || kc == 0x3C {
                    is_shift_kc = true;
                    !shift
                } else if kc == 0x3B || kc == 0x3E { // Control
                    is_ctrl_kc = true;
                    (flags & kCGEventFlagMaskControl) == 0
                } else if kc == 0x3A || kc == 0x3D { // Option
                    is_alt_kc = true;
                    (flags & kCGEventFlagMaskAlternate) == 0
                } else if kc == 0x37 || kc == 0x36 { // Command
                    is_win_kc = true;
                    (flags & kCGEventFlagMaskCommand) == 0
                } else {
                    false // Assume down for other modifier changes (cmd etc)
                }
            }
            _ => false,
        };

        let alt_needs_handling = ALT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_shift_needs_handling = LEFT_SHIFT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_shift_needs_handling = RIGHT_SHIFT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_ctrl_needs_handling = LEFT_CTRL_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_ctrl_needs_handling = RIGHT_CTRL_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_win_needs_handling = LEFT_WIN_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_win_needs_handling = RIGHT_WIN_NEEDS_HANDLING.load(Ordering::Relaxed);

        let shift_event_needs_handling = if !is_shift_kc {
            false
        } else if kc == 0x38 {
            left_shift_needs_handling
        } else if kc == 0x3C {
            right_shift_needs_handling
        } else {
            left_shift_needs_handling || right_shift_needs_handling
        };

        let ctrl_event_needs_handling = if !is_ctrl_kc {
            false
        } else if kc == 0x3B {
            left_ctrl_needs_handling
        } else if kc == 0x3E {
            right_ctrl_needs_handling
        } else {
            left_ctrl_needs_handling || right_ctrl_needs_handling
        };

        let win_event_needs_handling = if !is_win_kc {
            false
        } else if kc == 0x37 {
            left_win_needs_handling
        } else if kc == 0x36 {
            right_win_needs_handling
        } else {
            left_win_needs_handling || right_win_needs_handling
        };

        // Pass through Modifier key events themselves to ensure OS state is updated
        if (is_shift_kc && !shift_event_needs_handling)
            || (is_ctrl_kc && !ctrl_event_needs_handling)
            || (is_win_kc && !win_event_needs_handling)
            || (is_alt_kc && !alt_needs_handling)
        {
            return event;
        }

        let ctrl_pressed = (flags & kCGEventFlagMaskControl) != 0;
        let alt_pressed = (flags & kCGEventFlagMaskAlternate) != 0;
        let win_pressed = (flags & kCGEventFlagMaskCommand) != 0;

        // Ensure we bypass shortcuts like Cmd+A, if Cmd isn't needed by engine.
        // We evaluate only the combined state of modifiers.
        let combined_ctrl_needs_handling = left_ctrl_needs_handling || right_ctrl_needs_handling;
        let combined_win_needs_handling = left_win_needs_handling || right_win_needs_handling;

        if (ctrl_pressed && !combined_ctrl_needs_handling)
            || (win_pressed && !combined_win_needs_handling)
            || (alt_pressed && !alt_needs_handling)
        {
            // At least one pressed modifier is not needed by our engine,
            // which usually means this is an OS shortcut like Cmd+A.
            // But if it happens to be our explicit shortcut, let it through.
            // We verify shortcuts explicitly next.
        } else {
            // Modifiers either are needed, or are not pressed. We can proceed.
        }

        let sc_mapped = sc;
        
        let current_shortcut = {
            let mut val = vk as u64;
            if (flags & kCGEventFlagMaskControl) != 0 {
                val |= 1 << 32;
            }
            if shift {
                val |= 1 << 33;
            }
            if (flags & kCGEventFlagMaskAlternate) != 0 {
                val |= 1 << 34;
            }
            if (flags & kCGEventFlagMaskCommand) != 0 {
                val |= 1 << 35;
            }
            val | (1 << 63)
        };

        let suspend_sc = SUSPEND_SHORTCUT.load(Ordering::Relaxed);
        let settings_sc = SETTINGS_SHORTCUT.load(Ordering::Relaxed);
        let switch_sc = SWITCH_LAYOUT_SHORTCUT.load(Ordering::Relaxed);

        let is_suspend_key = current_shortcut == suspend_sc && suspend_sc != 0;
        let is_settings_key = current_shortcut == settings_sc && settings_sc != 0;
        let is_switch_layout_key = current_shortcut == switch_sc && switch_sc != 0;
        let is_any_shortcut = is_suspend_key || is_settings_key || is_switch_layout_key;

        // Perform the OS shortcut bypass we evaluated earlier, except if it is our exact shortcut
        if !is_any_shortcut {
            if (ctrl_pressed && !combined_ctrl_needs_handling)
                || (win_pressed && !combined_win_needs_handling)
                || (alt_pressed && !alt_needs_handling)
            {
                return event;
            }
        }

        if !ENGINE_ENABLED.load(Ordering::Relaxed) && !is_any_shortcut {
            return event;
        }
        
        if is_any_shortcut {
            let hook_event = HookEvent {
                sc: sc_mapped,
                ext,
                up,
                shift,
                vk,
                is_suspend: is_suspend_key,
                is_settings: is_settings_key,
                is_switch_layout: is_switch_layout_key,
                mac_kc: kc,
            };
            if let Ok(()) = HOOK_QUEUE.0.try_send(hook_event) {
                return std::ptr::null_mut(); // Block original event
            }
            return event;
        }

        if sc_mapped == 0 {
            return event;
        }

        let hook_event = HookEvent {
            sc: sc_mapped,
            ext,
            up,
            shift,
            vk,
            is_suspend: false,
            is_settings: false,
            is_switch_layout: false,
            mac_kc: kc,
        };

        match HOOK_QUEUE.0.try_send(hook_event) {
            Ok(()) => {
                std::ptr::null_mut() // Block original event
            },
            Err(_) => {
                event // Pass through if queue full
            },
        }
    }));

    match result {
        Ok(res) => res,
        Err(_) => event,
    }
}

pub fn install_hook() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/kikyo_panic.log") {
            let backtrace = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(f, "Panic occurred: {:?}\nBacktrace:\n{}", info, backtrace);
        }
    }));
    refresh_runtime_flags_from_engine();
    
    // Spawn worker thread
    let rx = HOOK_QUEUE.1.clone();
    thread::spawn(move || {
        for ev in rx.iter() {
            let mut engine = ENGINE.lock();
            let mut refresh_flags = false;

            if ev.is_suspend && !ev.up {
                let current = engine.is_enabled();
                engine.set_enabled(!current);
                refresh_flags = true;
            }

            if ev.is_settings && !ev.up {
                engine.trigger_settings_shortcut();
            }

            if ev.is_switch_layout && !ev.up {
                engine.trigger_switch_layout_shortcut();
            }

            let action = if ev.is_suspend || ev.is_settings || ev.is_switch_layout {
                KeyAction::Block
            } else {
                engine.process_key(ev.sc, ev.ext, ev.up, ev.shift)
            };
            drop(engine);
            
            if refresh_flags {
                refresh_runtime_flags_from_engine();
            }
            
            match action {
                KeyAction::Pass => {
                    let _ = inject_mac_keycode(ev.mac_kc, ev.up);
                }
                KeyAction::Block => {}
                KeyAction::Inject(events) => {
                    for i_ev in events {
                        match i_ev {
                            InputEvent::Scancode(sc, ext, up) => {
                                let _ = inject_scancode(sc, ext, up);
                            },
                            InputEvent::Unicode(c, up) => {
                                let _ = inject_unicode(c, up);
                            },
                            InputEvent::Delay(ms) => {
                                thread::sleep(Duration::from_millis(ms));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

pub fn uninstall_hook() {
    let mut ptr_lg = RUN_LOOP_PTR.lock();
    if let Some(rl) = *ptr_lg {
        unsafe { CFRunLoopStop(rl.0) };
        *ptr_lg = None;
    }
}

pub fn run_event_loop() {
    unsafe {
        let events_of_interest = (1u64 << kCGEventKeyDown) | (1u64 << kCGEventKeyUp) | (1u64 << kCGEventFlagsChanged);
        let tap = CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionDefault,
            events_of_interest,
            tap_callback,
            std::ptr::null_mut(),
        );

        if tap.is_null() {
            error!("Failed to create CGEventTap. Ensure Accessibility permissions are granted.");
            return;
        }

        *TAP_PORT.lock() = Some(TapPortPtr(tap));

        let dlsym_handle = libc::dlopen(
            std::ffi::CStr::from_bytes_with_nul(b"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation\0").unwrap().as_ptr(),
            libc::RTLD_LAZY
        );
        let k_cf_rl_modes_ptr = libc::dlsym(dlsym_handle, std::ffi::CStr::from_bytes_with_nul(b"kCFRunLoopCommonModes\0").unwrap().as_ptr());
        let common_modes = *(k_cf_rl_modes_ptr as *mut *mut c_void);

        let run_loop_source = CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0);
        let run_loop = CFRunLoopGetCurrent();
        
        *RUN_LOOP_PTR.lock() = Some(RunLoopPtr(run_loop));
        
        CFRunLoopAddSource(run_loop, run_loop_source, common_modes);
        CGEventTapEnable(tap, true);
        
        info!("MacOS Event Loop started.");
        CFRunLoopRun();
    }
}

pub fn inject_scancode(sc: u16, ext: bool, up: bool) -> anyhow::Result<()> {
    let mac_kc = win_to_mac(sc, ext);
    if mac_kc != 0xFFFF {
        inject_mac_keycode(mac_kc, up)?;
    }
    Ok(())
}

fn inject_mac_keycode(kc: u16, up: bool) -> anyhow::Result<()> {
    unsafe {
        // Fallback: If kc is 0xFFFF passed by error, rewrite it as 0 to be safe
        let safe_kc = if kc == 0xFFFF { 0 } else { kc };
        let event = CGEventCreateKeyboardEvent(std::ptr::null_mut(), safe_kc, !up);
        if !event.is_null() {
            CGEventSetIntegerValueField(event, 42, INJECTED_EXTRA_INFO as i64); // kCGEventSourceUserData
            CGEventPost(kCGSessionEventTap, event);
            CFRelease(event as *mut c_void);
        }
    }
    Ok(())
}

pub fn inject_unicode(c: char, up: bool) -> anyhow::Result<()> {
    unsafe {
        // Apple docs: "To generate a Unicode keystroke, use a virtual key code of 0 (kVK_ANSI_A)"
        let event = CGEventCreateKeyboardEvent(std::ptr::null_mut(), 0, !up);
        if !event.is_null() {
            let mut utf16 = [0u16; 2];
            let string_length = c.encode_utf16(&mut utf16).len();
            CGEventKeyboardSetUnicodeString(event, string_length, utf16.as_ptr());
            CGEventSetIntegerValueField(event, 42, INJECTED_EXTRA_INFO as i64);
            CGEventPost(kCGSessionEventTap, event);
            CFRelease(event as *mut c_void);
        }
    }
    Ok(())
}

pub fn check_accessibility_permission() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn request_accessibility_permission() -> bool {
    unsafe {
        // Prepare dictionary: { kAXTrustedCheckOptionPrompt: kCFBooleanTrue }
        let handle = libc::dlopen(
            std::ffi::CStr::from_bytes_with_nul(b"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices\0").unwrap().as_ptr(),
            libc::RTLD_LAZY
        );
        let prompt_key_ptr = libc::dlsym(handle, std::ffi::CStr::from_bytes_with_nul(b"kAXTrustedCheckOptionPrompt\0").unwrap().as_ptr());
        
        if prompt_key_ptr.is_null() {
            return AXIsProcessTrusted();
        }
        
        let prompt_key = *(prompt_key_ptr as *mut *mut std::ffi::c_void);
        
        let cf_handle = libc::dlopen(
            std::ffi::CStr::from_bytes_with_nul(b"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation\0").unwrap().as_ptr(),
            libc::RTLD_LAZY
        );
        let true_ptr = libc::dlsym(cf_handle, std::ffi::CStr::from_bytes_with_nul(b"kCFBooleanTrue\0").unwrap().as_ptr());
        let true_val = *(true_ptr as *mut *mut std::ffi::c_void);

        type CFDictionaryCreateFn = extern "C" fn(*mut std::ffi::c_void, *const *mut std::ffi::c_void, *const *mut std::ffi::c_void, isize, *const std::ffi::c_void, *const std::ffi::c_void) -> *mut std::ffi::c_void;
        let dict_create_ptr = libc::dlsym(cf_handle, std::ffi::CStr::from_bytes_with_nul(b"CFDictionaryCreate\0").unwrap().as_ptr());
        let dict_create: CFDictionaryCreateFn = std::mem::transmute(dict_create_ptr);

        let keys = [prompt_key];
        let values = [true_val];
        let options = dict_create(std::ptr::null_mut(), keys.as_ptr(), values.as_ptr(), 1, std::ptr::null(), std::ptr::null());

        let res = AXIsProcessTrustedWithOptions(options);
        
        type CFReleaseFn = extern "C" fn(*mut std::ffi::c_void);
        let release_ptr = libc::dlsym(cf_handle, std::ffi::CStr::from_bytes_with_nul(b"CFRelease\0").unwrap().as_ptr());
        let release: CFReleaseFn = std::mem::transmute(release_ptr);
        release(options);
        
        res
    }
}

pub fn is_sc_key_physically_down(key: crate::types::ScKey) -> Option<bool> {
    let mac_kc = win_to_mac(key.sc, key.ext);
    if mac_kc == 0xFFFF {
        return None;
    }
    let is_down = unsafe { CGEventSourceKeyState(kCGEventSourceStateHIDSystemState, mac_kc) };
    Some(is_down)
}

pub fn vk_to_scancode(vk: u16) -> Option<(u16, bool)> {
    // Basic mapping from common JS keyCodes to scancodes.
    match vk {
        65..=90 => {
            let sc = match vk - 65 {
                0 => 0x1E, 1 => 0x30, 2 => 0x2E, 3 => 0x20, 4 => 0x12, 5 => 0x21,
                6 => 0x22, 7 => 0x23, 8 => 0x17, 9 => 0x24, 10 => 0x25, 11 => 0x26,
                12 => 0x32, 13 => 0x31, 14 => 0x18, 15 => 0x19, 16 => 0x10, 17 => 0x13,
                18 => 0x1F, 19 => 0x14, 20 => 0x16, 21 => 0x2F, 22 => 0x11, 23 => 0x2D,
                24 => 0x15, 25 => 0x2C, _ => return None,
            };
            Some((sc, false))
        }
        27 => Some((0x01, false)),
        _ => None,
    }
}
