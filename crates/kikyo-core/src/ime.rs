use crate::chord_engine::ImeMode;
use std::mem::size_of;
use tracing;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd, ImmGetOpenStatus,
    ImmReleaseContext, ImmSetOpenStatus, IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, SendMessageW, GUITHREADINFO,
    WM_IME_CONTROL,
};

const IMC_GETCONVERSIONMODE: WPARAM = WPARAM(0x0001);
const IMC_GETOPENSTATUS: WPARAM = WPARAM(0x0005);

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
    let ime_on = is_ime_on(mode);
    if !ime_on {
        // tracing::info!("IME Check: OFF (OpenStatus=false)");
        return false;
    }

    // If Open, check Conversion Mode
    // If IME is ON but in Alpha mode, we treat it as non-Japanese.
    if let Some(mode_bits) = query_conversion_mode().or_else(query_conversion_mode_msg) {
        let is_native = (mode_bits & IME_CMODE_NATIVE) != IME_CONVERSION_MODE(0);
        // tracing::info!("IME Check: ON, Native={}", is_native);
        is_native
    } else {
        // Fallback: If we can't get conversion mode, assume True if IME is Open?
        // Or False? Let's assume True to be safe (preserve existing behavior).
        // tracing::info!("IME Check: ON, ConversionMode=Unknown -> Assume True");
        true
    }
}

fn query_tsf() -> Option<bool> {
    let hwnd = focused_window()?;
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.0 == 0 {
            // tracing::warn!(
            //     "query_tsf: ImmGetContext failed for HWND {:?}. Trying fallback...",
            //     hwnd
            // );
            return query_ime_msg(hwnd);
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
            tracing::warn!("query_imm: No Foreground Window");
            return None;
        }

        let himc = ImmGetContext(hwnd_fg);
        if himc.0 == 0 {
            // tracing::warn!(
            //     "query_imm: ImmGetContext failed for FG HWND {:?}. Trying fallback...",
            //     hwnd_fg
            // );
            return query_ime_msg(hwnd_fg);
        }

        let open = ImmGetOpenStatus(himc).as_bool();
        let _ = ImmReleaseContext(hwnd_fg, himc);
        Some(open)
    }
}

fn query_ime_msg(hwnd: HWND) -> Option<bool> {
    unsafe {
        let hwnd_ime = ImmGetDefaultIMEWnd(hwnd);
        if hwnd_ime.0 == 0 {
            tracing::warn!(
                "query_ime_msg: ImmGetDefaultIMEWnd returned 0 for HWND {:?}",
                hwnd
            );
            return None;
        }

        let res = SendMessageW(hwnd_ime, WM_IME_CONTROL, IMC_GETOPENSTATUS, LPARAM(0));

        Some(res.0 != 0)
    }
}

fn query_conversion_mode_msg() -> Option<IME_CONVERSION_MODE> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            return None;
        }
        // Try focused window first? Or just foreground.
        // Usually DefaultIMEWnd is per thread/window.
        let hwnd_ime = ImmGetDefaultIMEWnd(hwnd_fg);
        if hwnd_ime.0 == 0 {
            return None;
        }

        let res = SendMessageW(hwnd_ime, WM_IME_CONTROL, IMC_GETCONVERSIONMODE, LPARAM(0));

        // res.0 is the conversion mode (u32/isize)
        Some(IME_CONVERSION_MODE(res.0 as u32))
    }
}

fn focused_window() -> Option<HWND> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            tracing::warn!("focused_window: No Foreground Window");
            return None;
        }

        let tid = GetWindowThreadProcessId(hwnd_fg, None);
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };

        if GetGUIThreadInfo(tid, &mut info).is_ok() {
            if info.hwndFocus.0 != 0 {
                return Some(info.hwndFocus);
            }
        } else {
            tracing::warn!("focused_window: GetGUIThreadInfo failed");
        }

        Some(hwnd_fg)
    }
}

fn query_conversion_mode() -> Option<IME_CONVERSION_MODE> {
    unsafe {
        let hwnd_fg = GetForegroundWindow();
        if hwnd_fg.0 == 0 {
            tracing::warn!("query_conversion_mode: No Foreground Window");
            return None;
        }

        let himc = ImmGetContext(hwnd_fg);
        if himc.0 == 0 {
            // tracing::warn!("query_conversion_mode: ImmGetContext failed");
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
            tracing::warn!("query_conversion_mode: ImmGetConversionStatus failed");
            None
        }
    }
}

const IMC_SETOPENSTATUS: WPARAM = WPARAM(0x0006);

pub fn set_force_ime_status(open: bool) {
    // Try both ImmSetOpenStatus and TSF-like approaches if needed.
    // For now, standard ImmSetOpenStatus on the focused window context usually works for legacy apps.
    // For TSF apps, it might be more complex, but let's start with IMM.
    let hwnd = match focused_window() {
        Some(h) => h,
        None => {
            tracing::warn!("set_force_ime_status: No focused window found");
            return;
        }
    };

    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.0 == 0 {
            // tracing::warn!("set_force_ime_status: ImmGetContext failed, trying fallback message...");
            set_force_ime_status_msg(hwnd, open);
            return;
        }

        let res = ImmSetOpenStatus(himc, open);
        let _ = ImmReleaseContext(hwnd, himc);

        if !res.as_bool() {
            // tracing::warn!("set_force_ime_status: ImmSetOpenStatus failed, trying fallback message...");
            set_force_ime_status_msg(hwnd, open);
        }
    }
}

fn set_force_ime_status_msg(hwnd: HWND, open: bool) {
    unsafe {
        let hwnd_ime = ImmGetDefaultIMEWnd(hwnd);
        if hwnd_ime.0 == 0 {
            tracing::warn!("set_force_ime_status_msg: ImmGetDefaultIMEWnd failed");
            return;
        }
        let _ = SendMessageW(
            hwnd_ime,
            WM_IME_CONTROL,
            IMC_SETOPENSTATUS,
            LPARAM(if open { 1 } else { 0 }),
        );
    }
}
