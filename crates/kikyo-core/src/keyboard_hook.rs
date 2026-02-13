use crate::engine::ENGINE;
use crate::types::InputEvent;
use crate::types::KeyAction;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, LASTINPUTINFO,
    VIRTUAL_KEY, VK_CONTROL,
    // VK_ESCAPE, // Emergency stop is currently disabled.
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG,
    LLKHF_ALTDOWN, WH_KEYBOARD_LL, WM_APP, WM_KEYUP, WM_SYSKEYUP,
};
/// Magic number to identify our own injected events.
const INJECTED_EXTRA_INFO: usize = 0xFFC3C3C3;

static HOOK_HANDLE: Mutex<Option<HHOOK>> = Mutex::new(None);
static HOOK_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static HOOK_WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static LAST_HOOK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_REINSTALL_MS: AtomicU64 = AtomicU64::new(0);
static ALT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static START_INSTANT: OnceLock<std::time::Instant> = OnceLock::new();

const HOOK_QUEUE_SIZE: usize = 1024;
const WATCHDOG_INTERVAL_MS: u64 = 1000;
const HOOK_STALL_MS: u64 = 5000;
const INPUT_RECENT_MS: u64 = 2000;
const REINSTALL_BACKOFF_MS: u64 = 10000;
const WM_HOOK_REINSTALL: u32 = WM_APP + 0x4B10;

#[derive(Clone, Copy, Debug)]
struct HookEvent {
    sc: u16,
    ext: bool,
    up: bool,
    shift: bool,
    vk: u32,
}

lazy_static::lazy_static! {
    static ref HOOK_QUEUE: (Sender<HookEvent>, Receiver<HookEvent>) =
        crossbeam_channel::bounded(HOOK_QUEUE_SIZE);
}

fn monotonic_ms() -> u64 {
    let start = START_INSTANT.get_or_init(std::time::Instant::now);
    start.elapsed().as_millis() as u64
}

fn ensure_worker_thread() {
    if HOOK_WORKER_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    let rx = HOOK_QUEUE.1.clone();
    thread::Builder::new()
        .name("kikyo-hook-worker".to_string())
        .spawn(move || hook_worker(rx))
        .expect("Failed to spawn hook worker thread");
}

fn ensure_watchdog_thread() {
    if HOOK_WATCHDOG_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    thread::Builder::new()
        .name("kikyo-hook-watchdog".to_string())
        .spawn(watchdog_loop)
        .expect("Failed to spawn hook watchdog thread");
}

pub fn refresh_runtime_flags_from_engine() {
    let engine = ENGINE.lock();
    ALT_NEEDS_HANDLING.store(engine.needs_alt_handling(), Ordering::Relaxed);
}

/// Starts the keyboard hook.
/// This must be called from a thread that pumps messages (GetMessage/PeekMessage).
pub fn install_hook() -> anyhow::Result<()> {
    ensure_worker_thread();
    ensure_watchdog_thread();
    refresh_runtime_flags_from_engine();

    info!("Installing keyboard hook...");

    // Avoid leaking an old handle if this is a reinstall request.
    uninstall_hook();

    // Low-level hooks require hMod to be NULL if threadId is 0.
    // However, Rust/Windows crates handle Option<HINSTANCE> -> 0.
    let hook_id =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), HINSTANCE::default(), 0) }?;

    if hook_id.is_invalid() {
        return Err(anyhow::anyhow!("Failed to install hook"));
    }

    *HOOK_HANDLE.lock().unwrap() = Some(hook_id);
    info!(
        "Keyboard hook installed successfully. Handle: {:?}",
        hook_id
    );
    Ok(())
}

pub fn uninstall_hook() {
    let mut handle = HOOK_HANDLE.lock().unwrap();
    if let Some(h) = *handle {
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        };
        info!("Keyboard hook uninstalled.");
    }
    *handle = None;
}

