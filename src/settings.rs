use crate::{
    config::{self, Binding, Config, TapMode},
    tray::wide,
};
use std::{cell::Cell, fs, path::PathBuf};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::BST_CHECKED, HiDpi::*, Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "GestureLiteSettingsWindow";
thread_local! { static WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) }; }
const LIST: usize = 200;
const ADD: usize = 201;
const REMOVE: usize = 202;
const SAVE: usize = 203;
const CLOSE: usize = 204;
const ENABLED: usize = 205;
const DISTANCE: usize = 206;
const TRIGGER: usize = 207;
const TAP: usize = 208;
const CLICK: usize = 209;
const ACTION: usize = 210;
const PRESETS: &[(&str, &str)] = &[
    ("No action", ""),
    ("Previous desktop", "WIN+CTRL+LEFT"),
    ("Next desktop", "WIN+CTRL+RIGHT"),
    ("Task view", "WIN+TAB"),
    ("Show desktop", "WIN+D"),
    ("Copy", "CTRL+C"),
    ("Paste", "CTRL+V"),
    ("Cut", "CTRL+X"),
    ("Undo", "CTRL+Z"),
    ("Redo", "CTRL+Y"),
    ("Select all", "CTRL+A"),
    ("Delete", "DELETE"),
    ("Enter", "ENTER"),
    ("Save", "CTRL+S"),
    ("Close tab", "CTRL+W"),
    ("Switch app", "ALT+TAB"),
    ("Maximize window", "WIN+UP"),
    ("Minimize window", "WIN+DOWN"),
    ("Move window left", "WIN+SHIFT+LEFT"),
    ("Move window right", "WIN+SHIFT+RIGHT"),
    ("Volume up", "VOLUMEUP"),
    ("Volume down", "VOLUMEDOWN"),
    ("Mute", "VOLUMEMUTE"),
    ("Play / pause", "PLAYPAUSE"),
];
fn action_value(text: &str) -> Result<Option<String>, String> {
    let trimmed = text.trim();
    let value = PRESETS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
        .map_or(trimmed, |(_, chord)| *chord);
    if value.is_empty() {
        return Ok(None);
    }
    config::chord(value)?;
    Ok(Some(
        value
            .split('+')
            .map(|part| part.trim().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join("+"),
    ))
}
fn action_label(action: &Option<String>) -> String {
    let value = action.as_deref().unwrap_or("");
    PRESETS
        .iter()
        .find(|(_, chord)| chord.eq_ignore_ascii_case(value))
        .map_or_else(|| value.to_string(), |(name, _)| name.to_string())
}
fn trigger_label(value: &str) -> String {
    match value.to_ascii_uppercase().as_str() {
        "XBUTTON1" => "Back mouse button".into(),
        "XBUTTON2" => "Forward mouse button".into(),
        _ => value.to_string(),
    }
}
fn trigger_value(value: &str) -> String {
    match value.trim() {
        "Back mouse button" => "XButton1".into(),
        "Forward mouse button" => "XButton2".into(),
        value => value.trim().to_ascii_uppercase(),
    }
}
struct State {
    draft: Config,
    path: PathBuf,
    original: Vec<u8>,
    selected: Option<usize>,
    dirty: bool,
    loading: bool,
    controls: Vec<(HWND, (i32, i32, i32, i32))>,
    font: HFONT,
    dpi: u32,
}
unsafe fn control(hwnd: HWND, id: usize) -> HWND {
    GetDlgItem(hwnd, id as i32)
}
unsafe fn text(hwnd: HWND) -> String {
    let size = GetWindowTextLengthW(hwnd).clamp(0, 4096) as usize;
    let mut buffer = vec![0u16; size + 1];
    let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..count.max(0) as usize])
}
unsafe fn set(hwnd: HWND, value: &str) {
    SetWindowTextW(hwnd, wide(value).as_ptr());
}
unsafe fn message(hwnd: HWND, value: &str, flags: u32) -> i32 {
    MessageBoxW(
        hwnd,
        wide(value).as_ptr(),
        wide("GestureLite").as_ptr(),
        flags,
    )
}
impl State {
    unsafe fn child(
        &mut self,
        parent: HWND,
        class: &str,
        title: &str,
        id: usize,
        style: u32,
        bounds: (i32, i32, i32, i32),
    ) -> Result<HWND, String> {
        let child = CreateWindowExW(
            if class == "EDIT" || class == "LISTBOX" {
                WS_EX_CLIENTEDGE
            } else {
                0
            },
            wide(class).as_ptr(),
            wide(title).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        if child.is_null() {
            return Err(format!(
                "Settings control creation failed: {}",
                GetLastError()
            ));
        }
        self.controls.push((child, bounds));
        Ok(child)
    }
    unsafe fn layout(&mut self) {
        let old = self.font;
        self.font = CreateFontW(
            -(self.dpi as i32 * 9 / 72),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            DEFAULT_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        );
        for (child, (x, y, w, h)) in &self.controls {
            let scale = |value: i32| value * self.dpi as i32 / 96;
            MoveWindow(*child, scale(*x), scale(*y), scale(*w), scale(*h), 1);
            SendMessageW(*child, WM_SETFONT, self.font as usize, 1);
        }
        if !old.is_null() {
            DeleteObject(old);
        }
    }
    unsafe fn create(&mut self, hwnd: HWND) -> Result<(), String> {
        self.child(
            hwnd,
            "BUTTON",
            "Enable gestures",
            ENABLED,
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            (18, 12, 260, 25),
        )?;
        self.child(
            hwnd,
            "STATIC",
            "Distance per gesture:",
            0,
            0,
            (370, 16, 145, 22),
        )?;
        self.child(
            hwnd,
            "EDIT",
            "",
            DISTANCE,
            ES_NUMBER as u32 | WS_TABSTOP,
            (515, 12, 65, 25),
        )?;
        self.child(hwnd, "STATIC", "pixels", 0, 0, (590, 16, 70, 22))?;
        self.child(hwnd,"STATIC","Hold and move for one gesture per press. No gesture: click action runs on release.\nNo click action assigned: normal tap if enabled below. No timer or repeat.",0,0,(18,45,660,38))?;
        self.child(hwnd, "STATIC", "Buttons / keys", 0, 0, (18, 90, 180, 20))?;
        self.child(
            hwnd,
            "LISTBOX",
            "",
            LIST,
            LBS_NOTIFY as u32 | WS_VSCROLL | WS_TABSTOP,
            (18, 112, 180, 230),
        )?;
        self.child(
            hwnd,
            "BUTTON",
            "Add button",
            ADD,
            WS_TABSTOP,
            (18, 351, 86, 28),
        )?;
        self.child(
            hwnd,
            "BUTTON",
            "Remove",
            REMOVE,
            WS_TABSTOP,
            (112, 351, 86, 28),
        )?;
        self.child(hwnd, "STATIC", "Trigger", 0, 0, (225, 91, 90, 22))?;
        let trigger = self.child(
            hwnd,
            "COMBOBOX",
            "",
            TRIGGER,
            CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32 | WS_VSCROLL | WS_TABSTOP,
            (320, 87, 350, 200),
        )?;
        for value in [
            "Back mouse button".to_string(),
            "Forward mouse button".to_string(),
        ]
        .into_iter()
        .chain((13..=24).map(|n| format!("F{n}")))
        {
            SendMessageW(trigger, CB_ADDSTRING, 0, wide(&value).as_ptr() as isize);
        }
        self.child(
            hwnd,
            "BUTTON",
            "Use normal click if no click action is assigned",
            TAP,
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            (225, 124, 445, 25),
        )?;
        self.child(hwnd, "STATIC", "Click action", 0, 0, (225, 164, 90, 22))?;
        let click = self.child(
            hwnd,
            "COMBOBOX",
            "",
            CLICK,
            CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32 | WS_VSCROLL | WS_TABSTOP,
            (320, 160, 350, 240),
        )?;
        for (name, _) in PRESETS {
            SendMessageW(click, CB_ADDSTRING, 0, wide(name).as_ptr() as isize);
        }
        for (i, label) in ["Left", "Right", "Up", "Down"].iter().enumerate() {
            let y = 208 + i as i32 * 38;
            self.child(hwnd, "STATIC", label, 0, 0, (225, y + 4, 90, 22))?;
            let combo = self.child(
                hwnd,
                "COMBOBOX",
                "",
                ACTION + i,
                CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32 | WS_VSCROLL | WS_TABSTOP,
                (320, y, 350, 240),
            )?;
            for (name, _) in PRESETS {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(name).as_ptr() as isize);
            }
        }
        self.child(hwnd,"STATIC","Choose an action or type a shortcut: CTRL+SHIFT+S, F13, etc.\nFor G HUB, assign a button to F13-F24 and use that key here.",0,0,(225,364,445,48))?;
        self.child(
            hwnd,
            "STATIC",
            "Changes apply when you choose Save and Apply.",
            220,
            0,
            (18, 434, 410, 34),
        )?;
        self.child(
            hwnd,
            "BUTTON",
            "Save and Apply",
            SAVE,
            BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
            (440, 430, 125, 30),
        )?;
        self.child(
            hwnd,
            "BUTTON",
            "Close",
            CLOSE,
            WS_TABSTOP,
            (575, 430, 95, 30),
        )?;
        SendMessageW(
            control(hwnd, ENABLED),
            BM_SETCHECK,
            usize::from(self.draft.enabled),
            0,
        );
        set(
            control(hwnd, DISTANCE),
            &self.draft.threshold_px.to_string(),
        );
        self.refresh_list(hwnd);
        self.selected = if self.draft.triggers.is_empty() {
            None
        } else {
            Some(0)
        };
        self.load_editor(hwnd);
        self.layout();
        self.loading = false;
        Ok(())
    }
    unsafe fn refresh_list(&self, hwnd: HWND) {
        let list = control(hwnd, LIST);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        for binding in &self.draft.triggers {
            SendMessageW(
                list,
                LB_ADDSTRING,
                0,
                wide(&trigger_label(&binding.trigger)).as_ptr() as isize,
            );
        }
    }
    unsafe fn load_editor(&mut self, hwnd: HWND) {
        self.loading = true;
        let binding = self.selected.and_then(|i| self.draft.triggers.get(i));
        for id in [
            TRIGGER,
            TAP,
            CLICK,
            ACTION,
            ACTION + 1,
            ACTION + 2,
            ACTION + 3,
            REMOVE,
        ] {
            EnableWindow(control(hwnd, id), i32::from(binding.is_some()));
        }
        if let Some(binding) = binding {
            set(control(hwnd, CLICK), &action_label(&binding.click_action));
            set(control(hwnd, TRIGGER), &trigger_label(&binding.trigger));
            SendMessageW(
                control(hwnd, TAP),
                BM_SETCHECK,
                usize::from(binding.tap_mode == TapMode::TapPassthrough),
                0,
            );
            for (i, action) in [&binding.left, &binding.right, &binding.up, &binding.down]
                .iter()
                .enumerate()
            {
                set(control(hwnd, ACTION + i), &action_label(action));
            }
        } else {
            set(control(hwnd, CLICK), "");
            set(control(hwnd, TRIGGER), "");
            for i in 0..4 {
                set(control(hwnd, ACTION + i), "");
            }
            SendMessageW(control(hwnd, TAP), BM_SETCHECK, 0, 0);
        }
        SendMessageW(
            control(hwnd, LIST),
            LB_SETCURSEL,
            self.selected.unwrap_or(usize::MAX),
            0,
        );
        self.loading = false;
    }
    unsafe fn commit_editor(&mut self, hwnd: HWND) -> Result<(), String> {
        let Some(i) = self.selected else {
            return Ok(());
        };
        let mut binding = self.draft.triggers[i].clone();
        binding.click_action =
            action_value(&text(control(hwnd, CLICK))).map_err(|e| format!("Click action: {e}"))?;
        binding.trigger = trigger_value(&text(control(hwnd, TRIGGER)));
        binding.tap_mode =
            if SendMessageW(control(hwnd, TAP), BM_GETCHECK, 0, 0) == BST_CHECKED as isize {
                TapMode::TapPassthrough
            } else {
                TapMode::GestureOnly
            };
        for (direction, field) in [
            &mut binding.left,
            &mut binding.right,
            &mut binding.up,
            &mut binding.down,
        ]
        .into_iter()
        .enumerate()
        {
            *field = action_value(&text(control(hwnd, ACTION + direction))).map_err(|error| {
                format!(
                    "{} action: {error}",
                    ["Left", "Right", "Up", "Down"][direction]
                )
            })?;
        }
        let mut candidate = self.draft.clone();
        candidate.triggers[i] = binding;
        candidate.compile()?;
        self.draft = candidate;
        Ok(())
    }
    unsafe fn save(&mut self, hwnd: HWND) -> Result<(), String> {
        self.commit_editor(hwnd)?;
        let mut candidate = self.draft.clone();
        candidate.threshold_px = text(control(hwnd, DISTANCE))
            .trim()
            .parse()
            .map_err(|_| "Enter a gesture distance from 1 to 2000 pixels".to_string())?;
        candidate.enabled =
            SendMessageW(control(hwnd, ENABLED), BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        candidate.compile()?;
        let bytes = config::save(&self.path, &candidate, &self.original)?;
        crate::tray::apply_settings(candidate.clone());
        self.original = bytes;
        self.draft = candidate;
        self.dirty = false;
        self.loading = true;
        self.refresh_list(hwnd);
        self.load_editor(hwnd);
        set(
            control(hwnd, 220),
            "Saved. Gestures use these settings now.",
        );
        Ok(())
    }
    unsafe fn command(&mut self, hwnd: HWND, id: usize, notification: u16) {
        if self.loading {
            return;
        }
        let result = match id {
            SAVE => self.save(hwnd),
            ADD => self.commit_editor(hwnd).and_then(|()| {
                if self.draft.triggers.len() >= 64 {
                    return Err("At most 64 buttons / keys are supported".into());
                }
                let available = ["XButton1".to_string(), "XButton2".to_string()]
                    .into_iter()
                    .chain((13..=24).map(|n| format!("F{n}")))
                    .chain((1..=12).map(|n| format!("F{n}")))
                    .chain(('A'..='Z').map(|c| c.to_string()))
                    .find(|value| {
                        let mut candidate = self.draft.clone();
                        candidate.triggers.push(Binding {
                            trigger: value.clone(),
                            tap_mode: TapMode::TapPassthrough,
                            click_action: None,
                            left: None,
                            right: None,
                            up: None,
                            down: None,
                        });
                        candidate.compile().is_ok()
                    })
                    .ok_or("No unused suggested key; edit an existing trigger first")?;
                self.draft.triggers.push(Binding {
                    trigger: available,
                    tap_mode: TapMode::TapPassthrough,
                    click_action: None,
                    left: None,
                    right: None,
                    up: None,
                    down: None,
                });
                self.selected = Some(self.draft.triggers.len() - 1);
                self.loading = true;
                self.refresh_list(hwnd);
                self.load_editor(hwnd);
                self.dirty = true;
                Ok(())
            }),
            REMOVE => {
                if let Some(i) = self.selected {
                    self.draft.triggers.remove(i);
                    self.selected = if self.draft.triggers.is_empty() {
                        None
                    } else {
                        Some(i.min(self.draft.triggers.len() - 1))
                    };
                    self.loading = true;
                    self.refresh_list(hwnd);
                    self.load_editor(hwnd);
                    self.dirty = true;
                }
                Ok(())
            }
            LIST if notification == LBN_SELCHANGE as u16 => {
                let next = SendMessageW(control(hwnd, LIST), LB_GETCURSEL, 0, 0);
                match self.commit_editor(hwnd) {
                    Ok(()) => {
                        self.selected = (next >= 0).then_some(next as usize);
                        self.load_editor(hwnd);
                        Ok(())
                    }
                    Err(error) => {
                        SendMessageW(
                            control(hwnd, LIST),
                            LB_SETCURSEL,
                            self.selected.unwrap_or(usize::MAX),
                            0,
                        );
                        Err(error)
                    }
                }
            }
            ENABLED | TAP if notification == BN_CLICKED as u16 => {
                self.dirty = true;
                set(
                    control(hwnd, 220),
                    "Unsaved changes. Choose Save and Apply to use them.",
                );
                Ok(())
            }
            DISTANCE if notification == EN_CHANGE as u16 => {
                self.dirty = true;
                set(
                    control(hwnd, 220),
                    "Unsaved changes. Choose Save and Apply to use them.",
                );
                Ok(())
            }
            TRIGGER | CLICK | ACTION..=213
                if notification == CBN_EDITCHANGE as u16
                    || notification == CBN_SELCHANGE as u16 =>
            {
                self.dirty = true;
                set(
                    control(hwnd, 220),
                    "Unsaved changes. Choose Save and Apply to use them.",
                );
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            set(control(hwnd, 220), &error);
        }
    }
}
pub unsafe fn show(owner: HWND, config: Config, path: PathBuf) -> Result<(), String> {
    crate::logging::log("Settings requested");
    let existing = FindWindowW(wide(CLASS).as_ptr(), std::ptr::null());
    if !existing.is_null() {
        ShowWindow(existing, SW_RESTORE);
        SetForegroundWindow(existing);
        return Ok(());
    }
    let instance = GetModuleHandleW(std::ptr::null());
    let original = fs::read(&path).map_err(|e| e.to_string())?;
    let loaded = serde_json::from_slice::<Config>(&original)
        .ok()
        .filter(|draft| draft.compile().is_ok());
    let recovered = loaded.is_none();
    let mut draft = loaded.unwrap_or_else(|| config.clone());
    draft.enabled = config.enabled;
    let name = wide(CLASS);
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: name.as_ptr(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        ..std::mem::zeroed()
    };
    if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        return Err("Unable to register settings window".into());
    }
    let dpi = GetDpiForWindow(owner).max(96);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 700 * dpi as i32 / 96,
        bottom: 480 * dpi as i32 / 96,
    };
    AdjustWindowRectExForDpi(&mut rect, style, 0, WS_EX_APPWINDOW, dpi);
    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        name.as_ptr(),
        wide("GestureLite Settings").as_ptr(),
        style,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        rect.right - rect.left,
        rect.bottom - rect.top,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return Err("Unable to create settings window".into());
    }
    let state = Box::new(State {
        draft,
        path: path.clone(),
        original,
        selected: None,
        dirty: false,
        loading: true,
        controls: Vec::new(),
        font: std::ptr::null_mut(),
        dpi: GetDpiForWindow(hwnd).max(96),
    });
    let ptr = Box::into_raw(state);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
    WINDOW.with(|window| window.set(hwnd));
    if let Err(error) = (*ptr).create(hwnd) {
        DestroyWindow(hwnd);
        return Err(error);
    }
    if recovered {
        set(
            control(hwnd, 220),
            "JSON was invalid. Review these settings before saving; a backup will be kept.",
        );
    }
    ShowWindow(hwnd, SW_SHOWNORMAL);
    SetForegroundWindow(hwnd);
    crate::logging::log("Settings window ready");
    Ok(())
}
pub unsafe fn destroy() {
    let hwnd = WINDOW.with(Cell::get);
    if !hwnd.is_null() {
        DestroyWindow(hwnd);
    }
}
pub unsafe fn dialog_message(message: &MSG) -> bool {
    let hwnd = WINDOW.with(Cell::get);
    !hwnd.is_null() && IsDialogMessageW(hwnd, message) != 0
}
pub unsafe fn close() -> bool {
    let hwnd = WINDOW.with(Cell::get);
    if !hwnd.is_null() {
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    WINDOW.with(Cell::get).is_null()
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    match msg {
        WM_COMMAND if (*ptr).loading => 0,
        WM_COMMAND if w & 0xffff == CLOSE || w & 0xffff == IDCANCEL as usize => {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            0
        }
        WM_COMMAND if w & 0xffff == IDOK as usize => {
            (*ptr).command(hwnd, SAVE, 0);
            0
        }
        WM_COMMAND => {
            (*ptr).command(hwnd, w & 0xffff, ((w >> 16) & 0xffff) as u16);
            0
        }
        WM_CLOSE => {
            if (*ptr).dirty {
                match message(
                    hwnd,
                    "Save your changes before closing?",
                    MB_YESNOCANCEL | MB_ICONQUESTION,
                ) {
                    IDYES => {
                        if let Err(error) = (*ptr).save(hwnd) {
                            set(control(hwnd, 220), &error);
                            return 0;
                        }
                    }
                    IDNO => {}
                    _ => return 0,
                }
            }
            DestroyWindow(hwnd);
            0
        }
        WM_DPICHANGED => {
            (*ptr).dpi = (w & 0xffff) as u32;
            let rect = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            (*ptr).layout();
            0
        }
        WM_NCDESTROY => {
            WINDOW.with(|window| window.set(std::ptr::null_mut()));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let state = Box::from_raw(ptr);
            if !state.font.is_null() {
                DeleteObject(state.font);
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_and_custom_shortcuts_roundtrip_without_changing_mappings() {
        for (label, chord) in PRESETS {
            let parsed = action_value(label).unwrap();
            assert_eq!(parsed.as_deref().unwrap_or(""), *chord);
            assert_eq!(action_value(&action_label(&parsed)).unwrap(), parsed);
        }
        assert_eq!(
            action_value("ctrl + shift + s").unwrap(),
            Some("CTRL+SHIFT+S".into())
        );
        assert!(action_value("CTRL+SHIFT").is_err());
        assert!(action_value("CTRL+C+V").is_err());
        let mut config = Config::default();
        let before = serde_json::to_value(&config).unwrap();
        config.triggers[1].click_action = action_value("ctrl + shift + s").unwrap();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["triggers"][1]["click_action"], "CTRL+SHIFT+S");
        for direction in ["left", "right", "up", "down"] {
            assert_eq!(
                json["triggers"][1][direction],
                before["triggers"][1][direction]
            );
        }
        assert_eq!(
            action_value(&action_label(&config.triggers[1].click_action)).unwrap(),
            config.triggers[1].click_action
        );
        config.triggers[1].click_action = action_value("No action").unwrap();
        assert_eq!(serde_json::to_value(&config).unwrap(), before);
        for key in ["XButton1", "XButton2", "F13", "F24", "A", "VK_0x70"] {
            let label = trigger_label(key);
            assert_eq!(
                trigger_value(&label).to_ascii_uppercase(),
                key.to_ascii_uppercase()
            );
        }
    }
}
