#![windows_subsystem = "windows"]

mod hook;
mod ime;
mod tray;

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, WriteFile, OPEN_EXISTING};
use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, GetStdHandle, SetStdHandle, WriteConsoleW,
    ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MessageBoxW, TranslateMessage, MB_ICONINFORMATION, MB_OK,
    MSG,
};

use crate::hook::{install_hook, set_ime_mode, set_timeout_ms, uninstall_hook};
use crate::ime::ImeMode;
use crate::tray::TrayIcon;

static CONSOLE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn console_print(s: &str) {
    if !CONSOLE_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if !handle.is_null() && handle != -1isize as _ {
            let wide: Vec<u16> = s.encode_utf16().collect();
            let mut written = 0;
            let success = WriteConsoleW(
                handle,
                wide.as_ptr(),
                wide.len() as u32,
                &mut written,
                null_mut(),
            );
            if success == 0 {
                let bytes = s.as_bytes();
                let mut bytes_written = 0;
                WriteFile(
                    handle,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut bytes_written,
                    null_mut(),
                );
            }
        }
    }
}

macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::console_print(&format!("{}\r\n", format_args!($($arg)*)));
    };
}
pub(crate) use log_info;

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn attach_or_alloc_console() {
    unsafe {
        let attached = if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            true
        } else {
            AllocConsole() != 0
        };

        if attached {
            let conout_name = to_wide_null("CONOUT$");
            let conout = CreateFileW(
                conout_name.as_ptr(),
                0xC0000000, // GENERIC_READ | GENERIC_WRITE
                1 | 2,      // FILE_SHARE_READ | FILE_SHARE_WRITE
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if !conout.is_null() && conout != -1isize as _ {
                SetStdHandle(STD_OUTPUT_HANDLE, conout);
                SetStdHandle(STD_ERROR_HANDLE, conout);
            }
            CONSOLE_ENABLED.store(true, Ordering::Relaxed);
        }
    }
}

fn print_help() {
    log_info!(
        r#"ctrl-ime-key - US keyboard Ctrl tap IME on/off utility
Usage: ctrl-ime-key.exe [OPTIONS]

Options:
  --console               Attach/allocate console to view debug logs
  --mode <MODE>           IME switching mode:
                            imm      IMM32 API message (alt-ime-ahk method) [default]
                            ime      VK_IME_ON / VK_IME_OFF (Windows 10/11 standard)
                            convert  VK_CONVERT / VK_NONCONVERT (Henkan / Muhenkan)
                            hybrid   Send both virtual keys
  --timeout <MS>          Ctrl tap timeout in milliseconds (default: 1000)
  -h, --help              Print this help information"#
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut initial_mode = ImeMode::ImmMessage;
    let mut tap_timeout = hook::DEFAULT_TIMEOUT_MS;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--console" => {
                attach_or_alloc_console();
            }
            "-h" | "--help" => {
                attach_or_alloc_console();
                print_help();
                return;
            }
            "--mode" => {
                if i + 1 < args.len() {
                    i += 1;
                    match args[i].to_lowercase().as_str() {
                        "imm" => initial_mode = ImeMode::ImmMessage,
                        "ime" => initial_mode = ImeMode::ImeKey,
                        "convert" => initial_mode = ImeMode::ConvertKey,
                        "hybrid" => initial_mode = ImeMode::Hybrid,
                        other => {
                            attach_or_alloc_console();
                            log_info!("Unknown mode: {}. Use 'imm', 'ime', 'convert', or 'hybrid'.", other);
                            return;
                        }
                    }
                }
            }
            "--timeout" => {
                if i + 1 < args.len() {
                    i += 1;
                    if let Ok(ms) = args[i].parse::<u32>() {
                        tap_timeout = ms;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Prevent multiple instances
    let mutex_name = to_wide_null("Local\\ctrl_ime_key_single_instance_mutex");
    let mutex = unsafe { CreateMutexW(null_mut(), 1, mutex_name.as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log_info!("ctrl-ime-key is already running. Exiting.");
        if !CONSOLE_ENABLED.load(Ordering::Relaxed) {
            unsafe {
                MessageBoxW(
                    null_mut(),
                    to_wide_null("ctrl-ime-key is already running.\nCheck your system tray.")
                        .as_ptr(),
                    to_wide_null("ctrl-ime-key").as_ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        return;
    }

    set_ime_mode(initial_mode);
    set_timeout_ms(tap_timeout);

    log_info!("========================================");
    log_info!("Starting ctrl-ime-key...");
    log_info!("Mode: {}", initial_mode.name());
    log_info!("Tap Timeout: {}ms", tap_timeout);
    log_info!("Left Ctrl tap  -> IME OFF (English)");
    log_info!("Right Ctrl tap -> IME ON  (Japanese)");
    log_info!("========================================");

    let h_instance = unsafe { GetModuleHandleW(null_mut()) };

    // Register low-level keyboard hook
    if let Err(err) = install_hook(h_instance) {
        log_info!("Error: {}", err);
        return;
    }
    log_info!("Keyboard hook successfully installed.");

    // Create system tray icon
    let _tray = match TrayIcon::new(h_instance) {
        Ok(t) => {
            log_info!("System tray icon registered.");
            Some(t)
        }
        Err(err) => {
            log_info!("Warning: Failed to create tray icon: {}", err);
            None
        }
    };

    // Run Windows message loop
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    log_info!("Shutting down ctrl-ime-key...");
    uninstall_hook();

    if !mutex.is_null() {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(mutex);
        }
    }
}
