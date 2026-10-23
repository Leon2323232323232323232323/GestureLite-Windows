use crate::{
    actions::{Action, HoldPermit, Job},
    config::Compiled,
    gesture::{is_own, Hold, Trigger},
    logging::log,
};
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::*,
};
const COMMAND: u32 = WM_APP + 10;
fn index(t: Trigger) -> usize {
    match t {
        Trigger::Key(k) => k as usize,
        Trigger::XButton(b) => 255 + b as usize,
    }
}
pub struct Engine {
    pub config: Compiled,
    holds: Vec<Hold>,
    draining: [bool; 258],
    hold_generations: Vec<Arc<AtomicU64>>,
}
impl Engine {
    pub fn new(config: Compiled) -> Self {
        let holds = vec![Hold::Idle; config.bindings.len()];
        let hold_generations = config
            .bindings
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect();
        Self {
            config,
            holds,
            draining: [false; 258],
            hold_generations,
        }
    }
    pub fn reset(&mut self) {
        crate::logging::trace("reset", 0, 0, 0);
        for ((b, h), token) in self
            .config
            .bindings
            .iter()
            .zip(&mut self.holds)
            .zip(&self.hold_generations)
        {
            if h.active() {
                token.fetch_add(1, Ordering::AcqRel);
                self.draining[index(b.trigger)] = true;
            }
            *h = Hold::Idle;
        }
    }
    pub fn replace(&mut self, config: Compiled) {
        self.reset();
        self.holds = vec![Hold::Idle; config.bindings.len()];
        self.hold_generations = config
            .bindings
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect();
        self.config = config;
    }
    pub fn button(&mut self, t: Trigger, down: bool, point: (i32, i32)) -> (bool, Option<Action>) {
        let binding = self.config.bindings.iter().position(|b| b.trigger == t);
        let state = binding.map_or(0, |i| match self.holds[i] {
            Hold::Idle => 0,
            Hold::Armed(_) => 1,
            Hold::Consumed => 2,
        });
        crate::logging::trace(
            "button",
            index(t) as i64,
            i64::from(down),
            state
                | (i64::from(self.draining[index(t)]) << 2)
                | (i64::from(self.config.enabled) << 3),
        );
        if self.draining[index(t)] {
            if !down {
                self.draining[index(t)] = false;
            }
            return (true, None);
        }
        let Some(i) = binding else {
            return (false, None);
        };
        if down {
            if !self.config.enabled {
                return (false, None);
            }
            if !self.holds[i].active() {
                self.hold_generations[i].fetch_add(1, Ordering::AcqRel);
            }
            self.holds[i].down(point);
            (true, None)
        } else {
            if !self.holds[i].active() {
                return (false, None);
            }
            self.hold_generations[i].fetch_add(1, Ordering::AcqRel);
            let binding = &self.config.bindings[i];
            let tap = self.holds[i].release(binding.passthrough || binding.click_action.is_some());
            (
                true,
                tap.then(|| binding.click_action.map_or(Action::Tap(t), Action::Chord)),
            )
        }
    }
    #[cfg(test)]
    pub fn movement(&mut self, point: (i32, i32), mut dispatch: impl FnMut(Action)) {
        self.movement_jobs(point, |action, _| dispatch(action));
    }
    pub fn movement_jobs(
        &mut self,
        point: (i32, i32),
        mut dispatch: impl FnMut(Action, HoldPermit),
    ) {
        if !self.config.enabled {
            return;
        }
        for ((b, h), token) in self
            .config
            .bindings
            .iter()
            .zip(&mut self.holds)
            .zip(&self.hold_generations)
        {
            if let Some(d) = h.movement(point, self.config.threshold, self.config.ratio) {
                crate::logging::trace(
                    "recognized",
                    index(b.trigger) as i64,
                    d.index() as i64,
                    i64::from(b.actions[d.index()].is_some()),
                );
                if let Some(c) = b.actions[d.index()] {
                    dispatch(
                        Action::Chord(c),
                        HoldPermit {
                            generation: token.clone(),
                            expected: token.load(Ordering::Acquire),
                        },
                    );
                }
            }
        }
    }
}
struct Context {
    engine: Engine,
    actions: SyncSender<Job>,
    commands: mpsc::Receiver<Command>,
    failures: Arc<AtomicU64>,
    epoch: u64,
}
thread_local! {static CONTEXT:RefCell<Option<Context>>=const {RefCell::new(None)};}
enum Command {
    Config(Compiled, u64),
    Reset(u64),
    Stop,
}
pub struct Hooks {
    sender: Sender<Command>,
    id: u32,
    thread: Option<JoinHandle<()>>,
    pub failures: Arc<AtomicU64>,
    generation: Arc<AtomicU64>,
}
impl Hooks {
    pub fn start(
        config: Compiled,
        actions: SyncSender<Job>,
        generation: Arc<AtomicU64>,
    ) -> Result<Self, String> {
        let (sender, commands) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let failures = Arc::new(AtomicU64::new(0));
        let f = failures.clone();
        let thread = thread::spawn(move || unsafe {
            let mut msg = std::mem::zeroed::<MSG>();
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            CONTEXT.with(|c| {
                *c.borrow_mut() = Some(Context {
                    engine: Engine::new(config),
                    actions,
                    commands,
                    failures: f,
                    epoch: 0,
                })
            });
            let instance = GetModuleHandleW(std::ptr::null());
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), instance, 0);
            let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), instance, 0);
            if mouse.is_null() || keyboard.is_null() {
                let e = GetLastError();
                if !mouse.is_null() {
                    UnhookWindowsHookEx(mouse);
                }
                if !keyboard.is_null() {
                    UnhookWindowsHookEx(keyboard);
                }
                let _ = ready_tx.send(Err(format!("Hook installation failed: {e}")));
                CONTEXT.with(|c| *c.borrow_mut() = None);
                return;
            }
            let _ = ready_tx.send(Ok(GetCurrentThreadId()));
            loop {
                let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                if msg.message == COMMAND {
                    let stop = CONTEXT.with(|c| {
                        let mut c = c.borrow_mut();
                        let c = c.as_mut().unwrap();
                        let mut stop = false;
                        while let Ok(cmd) = c.commands.try_recv() {
                            match cmd {
                                Command::Config(v, epoch) => {
                                    c.engine.replace(v);
                                    c.epoch = epoch;
                                }
                                Command::Reset(epoch) => {
                                    c.engine.reset();
                                    c.epoch = epoch;
                                }
                                Command::Stop => stop = true,
                            }
                        }
                        stop
                    });
                    if stop {
                        break;
                    }
                } else {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            UnhookWindowsHookEx(mouse);
            UnhookWindowsHookEx(keyboard);
            CONTEXT.with(|c| *c.borrow_mut() = None);
        });
        match ready_rx.recv().map_err(|e| e.to_string())? {
            Ok(id) => Ok(Self {
                sender,
                id,
                thread: Some(thread),
                failures,
                generation,
            }),
            Err(e) => {
                let _ = thread.join();
                Err(e)
            }
        }
    }
    fn send(&self, c: Command) {
        if self.sender.send(c).is_ok() {
            unsafe {
                if PostThreadMessageW(self.id, COMMAND, 0, 0) == 0 {
                    log("Unable to wake hook thread");
                }
            }
        }
    }
    pub fn configure(&self, c: Compiled) {
        let epoch = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.send(Command::Config(c, epoch));
    }
    pub fn reset(&self) {
        let epoch = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.send(Command::Reset(epoch));
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let n = self.failures.load(Ordering::Relaxed);
        if n != 0 {
            log(&format!(
                "Action queue failed {n} times; affected gestures were consumed without a tap"
            ));
        }
    }
}
fn enqueue(
    tx: &SyncSender<Job>,
    failures: &AtomicU64,
    epoch: u64,
    a: Action,
    permit: Option<HoldPermit>,
) {
    if tx
        .try_send(Job {
            action: a,
            epoch,
            permit,
        })
        .is_err()
    {
        crate::logging::trace("queue-full-or-closed", epoch as i64, 0, 0);
        failures.fetch_add(1, Ordering::Relaxed);
    }
}
unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let e = &*(l as *const MSLLHOOKSTRUCT);
        if matches!(w as u32, WM_XBUTTONDOWN | WM_XBUTTONUP) {
            crate::logging::trace(
                "mouse-button-meta",
                i64::from(e.flags),
                e.dwExtraInfo as i64,
                w as i64,
            );
        }
        if !is_own(e.dwExtraInfo) {
            let suppress = CONTEXT.with(|c| {
                let Ok(mut c) = c.try_borrow_mut() else {
                    return false;
                };
                let Some(c) = c.as_mut() else { return false };
                let p = (e.pt.x, e.pt.y);
                match w as u32 {
                    WM_MOUSEMOVE => {
                        crate::logging::trace_move(p.0, p.1);
                        let tx = &c.actions;
                        let f = &c.failures;
                        c.engine
                            .movement_jobs(p, |a, permit| enqueue(tx, f, c.epoch, a, Some(permit)));
                        false
                    }
                    WM_XBUTTONDOWN | WM_XBUTTONUP => {
                        let b = (e.mouseData >> 16) as u16;
                        if !(1..=2).contains(&b) {
                            return false;
                        }
                        let (s, a) =
                            c.engine
                                .button(Trigger::XButton(b), w as u32 == WM_XBUTTONDOWN, p);
                        if let Some(a) = a {
                            enqueue(&c.actions, &c.failures, c.epoch, a, None);
                        }
                        s
                    }
                    _ => false,
                }
            });
            if suppress {
                return 1;
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, w, l)
}
unsafe extern "system" fn keyboard_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let e = &*(l as *const KBDLLHOOKSTRUCT);
        // LLKHF_INJECTED is deliberately accepted: G HUB can inject trigger keys.
        if !is_own(e.dwExtraInfo)
            && e.vkCode <= 254
            && matches!(
                w as u32,
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            )
        {
            let suppress = CONTEXT.with(|c| {
                let Ok(mut c) = c.try_borrow_mut() else {
                    return false;
                };
                let Some(c) = c.as_mut() else { return false };
                let t = Trigger::Key(e.vkCode as u16);
                if !c.engine.config.bindings.iter().any(|b| b.trigger == t)
                    && !c.engine.draining[index(t)]
                {
                    return false;
                }
                let mut pt = POINT { x: 0, y: 0 };
                let down = matches!(w as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                if down && GetCursorPos(&mut pt) == 0 {
                    return false;
                }
                let (s, a) = c.engine.button(t, down, (pt.x, pt.y));
                if let Some(a) = a {
                    enqueue(&c.actions, &c.failures, c.epoch, a, None);
                }
                s
            });
            if suppress {
                return 1;
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, w, l)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    fn engine() -> Engine {
        Engine::new(Config::default().compile().unwrap())
    }
    fn click_engine(binding: usize) -> Engine {
        let mut config = Config::default();
        config.triggers[binding].click_action = Some("CTRL+SHIFT+S".into());
        Engine::new(config.compile().unwrap())
    }
    #[test]
    fn click_action_runs_once_on_release_for_mouse_and_keyboard() {
        for (binding, trigger) in [(1, Trigger::XButton(2)), (2, Trigger::Key(0x7c))] {
            let mut e = click_engine(binding);
            for _ in 0..1000 {
                assert!(e.button(trigger, true, (0, 0)).1.is_none());
                assert!(e.button(trigger, true, (0, 0)).1.is_none());
                e.movement_jobs((17, 0), |_, _| panic!("Below threshold"));
                let action = e.button(trigger, false, (17, 0)).1;
                assert!(
                    matches!(action, Some(Action::Chord(c)) if c == crate::config::chord("CTRL+SHIFT+S").unwrap())
                );
                assert!(e.button(trigger, false, (17, 0)).1.is_none());
                e.movement_jobs((100, 0), |_, _| panic!("Movement after release"));
            }
        }
    }
    #[test]
    fn gesture_and_unmapped_gesture_both_suppress_click_action() {
        for mapped in [true, false] {
            let mut e = click_engine(1);
            if !mapped {
                e.config.bindings[1].actions = [None; 4];
            }
            e.button(Trigger::XButton(2), true, (0, 0));
            let mut count = 0;
            e.movement_jobs((18, 0), |_, _| count += 1);
            e.movement_jobs((-100, 0), |_, _| panic!("Second gesture"));
            assert_eq!(count, usize::from(mapped));
            assert!(e.button(Trigger::XButton(2), false, (-100, 0)).1.is_none());
        }
    }
    #[test]
    fn reset_disable_and_config_replace_do_not_fire_click_actions() {
        for operation in 0..3 {
            let mut e = click_engine(1);
            let trigger = Trigger::XButton(2);
            e.button(trigger, true, (0, 0));
            match operation {
                0 => e.reset(),
                _ => {
                    let mut config = e.config.clone();
                    config.enabled = operation != 1;
                    e.replace(config);
                }
            }
            assert!(e.button(trigger, false, (0, 0)).1.is_none());
            if operation == 1 {
                assert!(!e.button(trigger, true, (0, 0)).0);
                assert!(e.button(trigger, false, (0, 0)).1.is_none());
            }
        }
    }
    #[test]
    fn released_click_passes_through_and_never_arms_later_motion() {
        let mut e = engine();
        let trigger = Trigger::XButton(2);
        e.button(trigger, true, (0, 0));
        assert!(matches!(e.button(trigger, false, (0, 0)).1, Some(Action::Tap(t)) if t == trigger));
        for x in -1000..=1000 {
            e.movement_jobs((x, x), |_, _| panic!("Released click armed a gesture"));
        }
    }
    #[test]
    fn tap_then_new_hold_rearms_even_with_the_tap_still_queued() {
        let mut e = engine();
        let trigger = Trigger::XButton(2);
        for cycle in 0..10_000 {
            let origin = (cycle * 100, 0);
            e.button(trigger, true, origin);
            let tap = Job {
                action: e.button(trigger, false, origin).1.expect("Normal tap"),
                epoch: 0,
                permit: None,
            };
            assert!(matches!(tap.action, Action::Tap(t) if t == trigger));
            e.button(trigger, true, origin);
            let mut jobs = Vec::new();
            e.movement_jobs((origin.0 + 18, 0), |action, permit| {
                jobs.push(Job {
                    action,
                    epoch: 0,
                    permit: Some(permit),
                });
            });
            assert_eq!(jobs.len(), 1);
            assert!(jobs[0].is_current(0));
            assert!(tap.is_current(0));
            assert!(e.button(trigger, false, (origin.0 + 18, 0)).1.is_none());
            assert!(!jobs[0].is_current(0));
        }
    }
    #[test]
    fn release_cancels_waiting_gesture_and_repress_cannot_revive_it() {
        let mut e = engine();
        let trigger = Trigger::XButton(2);
        e.button(trigger, true, (0, 0));
        let mut jobs = Vec::new();
        e.movement_jobs((18, 0), |action, permit| {
            jobs.push(Job {
                action,
                epoch: 0,
                permit: Some(permit),
            })
        });
        assert_eq!(jobs.len(), 1);
        assert!(jobs[0].is_current(0));
        assert!(!jobs[0].is_current(1));
        for x in 19..1000 {
            e.movement_jobs((x, 0), |_, _| panic!("Repeated gesture in one press"));
        }
        assert!(e.button(trigger, false, (1000, 0)).1.is_none());
        assert!(!jobs[0].is_current(0));
        e.button(trigger, true, (1000, 0));
        assert!(!jobs[0].is_current(0));
        e.movement_jobs((982, 0), |action, permit| {
            jobs.push(Job {
                action,
                epoch: 0,
                permit: Some(permit),
            })
        });
        assert_eq!(jobs.len(), 2);
        assert!(jobs[1].is_current(0));
    }
    #[test]
    fn releases_are_independent_and_reset_invalidates_remaining_gestures() {
        let mut e = engine();
        let back = Trigger::XButton(1);
        let forward = Trigger::XButton(2);
        e.button(back, true, (0, 0));
        e.button(forward, true, (0, 0));
        let mut permits = Vec::new();
        e.movement_jobs((18, 0), |_, permit| permits.push(permit));
        assert_eq!(permits.len(), 2);
        e.button(forward, false, (18, 0));
        assert!(permits[0].is_current());
        assert!(!permits[1].is_current());
        e.reset();
        assert!(!permits[0].is_current());
    }
    #[test]
    fn immediate_and_consumed() {
        let mut e = engine();
        assert!(e.button(Trigger::XButton(2), true, (0, 0)).0);
        let mut jobs = vec![];
        e.movement((18, 0), |a| jobs.push(a));
        assert_eq!(jobs.len(), 1);
        e.movement((100, 0), |a| jobs.push(a));
        assert_eq!(jobs.len(), 1);
        assert!(e.button(Trigger::XButton(2), false, (100, 0)).1.is_none());
    }
    #[test]
    fn independent_holds() {
        let mut e = engine();
        e.button(Trigger::XButton(1), true, (0, 0));
        e.button(Trigger::Key(0x7c), true, (10, 0));
        let mut jobs = vec![];
        e.movement((18, 0), |a| jobs.push(a));
        assert_eq!(jobs.len(), 1);
        e.movement((28, 0), |a| jobs.push(a));
        assert_eq!(jobs.len(), 2);
    }
    #[test]
    fn passthrough_and_repeat() {
        let mut e = engine();
        e.button(Trigger::XButton(1), true, (0, 0));
        assert!(matches!(
            e.button(Trigger::XButton(1), false, (0, 0)).1,
            Some(Action::Tap(_))
        ));
        assert!(e.button(Trigger::XButton(1), false, (0, 0)).1.is_none());
    }
    #[test]
    fn disabled_and_drain() {
        let mut e = engine();
        e.button(Trigger::XButton(1), true, (0, 0));
        let mut c = e.config.clone();
        c.enabled = false;
        e.replace(c);
        assert!(e.button(Trigger::XButton(1), false, (0, 0)).0);
        assert!(!e.button(Trigger::XButton(1), true, (0, 0)).0);
        assert!(!e.button(Trigger::XButton(1), false, (0, 0)).0);
    }
    #[test]
    fn reset_cancels_tap() {
        let mut e = engine();
        e.button(Trigger::Key(0x7c), true, (0, 0));
        e.reset();
        assert!(e.button(Trigger::Key(0x7c), false, (0, 0)).1.is_none());
        assert!(e.button(Trigger::Key(0x7c), true, (0, 0)).0);
    }
    #[test]
    fn null_mapping_consumes() {
        let mut e = engine();
        e.button(Trigger::Key(0x7d), true, (0, 0));
        let mut jobs = vec![];
        e.movement((18, 0), |a| jobs.push(a));
        assert!(jobs.is_empty());
        assert!(e.button(Trigger::Key(0x7d), false, (0, 0)).1.is_none());
    }
}
