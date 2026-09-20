use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, GetCursorPos, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TrackPopupMenu,
    HICON, IDI_APPLICATION, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_COMMAND,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};

use crate::hook::{get_ime_mode, is_enabled, set_enabled, set_ime_mode};
use crate::ime::ImeMode;

const WM_TRAYICON: u32 = WM_USER + 100;

const IDM_HEADER: usize = 1000;
const IDM_TOGGLE_ENABLED: usize = 1001;
const IDM_MODE_IMM: usize = 1002;
const IDM_MODE_IME_KEY: usize = 1003;
const IDM_MODE_CONVERT_KEY: usize = 1004;
const IDM_MODE_HYBRID: usize = 1005;
const IDM_EXIT: usize = 1006;

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn load_app_icon(h_instance: HINSTANCE) -> HICON {
    unsafe {
        // Attempt to load embedded resource icon (ID: 1, compiled via winres in build.rs)
        let icon = LoadIconW(h_instance, 1 as usize as *const u16);
        if !icon.is_null() {
            return icon;
        }
        // Fallback to standard application icon
        LoadIconW(null_mut(), IDI_APPLICATION)
    }
}

pub struct TrayIcon {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

impl TrayIcon {
    pub fn new(h_instance: HINSTANCE) -> Result<Self, &'static str> {
        unsafe {
            let class_name = to_wide_null("CtrlImeKeyTrayClass");

            let app_icon = load_app_icon(h_instance);

            let wnd_class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(tray_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: h_instance,
                hIcon: app_icon,
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null_mut(),
                lpszClassName: class_name.as_ptr(),
            };

            RegisterClassW(&wnd_class);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                to_wide_null("CtrlImeKeyTrayWindow").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                h_instance,
                null_mut(),
            );

            if hwnd.is_null() {
                return Err("Failed to create tray helper window");
            }

            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = app_icon;

            Self::update_tip(&mut nid, is_enabled());

            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                DestroyWindow(hwnd);
                return Err("Failed to register tray icon");
            }

            Ok(Self { hwnd, nid })
        }
    }

    fn update_tip(nid: &mut NOTIFYICONDATAW, enabled: bool) {
        let status = if enabled { "Active" } else { "Paused" };
        let tip = format!("ctrl-ime-key [{}]\nL-Ctrl: IME OFF | R-Ctrl: IME ON", status);
        let wide = to_wide_null(&tip);
        let len = wide.len().min(nid.szTip.len() - 1);
        for i in 0..len {
            nid.szTip[i] = wide[i];
        }
        nid.szTip[len] = 0;
    }

    #[allow(dead_code)]
    pub fn refresh_tooltip(&mut self) {
        unsafe {
            Self::update_tip(&mut self.nid, is_enabled());
            Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.nid);
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe fn show_context_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }

        let enabled = is_enabled();
        let mode = get_ime_mode();

        let enabled_flags = MF_STRING | if enabled { MF_CHECKED } else { MF_UNCHECKED };
        let imm_flags =
            MF_STRING | if mode == ImeMode::ImmMessage { MF_CHECKED } else { MF_UNCHECKED };
        let ime_key_flags =
            MF_STRING | if mode == ImeMode::ImeKey { MF_CHECKED } else { MF_UNCHECKED };
        let convert_key_flags =
            MF_STRING | if mode == ImeMode::ConvertKey { MF_CHECKED } else { MF_UNCHECKED };
        let hybrid_flags =
            MF_STRING | if mode == ImeMode::Hybrid { MF_CHECKED } else { MF_UNCHECKED };

        let header_text = to_wide_null("ctrl-ime-key (US Keyboard IME Toggle)");
        let toggle_text =
            to_wide_null(if enabled { "Enabled (有効)" } else { "Disabled (一時停止中)" });
        let mode_imm_text = to_wide_null("Mode: IMM Message (alt-ime-ahk互換, 推奨)");
        let mode_ime_text = to_wide_null("Mode: VK_IME_ON / OFF (Windows 10/11)");
        let mode_conv_text = to_wide_null("Mode: 変換 / 無変換 (Henkan / Muhenkan)");
        let mode_hyb_text = to_wide_null("Mode: Hybrid (両方送信)");
        let exit_text = to_wide_null("Exit (終了)");

        AppendMenuW(
            menu,
            MF_STRING | MF_GRAYED | MF_DISABLED,
            IDM_HEADER,
            header_text.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null_mut());
        AppendMenuW(menu, enabled_flags, IDM_TOGGLE_ENABLED, toggle_text.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null_mut());
        AppendMenuW(menu, imm_flags, IDM_MODE_IMM, mode_imm_text.as_ptr());
        AppendMenuW(menu, ime_key_flags, IDM_MODE_IME_KEY, mode_ime_text.as_ptr());
        AppendMenuW(
            menu,
            convert_key_flags,
            IDM_MODE_CONVERT_KEY,
            mode_conv_text.as_ptr(),
        );
        AppendMenuW(menu, hybrid_flags, IDM_MODE_HYBRID, mode_hyb_text.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, null_mut());
        AppendMenuW(menu, MF_STRING, IDM_EXIT, exit_text.as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);

        SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            null_mut(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);

        DestroyMenu(menu);
    }
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            let event = l_param as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                unsafe {
                    show_context_menu(hwnd);
                }
            }
            0
        }
        WM_COMMAND => {
            let cmd_id = (w_param & 0xFFFF) as usize;
            match cmd_id {
                IDM_TOGGLE_ENABLED => {
                    set_enabled(!is_enabled());
                }
                IDM_MODE_IMM => {
                    set_ime_mode(ImeMode::ImmMessage);
                }
                IDM_MODE_IME_KEY => {
                    set_ime_mode(ImeMode::ImeKey);
                }
                IDM_MODE_CONVERT_KEY => {
                    set_ime_mode(ImeMode::ConvertKey);
                }
                IDM_MODE_HYBRID => {
                    set_ime_mode(ImeMode::Hybrid);
                }
                IDM_EXIT => unsafe {
                    PostQuitMessage(0);
                },
                _ => {}
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}
