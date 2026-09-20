use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::Ime::{ImmGetDefaultIMEWnd, IMC_SETOPENSTATUS};

pub const IMC_GETOPENSTATUS: u32 = 0x0005;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_CONVERT, VK_IME_OFF, VK_IME_ON, VK_KANJI, VK_NONCONVERT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, SendMessageTimeoutW, GUITHREADINFO,
    SMTO_ABORTIFHUNG, WM_IME_CONTROL,
};

use crate::log_info;

/// Magic value placed into dwExtraInfo so our low-level keyboard hook
/// knows this event was injected by ctrl-ime-key and will ignore it.
pub const EXTRA_INFO_MAGIC: usize = 0x43494D45; // 'CIME'

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeMode {
    /// Control IME directly via IMM32 window message (same mechanism as alt-ime-ahk).
    /// Highly reliable and avoids triggering unwanted "reconversion" (再変換).
    #[default]
    ImmMessage,
    /// Send VK_IME_ON (0x16) / VK_IME_OFF (0x1A).
    ImeKey,
    /// Send VK_CONVERT (0x1C, 変換) / VK_NONCONVERT (0x1D, 無変換).
    ConvertKey,
    /// Send both VK_IME_ON/OFF and Henkan/Muhenkan.
    Hybrid,
}

impl ImeMode {
    pub fn name(&self) -> &'static str {
        match self {
            ImeMode::ImmMessage => "IMM API Message (alt-ime-ahk method, Recommended)",
            ImeMode::ImeKey => "VK_IME_ON / VK_IME_OFF",
            ImeMode::ConvertKey => "変換 / 無変換 (Henkan / Muhenkan)",
            ImeMode::Hybrid => "Hybrid (Virtual Keys)",
        }
    }
}

/// Retrieve the focused or active window that currently has keyboard focus.
pub unsafe fn get_target_window() -> HWND {
    unsafe {
        let mut gti: GUITHREADINFO = std::mem::zeroed();
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        // Passing 0 queries the active foreground thread
        if GetGUIThreadInfo(0, &mut gti) != 0 {
            if !gti.hwndFocus.is_null() {
                return gti.hwndFocus;
            }
            if !gti.hwndActive.is_null() {
                return gti.hwndActive;
            }
        }
        GetForegroundWindow()
    }
}

/// Get the current IME open status for the focused window.
/// Returns Some(true) for ON, Some(false) for OFF, or None if status cannot be determined.
pub unsafe fn get_ime_status(hwnd: HWND) -> Option<bool> {
    if hwnd.is_null() {
        return None;
    }
    let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
    if ime_wnd.is_null() {
        return None;
    }

    let mut result: usize = 0;
    let success = unsafe {
        SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            IMC_GETOPENSTATUS as usize,
            0,
            SMTO_ABORTIFHUNG,
            100, // 100ms timeout
            &mut result,
        )
    };

    if success != 0 {
        Some(result != 0)
    } else {
        None
    }
}

/// Set IME open status directly via WM_IME_CONTROL.
/// Returns true if the message was successfully delivered.
pub unsafe fn set_ime_by_message(hwnd: HWND, on: bool) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let ime_wnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
    if ime_wnd.is_null() {
        return false;
    }

    let mut result: usize = 0;
    let success = unsafe {
        SendMessageTimeoutW(
            ime_wnd,
            WM_IME_CONTROL,
            IMC_SETOPENSTATUS as usize,
            if on { 1 } else { 0 },
            SMTO_ABORTIFHUNG,
            100, // 100ms timeout
            &mut result,
        )
    };

    success != 0
}

fn create_key_input(vk: u16, scan: u16, key_up: bool) -> INPUT {
    let mut flags = 0;
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: EXTRA_INFO_MAGIC,
            },
        },
    }
}

/// Send a press and release of a virtual key.
unsafe fn send_key_stroke(vk: u16, scan: u16) {
    let mut inputs = [
        create_key_input(vk, scan, false),
        create_key_input(vk, scan, true),
    ];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Main entry point to switch IME on (Japanese) or off (English / Alphanumeric).
pub unsafe fn set_ime_state(on: bool, mode: ImeMode) {
    match mode {
        ImeMode::ImmMessage => {
            let hwnd = unsafe { get_target_window() };
            let current = unsafe { get_ime_status(hwnd) };

            log_info!(
                "ImmMessage: Target HWND={:?}, Current IME status={:?}, Desired={}",
                hwnd,
                current,
                if on { "ON" } else { "OFF" }
            );

            // If already in the desired state, do nothing!
            // This completely prevents unwanted "reconversion" (再変換) when pressing Right Ctrl.
            if let Some(cur) = current {
                if cur == on {
                    log_info!("IME is already in the requested state. Skipping.");
                    return;
                }
            }

            // Attempt to set status via direct WM_IME_CONTROL message
            let sent = unsafe { set_ime_by_message(hwnd, on) };
            log_info!("set_ime_by_message sent: {}", sent);

            // Verify if status changed. If not, fallback to VK_KANJI toggle.
            let after = unsafe { get_ime_status(hwnd) };
            if let Some(status_after) = after {
                if status_after != on {
                    log_info!("Status did not change via message, falling back to VK_KANJI toggle");
                    unsafe { send_key_stroke(VK_KANJI, 0) };
                }
            } else if !sent {
                // If message could not be delivered at all (e.g. some modern UWP apps),
                // fallback to VK_KANJI toggle if state needed to change
                log_info!("Could not deliver message to IME window, fallback to VK_KANJI toggle");
                unsafe { send_key_stroke(VK_KANJI, 0) };
            }
        }
        ImeMode::ImeKey => {
            let vk = if on { VK_IME_ON } else { VK_IME_OFF };
            unsafe { send_key_stroke(vk, 0) };
        }
        ImeMode::ConvertKey => {
            let (vk, scan) = if on {
                (VK_CONVERT, 0x79)
            } else {
                (VK_NONCONVERT, 0x7B)
            };
            unsafe { send_key_stroke(vk, scan) };
        }
        ImeMode::Hybrid => {
            let vk_ime = if on { VK_IME_ON } else { VK_IME_OFF };
            unsafe { send_key_stroke(vk_ime, 0) };

            let (vk_conv, scan) = if on {
                (VK_CONVERT, 0x79)
            } else {
                (VK_NONCONVERT, 0x7B)
            };
            unsafe { send_key_stroke(vk_conv, scan) };
        }
    }
}