/// Runs a blocking message loop.
/// This is a convenience helper for creating a hook thread.
pub fn run_event_loop() {
    info!("Starting message loop...");
    HOOK_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let mut msg = MSG::default();
    unsafe {
        // Force message queue creation
        let _ = PeekMessageW(
            &mut msg,
            None,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::PEEK_MESSAGE_REMOVE_TYPE(0),
        );

        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOOK_REINSTALL {
                reinstall_hook();
                continue;
            }

            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    info!("Message loop exited.");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let result = catch_unwind(AssertUnwindSafe(|| {
        LAST_HOOK_MS.store(monotonic_ms(), Ordering::Relaxed);

        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

        // Check self-injection guard
        if kbd.dwExtraInfo == INJECTED_EXTRA_INFO {
            // Pass through our own events
            return CallNextHookEx(None, code, wparam, lparam);
        }

        // Log visible events
        let msg = wparam.0 as u32;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        // Emergency stop is intentionally disabled for now.
        // To restore Ctrl+Alt+Esc shutdown behavior, uncomment this block.
        /*
        if kbd.vkCode == VK_ESCAPE.0 as u32 {
            let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0;
            let alt = GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0;
            if ctrl && alt {
                error!("EMERGENCY STOP TRIGGERED (Ctrl+Alt+Esc). Exiting process.");
                std::process::exit(1);
            }
        }
        */

        // Check for modifiers to disable hook
        let is_shift_vk = kbd.vkCode == VK_SHIFT.0 as u32
            || kbd.vkCode == VK_LSHIFT.0 as u32
            || kbd.vkCode == VK_RSHIFT.0 as u32;
        let is_ctrl_vk = kbd.vkCode == VK_CONTROL.0 as u32
            || kbd.vkCode == VK_LCONTROL.0 as u32
            || kbd.vkCode == VK_RCONTROL.0 as u32;
        let is_alt_vk = kbd.vkCode == VK_MENU.0 as u32
            || kbd.vkCode == VK_LMENU.0 as u32
            || kbd.vkCode == VK_RMENU.0 as u32;
        let is_win_vk = kbd.vkCode == VK_LWIN.0 as u32 || kbd.vkCode == VK_RWIN.0 as u32;

        // Alt may be used as a logical key source via [機能キー] swap.
        // In that case we must feed Alt events into the engine.
        let alt_needs_handling = ALT_NEEDS_HANDLING.load(Ordering::Relaxed);

        // Pass through Modifier key events themselves to ensure OS state is updated
        if is_shift_vk || is_ctrl_vk || is_win_vk || (is_alt_vk && !alt_needs_handling) {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        // Check modifier states only for non-modifier keys that can be handled.
        let ctrl_pressed = GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0;
        let shift_pressed = GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000 != 0;
        let lwin_pressed = GetAsyncKeyState(VK_LWIN.0 as i32) as u16 & 0x8000 != 0;
        let rwin_pressed = GetAsyncKeyState(VK_RWIN.0 as i32) as u16 & 0x8000 != 0;
        let alt_pressed = is_alt_vk || (kbd.flags.0 & LLKHF_ALTDOWN.0) != 0;

        if ctrl_pressed || lwin_pressed || rwin_pressed || (alt_pressed && !alt_needs_handling) {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let ext = (kbd.flags.0 & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0) != 0;

        let event = HookEvent {
            sc: kbd.scanCode as u16,
            ext,
            up,
            shift: shift_pressed,
            vk: kbd.vkCode,
        };

        match HOOK_QUEUE.0.try_send(event) {
            Ok(()) => LRESULT(1), // Block original; worker will decide inject/pass.
            Err(TrySendError::Full(_)) => CallNextHookEx(None, code, wparam, lparam),
            Err(TrySendError::Disconnected(_)) => CallNextHookEx(None, code, wparam, lparam),
        }
    }));

    match result {
        Ok(res) => res,
        Err(_) => {
            error!("Panic in hook_proc; falling back to CallNextHookEx");
            CallNextHookEx(None, code, wparam, lparam)
        }
    }
}

fn hook_worker(rx: Receiver<HookEvent>) {
    for event in rx.iter() {
        let result = catch_unwind(AssertUnwindSafe(|| process_event(event)));
        if result.is_err() {
            error!("Panic in hook worker; dropping event");
        }
    }
}

fn process_event(event: HookEvent) {
    let action = {
        let mut engine = ENGINE.lock();
        ALT_NEEDS_HANDLING.store(engine.needs_alt_handling(), Ordering::Relaxed);

        if let Some(vk) = suspend_key_vk(engine.get_suspend_key()) {
            if event.vk == vk && !event.up {
                let current = engine.is_enabled();
                engine.set_enabled(!current);
                info!(
                    "Suspend Key triggered. Toggled enabled state to: {}",
                    !current
                );
            }
        }

        engine.process_key(event.sc, event.ext, event.up, event.shift)
    };

    match action {
        KeyAction::Pass => {
            let _ = inject_scancode(event.sc, event.ext, event.up);
        }
        KeyAction::Block => {}
        KeyAction::Inject(events) => {
            for ev in events {
                match ev {
                    InputEvent::Scancode(sc, ext, up) => {
                        let _ = inject_scancode(sc, ext, up);
                    }
                    InputEvent::Unicode(c, up) => {
                        let _ = inject_unicode(c, up);
                    }
                    InputEvent::ImeControl(open) => {
                        // IME Control is a state change, not a key press/release pair.
                        // Ideally we should execute it only once.
                        // Since engine emits it as a single event, we just execute it.
                        crate::ime::set_force_ime_status(open);
                    }
                }
            }
        }
    }
}

fn suspend_key_vk(suspend_key: crate::chord_engine::SuspendKey) -> Option<u32> {
    match suspend_key {
        crate::chord_engine::SuspendKey::None => None,
        crate::chord_engine::SuspendKey::ScrollLock => Some(0x91), // VK_SCROLL
        crate::chord_engine::SuspendKey::Pause => Some(0x13),      // VK_PAUSE
        crate::chord_engine::SuspendKey::Insert => Some(0x2D),     // VK_INSERT
        crate::chord_engine::SuspendKey::RightShift => Some(0xA1), // VK_RSHIFT
        crate::chord_engine::SuspendKey::RightControl => Some(0xA3), // VK_RCONTROL
        crate::chord_engine::SuspendKey::RightAlt => Some(0xA5),   // VK_RMENU
    }
}

fn reinstall_hook() {
    if let Err(e) = install_hook() {
        error!("Failed to reinstall hook: {}", e);
    } else {
        info!("Keyboard hook reinstalled by watchdog.");
    }
}

fn request_reinstall() -> bool {
    let thread_id = HOOK_THREAD_ID.load(Ordering::Acquire);
    if thread_id == 0 {
        return false;
    }

    unsafe { PostThreadMessageW(thread_id, WM_HOOK_REINSTALL, WPARAM(0), LPARAM(0)).is_ok() }
}

fn last_input_age_ms() -> Option<u64> {
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    unsafe {
        if !GetLastInputInfo(&mut lii).as_bool() {
            return None;
        }
    }

    let now = unsafe { GetTickCount() };
    let age_ms = now.wrapping_sub(lii.dwTime) as u64;
    Some(age_ms)
}

fn watchdog_loop() {
    loop {
        thread::sleep(Duration::from_millis(WATCHDOG_INTERVAL_MS));

        let handle_present = HOOK_HANDLE.lock().unwrap().is_some();
        if !handle_present {
            continue;
        }

        let last_hook = LAST_HOOK_MS.load(Ordering::Relaxed);
        if last_hook == 0 {
            continue;
        }

        let now = monotonic_ms();
        let since_hook = now.saturating_sub(last_hook);
        if since_hook < HOOK_STALL_MS {
            continue;
        }

        let input_age = match last_input_age_ms() {
            Some(age) => age,
            None => {
                warn!("GetLastInputInfo failed; skipping watchdog cycle");
                continue;
            }
        };

        if input_age > INPUT_RECENT_MS {
            continue;
        }

        let last_reinstall = LAST_REINSTALL_MS.load(Ordering::Relaxed);
        if now.saturating_sub(last_reinstall) < REINSTALL_BACKOFF_MS {
            continue;
        }

        if request_reinstall() {
            LAST_REINSTALL_MS.store(now, Ordering::Relaxed);
            warn!(
                "Hook watchdog requested reinstall: last_hook={}ms ago, last_input={}ms ago",
                since_hook, input_age
            );
        }
    }
}

/// Inject a key event (scancode).
/// up: true for KeyUp, false for KeyDown.
pub fn inject_scancode(sc: u16, ext: bool, up: bool) -> anyhow::Result<()> {
    let mut flags = KEYEVENTF_SCANCODE;
    if ext {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }

    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: sc,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_EXTRA_INFO,
            },
        },
    };

    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
    Ok(())
}

/// Inject a unicode character.
pub fn inject_unicode(c: char, up: bool) -> anyhow::Result<()> {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }

    // Convert char to utf-16
    let mut buf = [0; 2];
    let encoded = c.encode_utf16(&mut buf);

    for code_unit in encoded {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: *code_unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: INJECTED_EXTRA_INFO,
                },
            },
        };

        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }
    Ok(())
}
