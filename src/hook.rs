use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_LCONTROL, VK_RCONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::ime::{set_ime_state, ImeMode, EXTRA_INFO_MAGIC};
use crate::log_info;

/// Default tap timeout in milliseconds.
/// If Ctrl is held longer than this, releasing it will not trigger IME toggle.
pub const DEFAULT_TIMEOUT_MS: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapResult {
    None,
    LeftCtrlTap,
    RightCtrlTap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingCtrl {
    None,
    Left { pressed_time: u32 },
    Right { pressed_time: u32 },
}

pub fn is_left_ctrl(vk: u32, flags: u32) -> bool {
    vk == VK_LCONTROL as u32 || (vk == VK_CONTROL as u32 && (flags & LLKHF_EXTENDED == 0))
}

pub fn is_right_ctrl(vk: u32, flags: u32) -> bool {
    vk == VK_RCONTROL as u32 || (vk == VK_CONTROL as u32 && (flags & LLKHF_EXTENDED != 0))
}

#[derive(Debug)]
pub struct CtrlTapTracker {
    pending: PendingCtrl,
    pub timeout_ms: u32,
}

impl CtrlTapTracker {
    pub const fn new(timeout_ms: u32) -> Self {
        Self {
            pending: PendingCtrl::None,
            timeout_ms,
        }
    }

    pub fn reset(&mut self) {
        self.pending = PendingCtrl::None;
    }

    pub fn on_key_down(&mut self, vk_code: u32, flags: u32, time: u32) {
        let left = is_left_ctrl(vk_code, flags);
        let right = is_right_ctrl(vk_code, flags);

        if left {
            match self.pending {
                PendingCtrl::Left { .. } => {
                    // Key repeat: keep original pressed_time
                }
                _ => {
                    self.pending = PendingCtrl::Left { pressed_time: time };
                }
            }
        } else if right {
            match self.pending {
                PendingCtrl::Right { .. } => {
                    // Key repeat: keep original pressed_time
                }
                _ => {
                    self.pending = PendingCtrl::Right { pressed_time: time };
                }
            }
        } else {
            // Any other key pressed while Ctrl was held cancels the tap!
            // (e.g. Ctrl + C, Ctrl + V, etc.)
            self.pending = PendingCtrl::None;
        }
    }

    pub fn on_key_up(&mut self, vk_code: u32, flags: u32, time: u32) -> TapResult {
        let left = is_left_ctrl(vk_code, flags);
        let right = is_right_ctrl(vk_code, flags);

        let mut result = TapResult::None;

        if left {
            if let PendingCtrl::Left { pressed_time } = self.pending {
                let elapsed = time.wrapping_sub(pressed_time);
                if elapsed <= self.timeout_ms {
                    result = TapResult::LeftCtrlTap;
                }
            }
            self.pending = PendingCtrl::None;
        } else if right {
            if let PendingCtrl::Right { pressed_time } = self.pending {
                let elapsed = time.wrapping_sub(pressed_time);
                if elapsed <= self.timeout_ms {
                    result = TapResult::RightCtrlTap;
                }
            }
            self.pending = PendingCtrl::None;
        }

        result
    }
}

struct HookGlobalState {
    tracker: CtrlTapTracker,
    mode: ImeMode,
}

static HOOK_HANDLE: Mutex<isize> = Mutex::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
static TIMEOUT_MS: AtomicU32 = AtomicU32::new(DEFAULT_TIMEOUT_MS);
static STATE: Mutex<HookGlobalState> = Mutex::new(HookGlobalState {
    tracker: CtrlTapTracker::new(DEFAULT_TIMEOUT_MS),
    mode: ImeMode::ImeKey,
});

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if !enabled {
        if let Ok(mut state) = STATE.lock() {
            state.tracker.reset();
        }
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_ime_mode(mode: ImeMode) {
    if let Ok(mut state) = STATE.lock() {
        state.mode = mode;
    }
}

pub fn get_ime_mode() -> ImeMode {
    if let Ok(state) = STATE.lock() {
        state.mode
    } else {
        ImeMode::default()
    }
}

pub fn set_timeout_ms(ms: u32) {
    TIMEOUT_MS.store(ms, Ordering::Relaxed);
    if let Ok(mut state) = STATE.lock() {
        state.tracker.timeout_ms = ms;
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        let kbd = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };

        // Ignore our own synthetic inputs
        if kbd.dwExtraInfo == EXTRA_INFO_MAGIC {
            return unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) };
        }

        if ENABLED.load(Ordering::Relaxed) {
            let msg = w_param as u32;
            let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

            if is_down {
                if let Ok(mut state) = STATE.lock() {
                    state.tracker.on_key_down(kbd.vkCode, kbd.flags, kbd.time);
                }
            } else if is_up {
                let mut trigger_ime: Option<(bool, ImeMode)> = None;

                if let Ok(mut state) = STATE.lock() {
                    let res = state.tracker.on_key_up(kbd.vkCode, kbd.flags, kbd.time);
                    match res {
                        TapResult::LeftCtrlTap => {
                            trigger_ime = Some((false, state.mode));
                        }
                        TapResult::RightCtrlTap => {
                            trigger_ime = Some((true, state.mode));
                        }
                        TapResult::None => {}
                    }
                }

                if let Some((on, mode)) = trigger_ime {
                    if on {
                        log_info!("[Right Ctrl Tap] -> IME ON (mode: {:?})", mode);
                    } else {
                        log_info!("[Left Ctrl Tap] -> IME OFF (mode: {:?})", mode);
                    }
                    unsafe {
                        set_ime_state(on, mode);
                    }
                }
            }
        }
    }

    unsafe { CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param) }
}

