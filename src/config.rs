use crate::gesture::Trigger;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub threshold_px: u32,
    pub dominance_ratio: f64,
    pub triggers: Vec<Binding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub trigger: String,
    pub tap_mode: TapMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_action: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub up: Option<String>,
    pub down: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum TapMode {
    GestureOnly,
    TapPassthrough,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chord {
    pub keys: [u16; 5],
    pub len: usize,
}
#[derive(Clone, Debug)]
pub struct CompiledBinding {
    pub trigger: Trigger,
    pub passthrough: bool,
    pub click_action: Option<Chord>,
    pub actions: [Option<Chord>; 4],
}
#[derive(Clone, Debug)]
pub struct Compiled {
    pub enabled: bool,
    pub threshold: u32,
    pub ratio: f64,
    pub bindings: Vec<CompiledBinding>,
}
pub fn key(s: &str) -> Result<u16, String> {
    let s = s.trim().to_ascii_uppercase();
    let v = match s.as_str() {
        "WIN" => 0x5b,
        "CTRL" => 0x11,
        "ALT" => 0x12,
        "SHIFT" => 0x10,
        "LEFT" => 0x25,
        "RIGHT" => 0x27,
        "UP" => 0x26,
        "DOWN" => 0x28,
        "TAB" => 9,
        "ENTER" => 13,
        "ESC" => 27,
        "SPACE" => 32,
        "DELETE" => 0x2e,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        "VOLUMEUP" => 0xaf,
        "VOLUMEDOWN" => 0xae,
        "VOLUMEMUTE" => 0xad,
        "PLAYPAUSE" => 0xb3,
        _ if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => u16::from(s.as_bytes()[0]),
        _ if s.starts_with('F') => {
            let n = s[1..]
                .parse::<u16>()
                .map_err(|_| format!("Unknown key: {s}"))?;
            if !(1..=24).contains(&n) {
                return Err(format!("Invalid function key: {s}"));
            }
            0x6f + n
        }
        _ if s.starts_with("VK_0X") => {
            u16::from_str_radix(&s[5..], 16).map_err(|_| format!("Invalid virtual key: {s}"))?
        }
        _ => return Err(format!("Unknown key: {s}")),
    };
    if v == 0 || v > 254 {
        return Err(format!("Invalid virtual key: {s}"));
    }
    Ok(v)
}
pub fn modifier(v: u16) -> bool {
    matches!(v,0x10..=0x12|0x5b|0x5c|0xa0..=0xa5)
}
fn modifier_family(v: u16) -> Option<u8> {
    match v {
        0x10 | 0xa0 | 0xa1 => Some(0),
        0x11 | 0xa2 | 0xa3 => Some(1),
        0x12 | 0xa4 | 0xa5 => Some(2),
        0x5b | 0x5c => Some(3),
        _ => None,
    }
}
pub fn chord(s: &str) -> Result<Chord, String> {
    let mut c = Chord {
        keys: [0; 5],
        len: 0,
    };
    for part in s.split('+') {
        if c.len == 5 {
            return Err("A chord supports at most four modifiers and one target".into());
        }
        let v = key(part)?;
        if c.keys[..c.len].contains(&v) {
            return Err("Duplicate chord key".into());
        }
        if let Some(family) = modifier_family(v) {
            if c.keys[..c.len]
                .iter()
                .any(|k| modifier_family(*k) == Some(family))
            {
                return Err("Use each modifier family only once".into());
            }
        }
        c.keys[c.len] = v;
        c.len += 1;
    }
    if c.len == 0
        || modifier(c.keys[c.len - 1])
        || matches!(c.keys[c.len - 1], 1..=6)
        || c.keys[..c.len - 1].iter().any(|v| !modifier(*v))
    {
        return Err("Use modifiers followed by one target key".into());
    }
    Ok(c)
}
impl Config {
    pub fn compile(&self) -> Result<Compiled, String> {
        if !(1..=2000).contains(&self.threshold_px) {
            return Err("threshold_px must be 1..2000".into());
        }
        if !self.dominance_ratio.is_finite() || !(1.01..=10.0).contains(&self.dominance_ratio) {
            return Err("dominance_ratio must be 1.01..10".into());
        }
        if self.triggers.len() > 64 {
            return Err("At most 64 triggers".into());
        }
        let mut bindings: Vec<CompiledBinding> = Vec::new();
        for b in &self.triggers {
            let trigger = match b.trigger.to_ascii_uppercase().as_str() {
                "XBUTTON1" => Trigger::XButton(1),
                "XBUTTON2" => Trigger::XButton(2),
                _ => {
                    let k = key(&b.trigger)?;
                    if modifier(k) || matches!(k, 1..=6) {
                        return Err(
                            "Modifier and mouse virtual keys cannot be keyboard triggers".into(),
                        );
                    }
                    Trigger::Key(k)
                }
            };
            if bindings.iter().any(|b| b.trigger == trigger) {
                return Err(format!("Duplicate trigger {}", b.trigger));
            }
            let mut actions = [None; 4];
            for (i, s) in [&b.left, &b.right, &b.up, &b.down].iter().enumerate() {
                actions[i] = s.as_ref().map(|s| chord(s)).transpose()?;
            }
            bindings.push(CompiledBinding {
                trigger,
                passthrough: b.tap_mode == TapMode::TapPassthrough,
                click_action: b
                    .click_action
                    .as_deref()
                    .map(chord)
                    .transpose()
                    .map_err(|e| format!("{} click action: {e}", b.trigger))?,
                actions,
            });
        }
        Ok(Compiled {
            enabled: self.enabled,
            threshold: self.threshold_px,
            ratio: self.dominance_ratio,
            bindings,
        })
    }
}
impl Default for Config {
    fn default() -> Self {
        serde_json::from_str(include_str!("../config.example.json")).expect("built-in config")
    }
}
pub fn directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("GestureLite")
}
pub fn load(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        fs::create_dir_all(path.parent().ok_or("Missing config directory")?)
            .map_err(|e| e.to_string())?;
        fs::write(path, include_str!("../config.example.json")).map_err(|e| e.to_string())?;
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let result = serde_json::from_str::<Config>(&text)
        .map_err(|e| e.to_string())
        .and_then(|c| {
            c.compile()?;
            Ok(c)
        });
    if result.is_err() {
        let backup = path.with_extension(format!(
            "invalid-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let _ = fs::copy(path, backup);
    }
    result
}
pub fn save(path: &Path, config: &Config, expected: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    config.compile()?;
    if fs::read(path).map_err(|e| e.to_string())? != expected {
        return Err("The JSON file changed outside this window. Close without saving, then reopen Settings to load it.".into());
    }
    let mut bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes == expected {
        return Ok(bytes);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = path.with_extension(format!("before-settings-{stamp}.json"));
    let temporary = path.with_extension(format!("settings-{stamp}.tmp"));
    let write_new = |file: &Path, data: &[u8]| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(file)
            .map_err(|e| e.to_string())?;
        file.write_all(data)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())
    };
    write_new(&backup, expected)?;
    let result = (|| {
        write_new(&temporary, &bytes)?;
        if fs::read(path).map_err(|e| e.to_string())? != expected {
            return Err(
                "The configuration changed while saving. Reopen Settings and try again.".into(),
            );
        }
        fs::rename(&temporary, path).map_err(|e| {
            format!("Could not replace the configuration; previous settings are preserved: {e}")
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    #[test]
    fn settings_save_preserves_backups_and_rejects_invalid_or_conflicting_edits() {
        use super::*;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "gesturelite-settings-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        let original = serde_json::to_vec_pretty(&Config::default()).unwrap();
        fs::write(&path, &original).unwrap();
        let mut candidate = Config {
            enabled: false,
            threshold_px: 720,
            ..Config::default()
        };
        candidate.triggers[1].right = Some("CTRL+SHIFT+S".into());
        candidate.triggers[1].click_action = Some("CTRL+C".into());
        let saved = save(&path, &candidate, &original).unwrap();
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(load(&path).unwrap().threshold_px, 720);
        assert!(!load(&path).unwrap().enabled);
        assert_eq!(
            load(&path).unwrap().triggers[1].click_action.as_deref(),
            Some("CTRL+C")
        );
        assert!(!load(&path).unwrap().compile().unwrap().enabled);
        assert_eq!(
            load(&path).unwrap().triggers[1].right.as_deref(),
            Some("CTRL+SHIFT+S")
        );
        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|p| p != &path)
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), original);
        candidate.threshold_px = 0;
        assert!(save(&path, &candidate, &saved).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
        candidate.threshold_px = 360;
        fs::write(&path, &original).unwrap();
        assert!(save(&path, &candidate, &saved).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        for entry in fs::read_dir(&dir).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(&dir).unwrap();
    }

    use super::*;
    #[test]
    fn defaults_valid() {
        let c = Config::default().compile().unwrap();
        assert_eq!(c.bindings.len(), 14);
    }
    #[test]
    fn old_json_preserves_defaults_and_click_actions_are_validated() {
        let mut config: Config =
            serde_json::from_str(include_str!("../config.example.json")).unwrap();
        assert!(config.triggers.iter().all(|b| b.click_action.is_none()));
        assert!(config
            .compile()
            .unwrap()
            .bindings
            .iter()
            .all(|b| b.click_action.is_none()));
        for invalid in ["CTRL", "CTRL+SHIFT", "VK_0x01", "CTRL+C+V", ""] {
            config.triggers[1].click_action = Some(invalid.into());
            assert!(config.compile().is_err(), "{invalid}");
        }
        config.triggers[1].click_action = Some("CTRL+SHIFT+S".into());
        let restored: Config =
            serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        assert_eq!(
            restored.compile().unwrap().bindings[1].click_action,
            Some(chord("CTRL+SHIFT+S").unwrap())
        );
    }
    #[test]
    fn invalid_config() {
        let mut c = Config {
            threshold_px: 0,
            ..Config::default()
        };
        assert!(c.compile().is_err());
        c.threshold_px = 18;
        c.dominance_ratio = 1.0;
        assert!(c.compile().is_err());
        c.dominance_ratio = 1.25;
        c.triggers.push(c.triggers[0].clone());
        assert!(c.compile().is_err());
    }
    #[test]
    fn chord_validation() {
        assert_eq!(
            chord("WIN+CTRL+RIGHT").unwrap().keys[..3],
            [0x5b, 0x11, 0x27]
        );
        for s in ["", "CTRL", "C+CTRL", "CTRL+CTRL+C", "BOGUS"] {
            assert!(chord(s).is_err(), "{s}");
        }
    }
    #[test]
    fn ordinary_virtual_key() {
        assert_eq!(key("VK_0x41"), Ok(65));
        assert_eq!(key("F24"), Ok(0x87));
        assert!(key("F25").is_err());
    }
    #[test]
    fn modifier_aliases_and_mouse_targets_rejected() {
        for s in [
            "CTRL+VK_0xA2+C",
            "WIN+VK_0x5C+LEFT",
            "VK_0x01",
            "ALT+VK_0x06",
        ] {
            assert!(chord(s).is_err(), "{s}");
        }
    }
    #[test]
    fn malformed_and_unknown_fields_rejected() {
        assert!(serde_json::from_str::<Config>("{broken").is_err());
        let mut c = serde_json::to_value(Config::default()).unwrap();
        c["threshold_pxx"] = serde_json::json!(18);
        assert!(serde_json::from_value::<Config>(c).is_err());
    }
}
