use crate::chord_engine::ImeMode;
use std::mem::size_of;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmGetOpenStatus, ImmReleaseContext, IME_CMODE_NATIVE,
    IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

pub fn is_ime_on(mode: ImeMode) -> bool {
    match mode {
        ImeMode::Ignore => true,
        ImeMode::ForceAlpha => true,
        ImeMode::Auto => query_tsf().or_else(query_imm).unwrap_or(false),
        ImeMode::Tsf => query_tsf().unwrap_or(false),
        ImeMode::Imm => query_imm().unwrap_or(false),
    }
}

pub fn is_japanese_input_active(mode: ImeMode) -> bool {
    // If ImeMode is Ignore, we treat it as "Force Enable" -> True (Japanese Mode)
    if matches!(mode, ImeMode::Ignore) {
        return true;
    }
    if matches!(mode, ImeMode::ForceAlpha) {
        return false;
    }

    // Check if IME is Open
    if !is_ime_on(mode) {
        return false;
    }

    // If Open, check Conversion Mode
    // If IME is ON but in Alpha mode, we treat it as non-Japanese.
    if let Some(mode_bits) = query_conversion_mode() {
        (mode_bits & IME_CMODE_NATIVE) != IME_CONVERSION_MODE(0)
    } else {
        // Fallback: If we can't get conversion mode, assume True if IME is Open?
        // Or False? Let's assume True to be safe (preserve existing behavior).
        true
    }
}

fn query_tsf() -> Option<bool> {
    let hwnd = focused_window()?;
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.0 == 0 {
            return None;
        }
        let open = ImmGetOpenStatus(himc).as_bool();
        let _ = ImmReleaseContext(hwnd, himc);
        Some(open)
    }
}

fn query_imm() -> Option<bool> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            return None;
        }

        let himc = ImmGetContext(hwnd_fg);
        if himc.0 == 0 {
            return None;
        }

        let open = ImmGetOpenStatus(himc).as_bool();
        let _ = ImmReleaseContext(hwnd_fg, himc);
        Some(open)
    }
}

fn focused_window() -> Option<HWND> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            return None;
        }

        let tid = GetWindowThreadProcessId(hwnd_fg, None);
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };

        if GetGUIThreadInfo(tid, &mut info).is_ok() && info.hwndFocus.0 != 0 {
            return Some(info.hwndFocus);
        }

        Some(hwnd_fg)
    }
}

fn query_conversion_mode() -> Option<IME_CONVERSION_MODE> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            return None;
        }

        let himc = ImmGetContext(hwnd_fg);
        if himc.0 == 0 {
            return None;
        }

        let mut conversion = IME_CONVERSION_MODE::default();
        let mut sentence = IME_SENTENCE_MODE::default();
        let res = ImmGetConversionStatus(
            himc,
            Some(&mut conversion as *mut _),
            Some(&mut sentence as *mut _),
        );
        let _ = ImmReleaseContext(hwnd_fg, himc);

        if res.as_bool() {
            Some(conversion)
        } else {
            None
        }
    }
}
