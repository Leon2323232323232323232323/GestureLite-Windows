use crate::{
    config::{self, Config},
    input::Hooks,
    logging::log,
};
use std::{cell::RefCell, path::PathBuf, sync::mpsc, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, RemoteDesktop::*},
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};
const TRAY: u32 = WM_APP + 1;
const ENABLE: usize = 100;
const DISABLE: usize = 101;
const OPEN: usize = 102;
const RELOAD: usize = 103;
const EXIT: usize = 104;
pub const SETTINGS: usize = 106;
struct App {
    hooks: Hooks,
    config: Config,
    path: PathBuf,
    icon: NOTIFYICONDATAW,
    taskbar_created: u32,
}
thread_local! {static APP:RefCell<Option<App>>=const {RefCell::new(None)};}
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn alert(s: &str) {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(s).as_ptr(),
            wide("GestureLite").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn apply_settings(config: Config) {
    APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            if let Ok(compiled) = config.compile() {
                app.hooks.configure(compiled);
                app.config = config;
                log("Settings saved and applied");
            }
        }
    });
}
pub fn run(
    mut config: Config,
    path: PathBuf,
    smoke: bool,
    show_settings: bool,
) -> Result<(), String> {
    if smoke {
        config.enabled = false;
    }
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("GestureLiteHiddenWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(format!("RegisterClass failed: {}", GetLastError()));
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(format!("CreateWindow failed: {}", GetLastError()));
        }
        let (tx, rx) = mpsc::sync_channel::<crate::actions::Job>(64);
        let generation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_generation = generation.clone();
        let worker = thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                crate::actions::execute(&job, &worker_generation);
            }
        });
        let hooks = match Hooks::start(config.compile()?, tx, generation) {
            Ok(h) => h,
            Err(e) => {
                DestroyWindow(hwnd);
                let _ = worker.join();
                return Err(e);
            }
        };
        let mut icon: NOTIFYICONDATAW = std::mem::zeroed();
        icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        icon.hWnd = hwnd;
        icon.uID = 1;
        icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        icon.uCallbackMessage = TRAY;
        icon.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        let tip = wide("GestureLite — right-click for controls");
        icon.szTip[..tip.len()].copy_from_slice(&tip);
        if Shell_NotifyIconW(NIM_ADD, &icon) == 0 {
            drop(hooks);
            DestroyWindow(hwnd);
            let _ = worker.join();
            return Err("Tray icon installation failed".into());
        }
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
            log("Session notification registration failed");
        }
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hooks,
                config,
                path,
                icon,
                taskbar_created,
            })
        });

        if smoke {
            SetTimer(hwnd, 1, 2000, None);
        }
        log("Started: hooks active, tray ready");
        if show_settings {
            PostMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
        }
        let mut msg: MSG = std::mem::zeroed();
        let mut result = Ok(());
        loop {
            let status = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if status == 0 {
                break;
            }
            if status < 0 {
                result = Err(format!("GetMessage failed: {}", GetLastError()));
                break;
            }
            if !crate::settings::dialog_message(&msg) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        WTSUnRegisterSessionNotification(hwnd);
        crate::settings::destroy();
        APP.with(|a| {
            if let Some(app) = a.borrow_mut().take() {
                Shell_NotifyIconW(NIM_DELETE, &app.icon);
                drop(app);
            }
        });
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        let _ = worker.join();
        log("Stopped; hooks removed");
        result
    }
}
unsafe fn menu(hwnd: HWND) {
    let enabled = APP.with(|a| a.borrow().as_ref().is_some_and(|a| a.config.enabled));
    let popup = CreatePopupMenu();
    if popup.is_null() {
        return;
    }
    for (id, label, disabled) in [
        (SETTINGS, "Settings...", false),
        (ENABLE, "Enable Gestures", enabled),
        (DISABLE, "Disable Gestures", !enabled),
        (OPEN, "Open Configuration", false),
        (RELOAD, "Reload Configuration", false),
        (EXIT, "Exit", false),
    ] {
        AppendMenuW(
            popup,
            MF_STRING | if disabled { MF_GRAYED } else { 0 },
            id,
            wide(label).as_ptr(),
        );
    }
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        popup,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(popup);
    PostMessageW(hwnd, WM_NULL, 0, 0);
    if cmd != 0 {
        PostMessageW(hwnd, WM_COMMAND, cmd as usize, 0);
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if APP.with(|a| {
        a.borrow()
            .as_ref()
            .is_some_and(|a| a.taskbar_created != 0 && message == a.taskbar_created)
    }) {
        APP.with(|a| {
            if let Some(a) = a.borrow().as_ref() {
                Shell_NotifyIconW(NIM_ADD, &a.icon);
            }
        });
        return 0;
    }
    match message {
        TRAY => {
            if l as u32 == WM_LBUTTONUP {
                PostMessageW(hwnd, WM_COMMAND, SETTINGS, 0);
            } else if l as u32 == WM_RBUTTONUP || l as u32 == WM_CONTEXTMENU {
                menu(hwnd);
            }
            0
        }
        WM_COMMAND => {
            match w & 0xffff {
                SETTINGS => {
                    let draft = APP.with(|app| {
                        app.borrow()
                            .as_ref()
                            .map(|app| (app.config.clone(), app.path.clone()))
                    });
                    if let Some((config, path)) = draft {
                        if let Err(error) = crate::settings::show(hwnd, config, path) {
                            alert(&error);
                        }
                    }
                }
                ENABLE | DISABLE => APP.with(|a| {
                    if let Some(a) = a.borrow_mut().as_mut() {
                        a.config.enabled = w & 0xffff == ENABLE;
                        if let Ok(c) = a.config.compile() {
                            a.hooks.configure(c);
                        }
                        log(if a.config.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        });
                    }
                }),
                OPEN => {
                    let path = APP.with(|a| a.borrow().as_ref().map(|a| a.path.clone()));
                    if let Some(path) = path {
                        let status = ShellExecuteW(
                            hwnd,
                            wide("open").as_ptr(),
                            wide("notepad.exe").as_ptr(),
                            wide(&format!("\"{}\"", path.display())).as_ptr(),
                            std::ptr::null(),
                            SW_SHOWNORMAL,
                        );
                        if status as isize <= 32 {
                            alert("Could not open Notepad. Open config.json from LocalAppData manually.");
                        }
                    }
                }
                RELOAD => {
                    let path = APP.with(|a| a.borrow().as_ref().map(|a| a.path.clone()));
                    if let Some(path) = path {
                        match config::load(&path) {
                            Ok(c) => APP.with(|a| {
                                if let Some(a) = a.borrow_mut().as_mut() {
                                    a.hooks.configure(c.compile().unwrap());
                                    a.config = c;
                                    log("Configuration reloaded");
                                }
                            }),
                            Err(e) => {
                                log(&format!("Configuration reload rejected: {e}"));
                                alert(&format!("Configuration rejected; the previous settings remain active.\n{e}"));
                            }
                        }
                    }
                }
                EXIT => {
                    if crate::settings::close() {
                        PostQuitMessage(0);
                    }
                }
                _ => {}
            }
            0
        }
        WM_POWERBROADCAST | WM_WTSSESSION_CHANGE | WM_DISPLAYCHANGE | WM_DEVICECHANGE => {
            APP.with(|a| {
                if let Some(a) = a.borrow().as_ref() {
                    a.hooks.reset();
                }
            });
            log(&format!(
                "Input state reset: Windows message {message:#x}, event {w}"
            ));
            1
        }
        WM_TIMER => {
            if w == 1 {
                // Diagnostic mode only: exercise the native command/reset paths before shutdown.
                SendMessageW(hwnd, WM_COMMAND, ENABLE, 0);
                SendMessageW(hwnd, WM_COMMAND, DISABLE, 0);
                SendMessageW(hwnd, WM_COMMAND, RELOAD, 0);
                SendMessageW(hwnd, WM_COMMAND, DISABLE, 0);
                SendMessageW(hwnd, WM_DISPLAYCHANGE, 0, 0);
                PostQuitMessage(0);
            }
            0
        }
        WM_CLOSE => {
            PostQuitMessage(0);
            0
        }
        WM_ENDSESSION => {
            if w != 0 {
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(hwnd, message, w, l),
    }
}
// Integration exercises real low-level hooks using benign F24 events; no mapped shortcut is dispatched.
pub fn integration_test() -> Result<(), String> {
    use crate::{
        actions::Action,
        config::{Binding, TapMode},
        gesture::MARKER,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let c = Config {
        triggers: vec![Binding {
            trigger: "F24".into(),
            tap_mode: TapMode::TapPassthrough,
            click_action: None,
            left: Some("F23".into()),
            right: Some("F23".into()),
            up: Some("F23".into()),
            down: Some("F23".into()),
        }],
        ..Config::default()
    };
    let (tx, rx) = mpsc::sync_channel::<crate::actions::Job>(64);
    let hooks = Hooks::start(
        c.compile()?,
        tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    )?;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut origin = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut origin) == 0 {
            return Err("GetCursorPos failed".into());
        }
        struct CursorRestore(POINT);
        impl Drop for CursorRestore {
            fn drop(&mut self) {
                unsafe {
                    SetCursorPos(self.0.x, self.0.y);
                }
            }
        }
        let _restore = CursorRestore(origin);
        let inject = |up: bool, marker: usize| -> Result<(), String> {
            let e = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0x87,
                        wScan: 0,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                        time: 0,
                        dwExtraInfo: marker,
                    },
                },
            };
            if SendInput(1, &e, std::mem::size_of::<INPUT>() as i32) != 1 {
                return Err("Integration SendInput failed".into());
            }
            Ok(())
        };
        inject(false, MARKER)?;
        inject(true, MARKER)?;
        if rx.recv_timeout(Duration::from_millis(100)).is_ok() {
            return Err("Own marked events were processed".into());
        }
        inject(false, 0x1234)?;
        inject(true, 0x1234)?;
        if !matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(crate::actions::Job {
                action: Action::Tap(_),
                ..
            })
        ) {
            return Err("Other injected key tap was not accepted".into());
        }
        inject(false, 0x1234)?;
        // Ordered SendInput movement follows the injected key-down in the same input stream.
        // SetCursorPos could race delivery of that key-down, invalidating the test's origin.
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        let monitor = MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            inject(true, 0x1234)?;
            return Err("Integration monitor query failed".into());
        }
        let center_x = info.rcMonitor.left + (info.rcMonitor.right - info.rcMonitor.left) / 2;
        let delta = if origin.x < center_x { 40 } else { -40 };
        let movement = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: delta,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_MOVE_NOCOALESCE,
                    time: 0,
                    dwExtraInfo: 0x1234,
                },
            },
        };
        if SendInput(1, &movement, std::mem::size_of::<INPUT>() as i32) != 1 {
            inject(true, 0x1234)?;
            return Err("Integration mouse injection failed".into());
        }
        if !matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(crate::actions::Job {
                action: Action::Chord(_),
                ..
            })
        ) {
            inject(true, 0x1234)?;
            return Err("Movement did not fire before trigger release".into());
        }
        SendInput(1, &movement, std::mem::size_of::<INPUT>() as i32);
        inject(true, 0x1234)?;
        if rx.recv_timeout(Duration::from_millis(100)).is_ok() {
            return Err("Duplicate gesture or leaked tap".into());
        }
    }
    drop(hooks);
    log("Integration passed: real hooks accepted injected F24; own marker ignored; immediate action; no duplicate/tap");
    Ok(())
}
