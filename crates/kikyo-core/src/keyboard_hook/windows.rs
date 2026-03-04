use crate::engine::ENGINE;
use crate::types::InputEvent;
use crate::types::KeyAction;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState,
    GetLastInputInfo,
    MapVirtualKeyW,
    SendInput,
    INPUT,
    INPUT_0,
    INPUT_KEYBOARD,
    KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE,
    KEYEVENTF_UNICODE,
    LASTINPUTINFO,
    MAPVK_VK_TO_VSC_EX,
    VIRTUAL_KEY,
    VK_CONTROL,
    // VK_ESCAPE, // Emergency stop is currently disabled.
    VK_LCONTROL,
    VK_LMENU,
    VK_LSHIFT,
    VK_LWIN,
    VK_MENU,
    VK_RCONTROL,
    VK_RMENU,
    VK_RSHIFT,
    VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_ALTDOWN, LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP,
    WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYUP, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};
/// Magic number to identify our own injected events.
const INJECTED_EXTRA_INFO: usize = 0xFFC3C3C3;

static HOOK_HANDLE: Mutex<Option<HHOOK>> = Mutex::new(None);
static MOUSE_HOOK_HANDLE: Mutex<Option<HHOOK>> = Mutex::new(None);
static HOOK_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static HOOK_WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static LAST_HOOK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_REINSTALL_MS: AtomicU64 = AtomicU64::new(0);
static ALT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_SHIFT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_SHIFT_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY: AtomicBool = AtomicBool::new(false);
static RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY: AtomicBool = AtomicBool::new(false);
static CAPTURED_SHIFT_MUTEX_POISONED_REPORTED: AtomicBool = AtomicBool::new(false);
static LEFT_CTRL_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_CTRL_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static LEFT_WIN_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static RIGHT_WIN_NEEDS_HANDLING: AtomicBool = AtomicBool::new(false);
static ENGINE_ENABLED: AtomicBool = AtomicBool::new(true);
static SUSPEND_SHORTCUT: AtomicU64 = AtomicU64::new(0);
static SETTINGS_SHORTCUT: AtomicU64 = AtomicU64::new(0);
static SWITCH_LAYOUT_SHORTCUT: AtomicU64 = AtomicU64::new(0);
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
    is_suspend: bool,
    is_settings: bool,
    is_switch_layout: bool,
}

fn encode_shortcut(s: &Option<crate::types::ShortcutKey>) -> u64 {
    if let Some(s) = s {
        let mut val = s.vkey as u64;
        if s.ctrl {
            val |= 1 << 32;
        }
        if s.shift {
            val |= 1 << 33;
        }
        if s.alt {
            val |= 1 << 34;
        }
        if s.win {
            val |= 1 << 35;
        }
        val | (1 << 63)
    } else {
        0
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CapturedShiftSideState {
    physical_down: bool,
    os_down_sent: bool,
    used_for_layout_output: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct CapturedShiftState {
    left: CapturedShiftSideState,
    right: CapturedShiftSideState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShiftSide {
    Left,
    Right,
}

lazy_static::lazy_static! {
    static ref HOOK_QUEUE: (Sender<HookEvent>, Receiver<HookEvent>) =
        crossbeam_channel::bounded(HOOK_QUEUE_SIZE);
    static ref CAPTURED_SHIFT_STATE: Mutex<CapturedShiftState> =
        Mutex::new(CapturedShiftState::default());
}

fn monotonic_ms() -> u64 {
    let start = START_INSTANT.get_or_init(std::time::Instant::now);
    start.elapsed().as_millis() as u64
}

fn shift_side_for_event(sc: u16, vk: u32) -> Option<ShiftSide> {
    if vk == VK_LSHIFT.0 as u32 || sc == 0x2A {
        return Some(ShiftSide::Left);
    }
    if vk == VK_RSHIFT.0 as u32 || sc == 0x36 {
        return Some(ShiftSide::Right);
    }
    None
}

fn shift_scancode(side: ShiftSide) -> u16 {
    match side {
        ShiftSide::Left => 0x2A,
        ShiftSide::Right => 0x36,
    }
}

fn captured_shift_side_enabled(side: ShiftSide) -> bool {
    match side {
        ShiftSide::Left => LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.load(Ordering::Relaxed),
        ShiftSide::Right => RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.load(Ordering::Relaxed),
    }
}

fn captured_shift_side_mut(
    state: &mut CapturedShiftState,
    side: ShiftSide,
) -> &mut CapturedShiftSideState {
    match side {
        ShiftSide::Left => &mut state.left,
        ShiftSide::Right => &mut state.right,
    }
}

fn report_captured_shift_mutex_poisoned_once() {
    if !CAPTURED_SHIFT_MUTEX_POISONED_REPORTED.swap(true, Ordering::AcqRel) {
        warn!("Captured Shift mutex was poisoned; continuing with recovered state");
    }
}

fn lock_captured_shift_state() -> MutexGuard<'static, CapturedShiftState> {
    match CAPTURED_SHIFT_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            report_captured_shift_mutex_poisoned_once();
            poisoned.into_inner()
        }
    }
}

fn try_lock_captured_shift_state() -> Option<MutexGuard<'static, CapturedShiftState>> {
    match CAPTURED_SHIFT_STATE.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(poisoned)) => {
            report_captured_shift_mutex_poisoned_once();
            Some(poisoned.into_inner())
        }
    }
}

