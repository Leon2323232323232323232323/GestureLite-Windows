use crate::{
    config::{modifier, Chord},
    gesture::{Trigger, MARKER},
    logging::log,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use windows_sys::Win32::{Foundation::GetLastError, UI::Input::KeyboardAndMouse::*};
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Chord(Chord),
    Tap(Trigger),
}
#[derive(Clone, Debug)]
pub struct HoldPermit {
    pub generation: Arc<AtomicU64>,
    pub expected: u64,
}
impl HoldPermit {
    pub fn is_current(&self) -> bool {
        self.expected & 1 == 1 && self.generation.load(Ordering::Acquire) == self.expected
    }
}
#[derive(Clone, Debug)]
pub struct Job {
    pub action: Action,
    pub epoch: u64,
    pub permit: Option<HoldPermit>,
}
impl Job {
    pub fn is_current(&self, epoch: u64) -> bool {
        self.epoch == epoch && self.permit.as_ref().is_none_or(HoldPermit::is_current)
    }
}
fn extended(v: u16) -> bool {
    matches!(v,0x21..=0x2e|0x5b|0x5c|0x6f|0xa3|0xa5|0xad..=0xb3)
}
fn keyboard(v: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: v,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }
                    | if extended(v) {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        0
                    },
                time: 0,
                dwExtraInfo: MARKER,
            },
        },
    }
}
fn mouse(button: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: u32::from(button),
                dwFlags: if up {
                    MOUSEEVENTF_XUP
                } else {
                    MOUSEEVENTF_XDOWN
                },
                time: 0,
                dwExtraInfo: MARKER,
            },
        },
    }
}
fn held(v: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(v)) < 0 }
}
fn modifier_held(v: u16) -> bool {
    match v {
        0x5b | 0x5c => held(0x5b) || held(0x5c),
        _ => held(v),
    }
}
// Plan a complete chord atomically; never release modifiers already held by the user.
fn plan(c: Chord, is_held: impl Fn(u16) -> bool) -> Vec<INPUT> {
    let mut events = Vec::with_capacity(c.len * 2);
    let mut owned = [0u16; 5];
    let mut count = 0;
    for v in &c.keys[..c.len] {
        if modifier(*v) && is_held(*v) {
            continue;
        }
        owned[count] = *v;
        count += 1;
        events.push(keyboard(*v, false));
    }
    for v in owned[..count].iter().rev() {
        events.push(keyboard(*v, true));
    }
    events
}
pub fn execute(job: &Job, generation: &AtomicU64) {
    crate::logging::trace(
        "worker-job",
        i64::from(job.permit.is_some()),
        job.epoch as i64,
        generation.load(Ordering::Acquire) as i64,
    );
    // Hook-observed state is authoritative: GestureLite suppresses trigger input,
    // so asynchronous Windows key state is not a reliable trigger-down gate.
    let allowed = || job.is_current(generation.load(Ordering::Acquire));
    if !allowed() {
        crate::logging::trace("cancelled-before-plan", 0, 0, 0);
        return;
    }
    let inputs = match job.action {
        Action::Chord(c) => {
            // Do not turn a user's held target into an unintended key-up.
            if held(c.keys[c.len - 1]) {
                crate::logging::trace("target-held", i64::from(c.keys[c.len - 1]), 0, 0);
                log("Action skipped: target key already held");
                return;
            }
            plan(c, modifier_held)
        }
        Action::Tap(Trigger::Key(k)) => vec![keyboard(k, false), keyboard(k, true)],
        Action::Tap(Trigger::XButton(b)) => vec![mouse(b, false), mouse(b, true)],
    };
    // Recheck after preparing input, immediately before submitting to Windows.
    // Already submitted/in-flight input cannot be recalled by a later release.
    if !allowed() {
        crate::logging::trace("cancelled-before-send", 0, 0, 0);
        return;
    }
    unsafe {
        crate::logging::trace("send-begin", inputs.len() as i64, 0, 0);
        let sent = SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        ) as usize;
        crate::logging::trace("send-end", sent as i64, inputs.len() as i64, 0);
        if sent != inputs.len() {
            let error = GetLastError();
            // Release only unmatched downs in the successfully inserted prefix.
            let cleanup = cleanup_prefix(&inputs, sent);
            let recovered = if cleanup.is_empty() {
                true
            } else {
                SendInput(
                    cleanup.len() as u32,
                    cleanup.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                ) as usize
                    == cleanup.len()
            };
            log(&format!("SendInput inserted {sent}/{} events; Win32 error {error}; cleanup={recovered}. UIPI may block elevated targets.",inputs.len()));
            if !recovered {
                log("URGENT: key-up cleanup failed; release keys manually or unlock the desktop");
            }
        }
    }
}
fn cleanup_prefix(inputs: &[INPUT], sent: usize) -> Vec<INPUT> {
    let mut pending: Vec<INPUT> = Vec::new();
    for e in inputs.iter().take(sent) {
        unsafe {
            match e.r#type {
                INPUT_KEYBOARD => {
                    let k = e.Anonymous.ki;
                    if k.dwFlags & KEYEVENTF_KEYUP == 0 {
                        pending.push(keyboard(k.wVk, true));
                    } else {
                        pending
                            .retain(|p| p.r#type != INPUT_KEYBOARD || p.Anonymous.ki.wVk != k.wVk);
                    }
                }
                INPUT_MOUSE => {
                    let m = e.Anonymous.mi;
                    if m.dwFlags & MOUSEEVENTF_XUP == 0 {
                        pending.push(mouse(m.mouseData as u16, true));
                    } else {
                        pending.retain(|p| {
                            p.r#type != INPUT_MOUSE || p.Anonymous.mi.mouseData != m.mouseData
                        });
                    }
                }
                _ => {}
            }
        }
    }
    pending.reverse();
    pending
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::chord;
    #[test]
    fn modifier_order_and_markers() {
        let p = plan(chord("WIN+CTRL+RIGHT").unwrap(), |_| false);
        let keys: Vec<_> = p.iter().map(|i| unsafe { i.Anonymous.ki.wVk }).collect();
        assert_eq!(keys, [0x5b, 0x11, 0x27, 0x27, 0x11, 0x5b]);
        assert!(p
            .iter()
            .all(|i| unsafe { i.Anonymous.ki.dwExtraInfo == MARKER }));
    }
    #[test]
    fn held_modifier_preserved() {
        let p = plan(chord("CTRL+C").unwrap(), |k| k == 0x11);
        assert_eq!(p.len(), 2);
        assert_eq!(unsafe { p[0].Anonymous.ki.wVk }, 67);
    }
    #[test]
    fn partial_cleanup() {
        let p = plan(chord("CTRL+C").unwrap(), |_| false);
        for (sent, count) in [(0, 0), (1, 1), (2, 2), (3, 1), (4, 0)] {
            let c = cleanup_prefix(&p, sent);
            assert_eq!(c.len(), count);
            assert!(c
                .iter()
                .all(|i| unsafe { i.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0 }));
        }
    }
}
