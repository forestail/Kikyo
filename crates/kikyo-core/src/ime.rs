use crate::chord_engine::ImeMode;
use std::mem::size_of;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

pub fn is_ime_on(mode: ImeMode) -> bool {
    match mode {
        ImeMode::Ignore => true,
        ImeMode::Auto => query_tsf().or_else(query_imm).unwrap_or(true),
        ImeMode::Tsf => query_tsf().unwrap_or(true),
        ImeMode::Imm => query_imm().unwrap_or(true),
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
