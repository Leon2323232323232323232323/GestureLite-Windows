use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();
struct TraceEvent {
    micros: u128,
    kind: &'static str,
    a: i64,
    b: i64,
    c: i64,
}
static TRACE: OnceLock<(mpsc::SyncSender<TraceEvent>, Instant)> = OnceLock::new();
static TRACING: AtomicBool = AtomicBool::new(false);
static MOVES: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);

// Opt-in, ten-minute diagnostic session. No file I/O or formatting in hooks;
// a full diagnostic queue drops records rather than waiting on the logger.
pub fn start_trace(dir: &std::path::Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(dir.join("gesture-diagnostics.log"))?;
    writeln!(
        file,
        "GestureLite diagnostic session; elapsed microseconds; expires after 600 seconds"
    )?;
    let (tx, rx) = mpsc::sync_channel::<TraceEvent>(256);
    let start = Instant::now();
    std::thread::Builder::new()
        .name("gesture-diagnostics".into())
        .spawn(move || {
            while let Some(remaining) = Duration::from_secs(600).checked_sub(start.elapsed()) {
                let Ok(e) = rx.recv_timeout(remaining) else {
                    break;
                };
                let _ = writeln!(file, "{} {} {} {} {}", e.micros, e.kind, e.a, e.b, e.c);
            }
            TRACING.store(false, Ordering::Relaxed);
            let _ = writeln!(
                file,
                "Session ended; dropped records={}",
                DROPPED.load(Ordering::Relaxed)
            );
        })?;
    let _ = TRACE.set((tx, start));
    TRACING.store(true, Ordering::Relaxed);
    Ok(())
}
pub fn trace(kind: &'static str, a: i64, b: i64, c: i64) {
    if !TRACING.load(Ordering::Relaxed) {
        return;
    }
    if let Some((tx, start)) = TRACE.get() {
        if tx
            .try_send(TraceEvent {
                micros: start.elapsed().as_micros(),
                kind,
                a,
                b,
                c,
            })
            .is_err()
        {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub fn trace_move(x: i32, y: i32) {
    if TRACING.load(Ordering::Relaxed) && MOVES.fetch_add(1, Ordering::Relaxed) & 63 == 0 {
        trace("mouse-sample", i64::from(x), i64::from(y), 0);
    }
}
pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let _ = LOG.set(Mutex::new(dir.join("gesturelite.log")));
}
pub fn log(message: &str) {
    let Some(path) = LOG.get() else { return };
    let Ok(path) = path.lock() else { return };
    if fs::metadata(&*path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = fs::rename(&*path, path.with_extension("previous.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&*path) {
        let _ = writeln!(
            f,
            "{} {message}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
    }
}