fn forward_pending_captured_shift_downs_if_needed() -> bool {
    let mut left_inject = false;
    let mut right_inject = false;

    {
        let Some(mut state) = try_lock_captured_shift_state() else {
            // Hook callbacks must avoid blocking; skip this cycle if state lock is contended.
            return false;
        };
        let left_state = &mut state.left;
        if left_state.physical_down && !left_state.os_down_sent {
            left_state.os_down_sent = true;
            left_inject = true;
        }
        let right_state = &mut state.right;
        if right_state.physical_down && !right_state.os_down_sent {
            right_state.os_down_sent = true;
            right_inject = true;
        }
    }

    if left_inject {
        let _ = inject_scancode(shift_scancode(ShiftSide::Left), false, false);
    }
    if right_inject {
        let _ = inject_scancode(shift_scancode(ShiftSide::Right), false, false);
    }

    left_inject || right_inject
}

fn mark_captured_shift_used_for_layout_output() {
    let mut state = lock_captured_shift_state();
    for side in [ShiftSide::Left, ShiftSide::Right] {
        let side_state = captured_shift_side_mut(&mut state, side);
        if side_state.physical_down && !side_state.os_down_sent {
            side_state.used_for_layout_output = true;
        }
    }
}

fn captured_shift_down_snapshot() -> (bool, bool) {
    let Some(state) = try_lock_captured_shift_state() else {
        return (false, false);
    };
    (state.left.physical_down, state.right.physical_down)
}

