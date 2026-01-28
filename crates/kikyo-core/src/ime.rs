use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SendMessageW, WM_IME_CONTROL};

const IMC_GETOPENSTATUS: usize = 0x0005;

/// Checks if the IME is currently open (ON) for the foreground window.
pub fn is_ime_on() -> bool {
    unsafe {
        let hwnd_foreground = GetForegroundWindow();
        if hwnd_foreground.0 == 0 {
            return false;
        }

        let hwnd_ime = ImmGetDefaultIMEWnd(hwnd_foreground);
        if hwnd_ime.0 == 0 {
            return false;
        }

        let status = SendMessageW(
            hwnd_ime,
            WM_IME_CONTROL,
            WPARAM(IMC_GETOPENSTATUS as usize),
            LPARAM(0),
        );

        status.0 != 0
    }
}
