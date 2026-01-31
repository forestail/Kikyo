use crate::engine::ENGINE;
use crate::types::InputEvent;
use crate::types::KeyAction;
use std::sync::Mutex;
use tracing::{error, info};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PeekMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYUP,
    WM_SYSKEYUP,
};
/// Magic number to identify our own injected events.
const INJECTED_EXTRA_INFO: usize = 0xFFC3C3C3;

static HOOK_HANDLE: Mutex<Option<HHOOK>> = Mutex::new(None);

/// Starts the keyboard hook.
/// This must be called from a thread that pumps messages (GetMessage/PeekMessage).
pub fn install_hook() -> anyhow::Result<()> {
    info!("Installing keyboard hook...");

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
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    info!("Message loop exited.");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);

    // Check self-injection guard
    if kbd.dwExtraInfo == INJECTED_EXTRA_INFO {
        // Pass through our own events
        // tracing::trace!("Ignored injected event");
        return CallNextHookEx(None, code, wparam, lparam);
    }

    // Log visible events
    let msg = wparam.0 as u32;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    // Limit logging to avoid flooding, but needed for debug
    // info!("Hook saw key: vk={:X} scan={:X} up={}", kbd.vkCode, kbd.scanCode, up);

    // Emergency Stop Check: Ctrl + Alt + Esc
    if kbd.vkCode == VK_ESCAPE.0 as u32 {
        let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0;
        let alt = GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0;
        if ctrl && alt {
            error!("EMERGENCY STOP TRIGGERED (Ctrl+Alt+Esc). Exiting process.");
            std::process::exit(1);
        }
    }

    // Check for modifiers to disable hook
    let ctrl_pressed = GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0;
    let alt_pressed = GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0;
    let shift_pressed = GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000 != 0;
    let lwin_pressed = GetAsyncKeyState(VK_LWIN.0 as i32) as u16 & 0x8000 != 0;
    let rwin_pressed = GetAsyncKeyState(VK_RWIN.0 as i32) as u16 & 0x8000 != 0;

    // Pass through Modifier key events themselves to ensure OS state is updated
    if kbd.vkCode == VK_SHIFT.0 as u32
        || kbd.vkCode == VK_LSHIFT.0 as u32
        || kbd.vkCode == VK_RSHIFT.0 as u32
        || kbd.vkCode == VK_CONTROL.0 as u32
        || kbd.vkCode == VK_LCONTROL.0 as u32
        || kbd.vkCode == VK_RCONTROL.0 as u32
        || kbd.vkCode == VK_MENU.0 as u32
        || kbd.vkCode == VK_LMENU.0 as u32
        || kbd.vkCode == VK_RMENU.0 as u32
        || kbd.vkCode == VK_LWIN.0 as u32
        || kbd.vkCode == VK_RWIN.0 as u32
    {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let suspend_key_code = {
        let engine = ENGINE.lock();
        let profile = engine.get_profile();
        match profile.suspend_key {
            crate::chord_engine::SuspendKey::None => None,
            crate::chord_engine::SuspendKey::ScrollLock => Some(0x91), // VK_SCROLL
            crate::chord_engine::SuspendKey::Pause => Some(0x13),      // VK_PAUSE
            crate::chord_engine::SuspendKey::Insert => Some(0x2D),     // VK_INSERT
            crate::chord_engine::SuspendKey::RightShift => Some(0xA1), // VK_RSHIFT
            crate::chord_engine::SuspendKey::RightControl => Some(0xA3), // VK_RCONTROL
            crate::chord_engine::SuspendKey::RightAlt => Some(0xA5),   // VK_RMENU
        }
    };

    if let Some(vk) = suspend_key_code {
        if kbd.vkCode == vk && !up {
            // Check for edge case: RightShift etc might be triggered by standard Shift if not distinguishable?
            // VK_RSHIFT is specific extended key or just specific ScanCode?
            // KBDLLHOOKSTRUCT has valid vkCode for L/R differentiation usually.

            let mut engine = ENGINE.lock();
            let current = engine.is_enabled();
            engine.set_enabled(!current);
            info!(
                "Suspend Key triggered. Toggled enabled state to: {}",
                !current
            );
        }
    }

    if ctrl_pressed || alt_pressed || lwin_pressed || rwin_pressed {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let ext = (kbd.flags.0 & windows::Win32::UI::WindowsAndMessaging::LLKHF_EXTENDED.0) != 0;

    let action = {
        let mut engine = ENGINE.lock();
        engine.process_key(kbd.scanCode as u16, ext, up, shift_pressed)
    };

    match action {
        KeyAction::Pass => CallNextHookEx(None, code, wparam, lparam),
        KeyAction::Block => LRESULT(1), // Block
        KeyAction::Inject(events) => {
            // info!("Injecting {} events replacement", events.len()); // Reduce noise
            for event in events {
                match event {
                    InputEvent::Scancode(sc, ext, up) => {
                        let _ = inject_scancode(sc, ext, up);
                    }
                    InputEvent::Unicode(c, up) => {
                        let _ = inject_unicode(c, up);
                    }
                }
            }
            LRESULT(1) // Block original
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