pub fn install_hook(h_instance: HINSTANCE) -> Result<(), &'static str> {
    unsafe {
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            h_instance,
            0,
        );
        if hook.is_null() {
            return Err("Failed to install low-level keyboard hook");
        }
        let mut handle = HOOK_HANDLE.lock().unwrap();
        *handle = hook as isize;
        Ok(())
    }
}

pub fn uninstall_hook() {
    unsafe {
        let mut handle = HOOK_HANDLE.lock().unwrap();
        let hook = *handle as HHOOK;
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
            *handle = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_C: u32 = 0x43;

    #[test]
    fn test_left_ctrl_tap_success() {
        let mut tracker = CtrlTapTracker::new(1000);
        // Left Ctrl press at 100ms
        tracker.on_key_down(VK_LCONTROL as u32, 0, 100);
        // Left Ctrl release at 250ms (within 1000ms)
        let res = tracker.on_key_up(VK_LCONTROL as u32, 0, 250);
        assert_eq!(res, TapResult::LeftCtrlTap);
    }

    #[test]
    fn test_right_ctrl_tap_success() {
        let mut tracker = CtrlTapTracker::new(1000);
        // Right Ctrl press at 100ms
        tracker.on_key_down(VK_RCONTROL as u32, LLKHF_EXTENDED, 100);
        // Right Ctrl release at 200ms
        let res = tracker.on_key_up(VK_RCONTROL as u32, LLKHF_EXTENDED, 200);
        assert_eq!(res, TapResult::RightCtrlTap);
    }

    #[test]
    fn test_ctrl_c_combo_does_not_trigger_tap() {
        let mut tracker = CtrlTapTracker::new(1000);
        // Left Ctrl press at 100ms
        tracker.on_key_down(VK_LCONTROL as u32, 0, 100);
        // 'C' press at 150ms
        tracker.on_key_down(VK_C, 0, 150);
        // 'C' release at 200ms
        let res_c = tracker.on_key_up(VK_C, 0, 200);
        assert_eq!(res_c, TapResult::None);
        // Left Ctrl release at 250ms
        let res_ctrl = tracker.on_key_up(VK_LCONTROL as u32, 0, 250);
        assert_eq!(res_ctrl, TapResult::None);
    }

    #[test]
    fn test_long_press_timeout_ignored() {
        let mut tracker = CtrlTapTracker::new(500);
        // Left Ctrl press at 1000ms
        tracker.on_key_down(VK_LCONTROL as u32, 0, 1000);
        // Left Ctrl release at 1600ms (elapsed 600ms > timeout 500ms)
        let res = tracker.on_key_up(VK_LCONTROL as u32, 0, 1600);
        assert_eq!(res, TapResult::None);
    }

    #[test]
    fn test_key_repeat_preserves_initial_timestamp() {
        let mut tracker = CtrlTapTracker::new(500);
        // Initial press at 1000ms
        tracker.on_key_down(VK_LCONTROL as u32, 0, 1000);
        // Repeat press at 1300ms
        tracker.on_key_down(VK_LCONTROL as u32, 0, 1300);
        // Release at 1400ms (1400 - 1000 = 400ms <= 500ms -> valid)
        let res = tracker.on_key_up(VK_LCONTROL as u32, 0, 1400);
        assert_eq!(res, TapResult::LeftCtrlTap);
    }

    #[test]
    fn test_vk_control_distinguished_by_extended_flag() {
        assert!(is_left_ctrl(VK_CONTROL as u32, 0));
        assert!(!is_right_ctrl(VK_CONTROL as u32, 0));

        assert!(!is_left_ctrl(VK_CONTROL as u32, LLKHF_EXTENDED));
        assert!(is_right_ctrl(VK_CONTROL as u32, LLKHF_EXTENDED));
    }

    #[test]
    fn test_reset_clears_pending() {
        let mut tracker = CtrlTapTracker::new(1000);
        tracker.on_key_down(VK_LCONTROL as u32, 0, 100);
        tracker.reset();
        let res = tracker.on_key_up(VK_LCONTROL as u32, 0, 200);
        assert_eq!(res, TapResult::None);
    }
}