fn clear_captured_shift_state() {
    let mut state = lock_captured_shift_state();
    *state = CapturedShiftState::default();
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
    let enabled = engine.is_enabled();
    ENGINE_ENABLED.store(enabled, Ordering::Relaxed);
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

    if !enabled {
        ALT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_SHIFT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_SHIFT_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(false, Ordering::Relaxed);
        RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(false, Ordering::Relaxed);
        LEFT_CTRL_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_CTRL_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        LEFT_WIN_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        RIGHT_WIN_NEEDS_HANDLING.store(false, Ordering::Relaxed);
        clear_captured_shift_state();
        return;
    }

    ALT_NEEDS_HANDLING.store(engine.needs_alt_handling(), Ordering::Relaxed);
    LEFT_SHIFT_NEEDS_HANDLING.store(engine.needs_left_shift_handling(), Ordering::Relaxed);
    RIGHT_SHIFT_NEEDS_HANDLING.store(engine.needs_right_shift_handling(), Ordering::Relaxed);
    LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(
        engine.capture_left_shift_for_romaji_pinky_shift(),
        Ordering::Relaxed,
    );
    RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(
        engine.capture_right_shift_for_romaji_pinky_shift(),
        Ordering::Relaxed,
    );
    LEFT_CTRL_NEEDS_HANDLING.store(engine.needs_left_ctrl_handling(), Ordering::Relaxed);
    RIGHT_CTRL_NEEDS_HANDLING.store(engine.needs_right_ctrl_handling(), Ordering::Relaxed);
    LEFT_WIN_NEEDS_HANDLING.store(engine.needs_left_win_handling(), Ordering::Relaxed);
    RIGHT_WIN_NEEDS_HANDLING.store(engine.needs_right_win_handling(), Ordering::Relaxed);
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
    let mouse_hook_id =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), HINSTANCE::default(), 0) }?;

    if hook_id.is_invalid() {
        return Err(anyhow::anyhow!("Failed to install hook"));
    }

    *HOOK_HANDLE.lock().unwrap() = Some(hook_id);
    *MOUSE_HOOK_HANDLE.lock().unwrap() = Some(mouse_hook_id);
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

    let mut mouse_handle = MOUSE_HOOK_HANDLE.lock().unwrap();
    if let Some(h) = *mouse_handle {
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        };
        info!("Mouse hook uninstalled.");
    }
    *mouse_handle = None;
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

        // Ignore injected events to prevent self-recursion loops.
        // - Own SendInput events: identified by dwExtraInfo marker.
        // - Other injected events (including IME/tool generated): LLKHF_INJECTED.
        if kbd.dwExtraInfo == INJECTED_EXTRA_INFO || (kbd.flags.0 & LLKHF_INJECTED.0) != 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        // Log visible events
        let msg = wparam.0 as u32;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        // Some IME paths report scanCode=0. Recover from vkCode so engine mapping still works.
        let mut scan_code = kbd.scanCode as u16;
        if scan_code == 0 {
            let mapped = MapVirtualKeyW(kbd.vkCode, MAPVK_VK_TO_VSC_EX);
            if mapped != 0 {
                scan_code = (mapped & 0x00FF) as u16;
            } else {
                // Keep original behavior for unmappable keys; do not block these events.
                return CallNextHookEx(None, code, wparam, lparam);
            }
        }

        let engine_enabled = ENGINE_ENABLED.load(Ordering::Relaxed);

        let lctrl_pressed = GetAsyncKeyState(VK_LCONTROL.0 as i32) as u16 & 0x8000 != 0;
        let rctrl_pressed = GetAsyncKeyState(VK_RCONTROL.0 as i32) as u16 & 0x8000 != 0;
        let lshift_pressed = GetAsyncKeyState(VK_LSHIFT.0 as i32) as u16 & 0x8000 != 0;
        let rshift_pressed = GetAsyncKeyState(VK_RSHIFT.0 as i32) as u16 & 0x8000 != 0;
        let lalt_pressed = GetAsyncKeyState(VK_LMENU.0 as i32) as u16 & 0x8000 != 0;
        let ralt_pressed = GetAsyncKeyState(VK_RMENU.0 as i32) as u16 & 0x8000 != 0;
        let lwin_pressed = GetAsyncKeyState(VK_LWIN.0 as i32) as u16 & 0x8000 != 0;
        let rwin_pressed = GetAsyncKeyState(VK_RWIN.0 as i32) as u16 & 0x8000 != 0;

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

        let alt_pressed =
            is_alt_vk || lalt_pressed || ralt_pressed || (kbd.flags.0 & LLKHF_ALTDOWN.0) != 0;
        let real_ctrl_pressed = lctrl_pressed || rctrl_pressed;
        let real_shift_pressed = lshift_pressed || rshift_pressed;
        let real_win_pressed = lwin_pressed || rwin_pressed;

        let current_shortcut = {
            let mut val = kbd.vkCode as u64;
            if real_ctrl_pressed {
                val |= 1 << 32;
            }
            if real_shift_pressed {
                val |= 1 << 33;
            }
            if alt_pressed {
                val |= 1 << 34;
            }
            if real_win_pressed {
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

        let pass_through = || -> LRESULT {
            if !is_shift_vk {
                let _ = forward_pending_captured_shift_downs_if_needed();
            } else {
                if let Some(side) = shift_side_for_event(scan_code, kbd.vkCode) {
                    if captured_shift_side_enabled(side) {
                        if let Some(mut captured) = try_lock_captured_shift_state() {
                            let side_state = captured_shift_side_mut(&mut captured, side);
                            if !up {
                                side_state.os_down_sent = true;
                            }
                        }
                    }
                }
            }
            CallNextHookEx(None, code, wparam, lparam)
        };

        // Fully bypass the hook while disabled so IME/OS receive original key events.
        // Keep only the suspend key routed so users can re-enable from keyboard.
        if !engine_enabled && !is_any_shortcut {
            return pass_through();
        }

        // Alt may be used as a logical key source via [機能キー] swap.
        // In that case we must feed Alt events into the engine.
        let alt_needs_handling = ALT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_shift_needs_handling = LEFT_SHIFT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_shift_needs_handling = RIGHT_SHIFT_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_shift_capture_for_romaji =
            LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.load(Ordering::Relaxed);
        let right_shift_capture_for_romaji =
            RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.load(Ordering::Relaxed);
        let left_ctrl_needs_handling = LEFT_CTRL_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_ctrl_needs_handling = RIGHT_CTRL_NEEDS_HANDLING.load(Ordering::Relaxed);
        let left_win_needs_handling = LEFT_WIN_NEEDS_HANDLING.load(Ordering::Relaxed);
        let right_win_needs_handling = RIGHT_WIN_NEEDS_HANDLING.load(Ordering::Relaxed);
        let shift_event_needs_handling = if !is_shift_vk {
            false
        } else if kbd.vkCode == VK_LSHIFT.0 as u32 || scan_code == 0x2A {
            left_shift_needs_handling || left_shift_capture_for_romaji
        } else if kbd.vkCode == VK_RSHIFT.0 as u32 || scan_code == 0x36 {
            right_shift_needs_handling || right_shift_capture_for_romaji
        } else {
            left_shift_needs_handling
                || right_shift_needs_handling
                || left_shift_capture_for_romaji
                || right_shift_capture_for_romaji
        };
        let ctrl_event_needs_handling = if !is_ctrl_vk {
            false
        } else if kbd.vkCode == VK_LCONTROL.0 as u32
            || ((kbd.flags.0 & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0) == 0
                && scan_code == 0x1D)
        {
            left_ctrl_needs_handling
        } else if kbd.vkCode == VK_RCONTROL.0 as u32
            || ((kbd.flags.0 & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0) != 0
                && scan_code == 0x1D)
        {
            right_ctrl_needs_handling
        } else {
            left_ctrl_needs_handling || right_ctrl_needs_handling
        };
        let win_event_needs_handling = if !is_win_vk {
            false
        } else if kbd.vkCode == VK_LWIN.0 as u32 {
            left_win_needs_handling
        } else if kbd.vkCode == VK_RWIN.0 as u32 {
            right_win_needs_handling
        } else {
            left_win_needs_handling || right_win_needs_handling
        };

        // Keep captured Shift physical state in sync at hook time so the next key
        // can observe Shift even before worker-thread processing catches up.
        if is_shift_vk {
            if let Some(side) = shift_side_for_event(scan_code, kbd.vkCode) {
                if captured_shift_side_enabled(side) {
                    if let Some(mut captured) = try_lock_captured_shift_state() {
                        let side_state = captured_shift_side_mut(&mut captured, side);
                        if up {
                            side_state.physical_down = false;
                        } else if !side_state.physical_down {
                            side_state.physical_down = true;
                            side_state.os_down_sent = false;
                            side_state.used_for_layout_output = false;
                        }
                    }
                }
            }
        }

        // Pass through Modifier key events themselves to ensure OS state is updated
        if (is_shift_vk && !shift_event_needs_handling)
            || (is_ctrl_vk && !ctrl_event_needs_handling)
            || (is_win_vk && !win_event_needs_handling)
            || (is_alt_vk && !alt_needs_handling)
        {
            // Allow them through, but shortcuts might still combinations of them.
            // Wait, if it's solely a modifier key press, we must pass_through.
            // But if it's part of a shortcut, evaluating the shortcut might require waiting.
            // For now modifiers pass through.
            return pass_through();
        }

        let ctrl_pressed = (lctrl_pressed && !left_ctrl_needs_handling)
            || (rctrl_pressed && !right_ctrl_needs_handling);
        let (captured_lshift_pressed, captured_rshift_pressed) = captured_shift_down_snapshot();
        let shift_pressed = (lshift_pressed && !left_shift_needs_handling)
            || (rshift_pressed && !right_shift_needs_handling)
            || (lshift_pressed && left_shift_capture_for_romaji)
            || (rshift_pressed && right_shift_capture_for_romaji)
            || (captured_lshift_pressed && left_shift_capture_for_romaji)
            || (captured_rshift_pressed && right_shift_capture_for_romaji);

        let win_pressed = (lwin_pressed && !left_win_needs_handling)
            || (rwin_pressed && !right_win_needs_handling);

        if (ctrl_pressed || win_pressed || (alt_pressed && !alt_needs_handling))
            && !is_any_shortcut
        {
            return pass_through();
        }

        let raw_ext =
            (kbd.flags.0 & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0) != 0;
        // Shift is represented as non-extended scancode in profile/config.
        // Normalize here so RightShift always matches thumb-shift assignment.
        let ext = if is_shift_vk { false } else { raw_ext };

        let event = HookEvent {
            sc: scan_code,
            ext,
            up,
            shift: shift_pressed, // keep original shift truth for chord engine
            vk: kbd.vkCode,
            is_suspend: is_suspend_key,
            is_settings: is_settings_key,
            is_switch_layout: is_switch_layout_key,
        };

        match HOOK_QUEUE.0.try_send(event) {
            Ok(()) => LRESULT(1), // Block original; worker will decide inject/pass.
            Err(TrySendError::Full(_)) => pass_through(),
            Err(TrySendError::Disconnected(_)) => pass_through(),
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
    if let Some(side) = shift_side_for_event(event.sc, event.vk) {
        if captured_shift_side_enabled(side) {
            let (should_inject_down, should_inject_up) = {
                let mut captured = lock_captured_shift_state();
                let side_state = captured_shift_side_mut(&mut captured, side);

                if !event.up {
                    if !side_state.physical_down {
                        side_state.physical_down = true;
                        side_state.os_down_sent = false;
                        side_state.used_for_layout_output = false;
                    }
                    return;
                }

                let inject_down = !side_state.os_down_sent && !side_state.used_for_layout_output;
                let inject_up = side_state.os_down_sent || !side_state.used_for_layout_output;

                *side_state = CapturedShiftSideState::default();
                (inject_down, inject_up)
            };

            if should_inject_down {
                let _ = inject_scancode(shift_scancode(side), false, false);
            }
            if should_inject_up {
                let _ = inject_scancode(shift_scancode(side), false, true);
            }
            return;
        }
    }

    let is_shift_event = shift_side_for_event(event.sc, event.vk).is_some();

    let (action, refresh_flags) = {
        let mut engine = ENGINE.lock();
        ALT_NEEDS_HANDLING.store(engine.needs_alt_handling(), Ordering::Relaxed);
        LEFT_SHIFT_NEEDS_HANDLING.store(engine.needs_left_shift_handling(), Ordering::Relaxed);
        RIGHT_SHIFT_NEEDS_HANDLING.store(engine.needs_right_shift_handling(), Ordering::Relaxed);
        LEFT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(
            engine.capture_left_shift_for_romaji_pinky_shift(),
            Ordering::Relaxed,
        );
        RIGHT_SHIFT_CAPTURE_FOR_ROMAJI_PINKY.store(
            engine.capture_right_shift_for_romaji_pinky_shift(),
            Ordering::Relaxed,
        );
        LEFT_CTRL_NEEDS_HANDLING.store(engine.needs_left_ctrl_handling(), Ordering::Relaxed);
        RIGHT_CTRL_NEEDS_HANDLING.store(engine.needs_right_ctrl_handling(), Ordering::Relaxed);
        LEFT_WIN_NEEDS_HANDLING.store(engine.needs_left_win_handling(), Ordering::Relaxed);
        RIGHT_WIN_NEEDS_HANDLING.store(engine.needs_right_win_handling(), Ordering::Relaxed);

        let mut refresh_flags = false;

        if event.is_suspend && !event.up {
            let current = engine.is_enabled();
            engine.set_enabled(!current);
            info!(
                "Suspend Key triggered. Toggled enabled state to: {}",
                !current
            );
            refresh_flags = true;
        }

        if event.is_settings && !event.up {
            engine.trigger_settings_shortcut();
        }

        if event.is_switch_layout && !event.up {
            engine.trigger_switch_layout_shortcut();
        }

        if event.is_suspend || event.is_settings || event.is_switch_layout {
            (KeyAction::Block, refresh_flags)
        } else {
            (
                engine.process_key(event.sc, event.ext, event.up, event.shift),
                refresh_flags,
            )
        }
    };

    if refresh_flags {
        refresh_runtime_flags_from_engine();
    }

    match action {
        KeyAction::Pass => {
            if !is_shift_event {
                let _ = forward_pending_captured_shift_downs_if_needed();
            }
            let _ = inject_scancode(event.sc, event.ext, event.up);
        }
        KeyAction::Block => {}
        KeyAction::Inject(events) => {
            if !is_shift_event && event.shift {
                mark_captured_shift_used_for_layout_output();
            }
            for ev in events {
                match ev {
                    InputEvent::Scancode(sc, ext, up) => {
                        let _ = inject_scancode(sc, ext, up);
                    }
                    InputEvent::Unicode(c, up) => {
                        let _ = inject_unicode(c, up);
                    }
                    InputEvent::CommitImeComposition => {
                        if crate::ime::is_composition_active() {
                            let _ = inject_scancode(0x1C, false, false);
                            let _ = inject_scancode(0x1C, false, true);
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    InputEvent::ImeControl(open) => {
                        // IME Control is a state change, not a key press/release pair.
                        // Ideally we should execute it only once.
                        // Since engine emits it as a single event, we just execute it.
                        crate::ime::set_force_ime_status(open);
                    }
                    InputEvent::WaitUntilImeStatus(expected, timeout_ms) => {
                        let start = monotonic_ms();
                        loop {
                            // Check current IME status (using relaxed check to avoid excessive overhead?)
                            // is_japanese_input_active queries OS.
                            // If expected is true (ON), we want is_japanese_input_active to be true.
                            // If expected is false (OFF), we want it to be false.

                            // Note: We might want to pass ImeMode here if needed, but Engine manages it.
                            // For now, assume Ignore mode behavior (check actual status) or use Auto.
                            // Let's use ImeMode::Ignore to force check actual OS status without mode override logic.
                            let current = crate::ime::is_japanese_input_active(
                                crate::chord_engine::ImeMode::Auto,
                            );
                            if current == expected {
                                break;
                            }

                            if monotonic_ms() - start >= timeout_ms {
                                warn!(
                                    "WaitUntilImeStatus timed out after {}ms (expected: {}, actual: {})",
                                    timeout_ms, expected, current
                                );
                                break;
                            }

                            // Sleep briefly to yield CPU
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    InputEvent::Delay(ms) => {
                        thread::sleep(Duration::from_millis(ms));
                    }
                    InputEvent::DirectString(s) => {
                        // Robust IME handling implemented here to avoid deadlock in Engine.
                        let ime_active = crate::ime::is_japanese_input_active(
                            crate::chord_engine::ImeMode::Auto,
                        );

                        if ime_active {
                            crate::ime::set_force_ime_status(false);
                            // Wait for OFF
                            let start = monotonic_ms();
                            loop {
                                if !crate::ime::is_japanese_input_active(
                                    crate::chord_engine::ImeMode::Auto,
                                ) {
                                    break;
                                }
                                if monotonic_ms() - start >= 50 {
                                    warn!("DirectString: Wait for IME OFF timed out");
                                    break;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                        }

                        for c in s.chars() {
                            let _ = inject_unicode(c, false);
                            let _ = inject_unicode(c, true);
                        }

                        if ime_active {
                            // Delay to prevent overtaking
                            thread::sleep(Duration::from_millis(10));

                            crate::ime::set_force_ime_status(true);
                            // Wait for ON
                            let start = monotonic_ms();
                            loop {
                                if crate::ime::is_japanese_input_active(
                                    crate::chord_engine::ImeMode::Auto,
                                ) {
                                    break;
                                }
                                if monotonic_ms() - start >= 50 {
                                    warn!("DirectString: Wait for IME ON timed out");
                                    break;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                        }
                    }
                }
            }
        }
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

pub fn check_accessibility_permission() -> bool {
    true // Not needed on Windows
}

pub fn request_accessibility_permission() -> bool {
    true // Not needed on Windows
}

pub fn is_sc_key_physically_down(key: crate::types::ScKey) -> Option<bool> {
    let mut scan = key.sc as u32;
    if key.ext {
        scan |= 0xE000;
    }
    let vk = unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK_EX) } as i32;
    if vk == 0 {
        return None;
    }
    let pressed = unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 };
    Some(pressed)
}

pub fn vk_to_scancode(vk: u16) -> Option<(u16, bool)> {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    if scan == 0 {
        return None;
    }
    let ext = (scan & 0xFF00) == 0xE000;
    Some(((scan & 0x00FF) as u16, ext))
unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let msg = wparam.0 as u32;
        let is_mouse_event = msg == WM_LBUTTONDOWN
            || msg == WM_LBUTTONUP
            || msg == WM_RBUTTONDOWN
            || msg == WM_RBUTTONUP
            || msg == WM_MBUTTONDOWN
            || msg == WM_MBUTTONUP
            || msg == WM_XBUTTONDOWN
            || msg == WM_XBUTTONUP
            || msg == WM_MOUSEWHEEL
            || msg == WM_MOUSEHWHEEL
            || msg == WM_MOUSEMOVE;

        // Ignore injected mouse events to prevent recursion
        let mouse = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        if mouse.dwExtraInfo == INJECTED_EXTRA_INFO || (mouse.flags & LLKHF_INJECTED.0) != 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        if is_mouse_event {
            let engine_enabled = ENGINE_ENABLED.load(Ordering::Relaxed);
            if engine_enabled {
                // Check and forward pending Shift downs BEFORE the mouse event is processed by the OS
                let injected = forward_pending_captured_shift_downs_if_needed();
                if injected {
                    // We just injected a Shift Down event.
                    // This means the OS might process the current mouse event BEFORE our injected Shift Down
                    // if we just pass_through.
                    // To guarantee the order (Shift Down, then Mouse Event), we MUST block this
                    // original mouse event and inject a new copy of it right after our Shift Down.
                    let mouse_data = mouse.mouseData;
                    // Standard flags
                    let dw_flags = match msg {
                        WM_LBUTTONDOWN => MOUSEEVENTF_LEFTDOWN,
                        WM_LBUTTONUP => MOUSEEVENTF_LEFTUP,
                        WM_RBUTTONDOWN => MOUSEEVENTF_RIGHTDOWN,
                        WM_RBUTTONUP => MOUSEEVENTF_RIGHTUP,
                        WM_MBUTTONDOWN => MOUSEEVENTF_MIDDLEDOWN,
                        WM_MBUTTONUP => MOUSEEVENTF_MIDDLEUP,
                        WM_XBUTTONDOWN => MOUSEEVENTF_XDOWN,
                        WM_XBUTTONUP => MOUSEEVENTF_XUP,
                        WM_MOUSEWHEEL => MOUSEEVENTF_WHEEL,
                        WM_MOUSEHWHEEL => MOUSEEVENTF_HWHEEL,
                        WM_MOUSEMOVE => MOUSEEVENTF_MOVE,
                        _ => MOUSEEVENTF_MOVE, // Fallback
                    };

                    let input = INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: mouse_data,
                                dwFlags: dw_flags,
                                time: 0,
                                dwExtraInfo: INJECTED_EXTRA_INFO,
                            },
                        },
                    };

                    unsafe {
                        windows::Win32::UI::Input::KeyboardAndMouse::SendInput(
                            &[input],
                            std::mem::size_of::<INPUT>() as i32,
                        );
                    }

                    return LRESULT(1); // Block the original event
                }
            }
        }

        CallNextHookEx(None, code, wparam, lparam)
    }));

    match result {
        Ok(res) => res,
        Err(_) => {
            error!("Panic in mouse_hook_proc; falling back to CallNextHookEx");
            CallNextHookEx(None, code, wparam, lparam)
        }
    }
}
