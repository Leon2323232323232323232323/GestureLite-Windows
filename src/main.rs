#![cfg_attr(not(test), windows_subsystem = "windows")]
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("GestureLite supports Windows x64 only");
mod actions;
mod config;
mod gesture;
mod input;
mod logging;
mod settings;
mod tray;
use windows_sys::Win32::{
    Foundation::*, System::Threading::CreateMutexW, UI::WindowsAndMessaging::*,
};
fn main() {
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let dir = config::directory();
    logging::init(dir.clone());
    if let Err(e) = run(dir) {
        logging::log(&e);
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                tray::wide(&e).as_ptr(),
                tray::wide("GestureLite error").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
fn run(dir: std::path::PathBuf) -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--check-config") {
        let c = config::load(&dir.join("config.json"))?;
        c.compile()?;
        logging::log("Configuration validation passed");
        return Ok(());
    }
    unsafe {
        let mutex = CreateMutexW(
            std::ptr::null(),
            0,
            tray::wide("Local\\GestureLite-0.1").as_ptr(),
        );
        if mutex.is_null() {
            return Err(format!("CreateMutex failed: {}", GetLastError()));
        }
        struct Handle(HANDLE);
        impl Drop for Handle {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let _mutex = Handle(mutex);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let window = FindWindowW(
                tray::wide("GestureLiteHiddenWindow").as_ptr(),
                std::ptr::null(),
            );
            if !window.is_null() {
                PostMessageW(window, WM_COMMAND, tray::SETTINGS, 0);
            }
            logging::log("Another instance already running; exiting");
            return Ok(());
        }
        if args.iter().any(|a| a == "--integration-test") {
            return tray::integration_test();
        }
        if args.iter().any(|a| a == "--trace-gestures") {
            logging::start_trace(&dir)
                .map_err(|e| format!("Cannot start gesture diagnostics: {e}"))?;
        }
        let path = dir.join("config.json");
        let c = match config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                logging::log(&format!("Config invalid/unreadable: {e}. Starting DISABLED with defaults; original file preserved."));
                MessageBoxW(std::ptr::null_mut(),tray::wide(&format!("Configuration could not be loaded. Gestures start disabled; original config is preserved.\nOpen Configuration, correct it, then Reload Configuration.\n{e}")).as_ptr(),tray::wide("GestureLite configuration").as_ptr(),MB_OK|MB_ICONWARNING);
                config::Config {
                    enabled: false,
                    ..config::Config::default()
                }
            }
        };
        tray::run(
            c,
            path,
            args.iter().any(|a| a == "--smoke-test"),
            !args.iter().any(|a| a == "--tray" || a == "--smoke-test"),
        )
    }
}
