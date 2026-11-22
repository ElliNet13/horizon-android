//! The watchdog: a second process that starts the game again after a crash, and the crash
//! screen the next start opens with.
//!
//! cargo-apk builds one activity in one process, so the watchdog is the shell (`assets/watchdog.sh`),
//! started by the game in a session of its own. The two watch each other: the script waits for the
//! game's process to end and starts the game when it left a `crashed` (or `restart`) marker; the
//! game checks on the script and starts it again when the system has killed it.
//!
//! A crash leaves its marker in two ways: the engine's crash report hook (an error or a panic,
//! with the report's file) and a signal handler (a native crash: no report).
use horizon_client::entry::CrashNotice;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const SCRIPT: &str = include_str!("../assets/watchdog.sh");

/// Left by a crash (the first marker stays: a report's, before the signal its exit raises): the first line says what ended the game, the rest is for the crash screen.
const CRASHED: &str = "crashed";
/// Left by the screen's Restart button.
const RESTART: &str = "restart";
/// The most times the game starts the watchdog again, so a script that cannot run does not
/// spawn for ever.
const MAX_RESPAWNS: u32 = 20;

/// The folder for the markers.
static DIR: OnceLock<PathBuf> = OnceLock::new();
/// `crashed`'s path for the signal handler, made before it can run.
static SIGNAL_MARKER: OnceLock<CString> = OnceLock::new();
/// The game ended on purpose: what happens while it winds down is not a crash.
static CLEAN: AtomicBool = AtomicBool::new(false);
static WATCHDOG: Mutex<Option<Child>> = Mutex::new(None);

/// Starts the watchdog and the watch over it, and the hooks that leave the markers. `dir` is
/// where they are kept (a folder the watchdog's shell can reach, the app's own).
pub fn start(dir: &Path) {
    let dir = dir.join("watchdog");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("watchdog: no folder {}: {e}", dir.display());
        return;
    }
    let _ = DIR.set(dir.clone());
    if let Ok(path) = CString::new(dir.join(CRASHED).as_os_str().as_encoded_bytes()) {
        let _ = SIGNAL_MARKER.set(path);
    }
    horizon_client::crash::on_report(report_hook);
    install_signal_handlers();
    horizon_client::entry::set_crash_notice_hook(take_notice);
    horizon_client::entry::set_restart_hook(restart);
    if std::fs::write(dir.join("watchdog.sh"), SCRIPT).is_err() {
        log::warn!("watchdog: could not write the script");
        return;
    }
    *WATCHDOG.lock().unwrap_or_else(|e| e.into_inner()) = spawn(&dir);
    let spawned = std::thread::Builder::new()
        .name("watchdog-watch".into())
        .spawn(move || watch(&dir));
    if let Err(e) = spawned {
        log::warn!("watchdog: no watch thread: {e}");
    }
}

/// Debug: raises a signal the way a native crash does (a null write), for the signal handler.
pub fn crash_natively() {
    // SAFETY: a deliberate crash, for the debug page.
    unsafe {
        libc::raise(libc::SIGSEGV);
    }
}

/// Debug: stops the watchdog, so the game's watch over it starts it again.
pub fn kill_watchdog() {
    if let Some(child) = WATCHDOG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = child.kill();
    }
}

/// The game is leaving on purpose (its loop ended without an error).
pub fn clean_exit() {
    CLEAN.store(true, Ordering::Relaxed);
}

/// The process name, which is the package.
fn package() -> String {
    std::fs::read("/proc/self/cmdline")
        .ok()
        .and_then(|bytes| {
            let name = bytes.split(|b| *b == 0).next()?.to_vec();
            String::from_utf8(name).ok()
        })
        .unwrap_or_default()
}

fn spawn(dir: &Path) -> Option<Child> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("/system/bin/sh");
    command
        .arg(dir.join("watchdog.sh"))
        .arg(std::process::id().to_string())
        .arg(dir)
        .arg(package())
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A session of its own, so it is not the game's process group's.
    // SAFETY: `setsid` only changes the child's session, between the fork and the exec.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match command.spawn() {
        Ok(child) => {
            log::info!("watchdog: started (pid {})", child.id());
            Some(child)
        }
        Err(e) => {
            log::warn!("watchdog: could not start: {e}");
            None
        }
    }
}

/// Starts the watchdog again whenever it is gone.
fn watch(dir: &Path) {
    let mut respawns = 0;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let mut guard = WATCHDOG.lock().unwrap_or_else(|e| e.into_inner());
        let gone = match guard.as_mut() {
            Some(child) => !matches!(child.try_wait(), Ok(None)),
            None => true,
        };
        if gone {
            if respawns >= MAX_RESPAWNS {
                log::warn!("watchdog: gave up restarting it");
                return;
            }
            respawns += 1;
            log::warn!("watchdog: it was gone; starting it again");
            *guard = spawn(dir);
        }
    }
}

/// Asks the watchdog to start the game again, then ends this process (the crash screen's button).
fn restart() -> bool {
    let Some(dir) = DIR.get() else {
        return false;
    };
    if std::fs::write(dir.join(RESTART), "restart\n").is_err() {
        return false;
    }
    // A moment for the screen to close; the watchdog starts the game once this process is gone.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(300));
        std::process::exit(0);
    });
    true
}

/// What the engine's crash handler calls with each report: the marker names the report's file.
fn report_hook(game_dir: &Path, report: &horizon_client::crash::Report) {
    let Some(dir) = DIR.get() else {
        return;
    };
    let file = game_dir
        .join("crash-reports")
        .join(horizon_client::crash::file_name(&report.time, report.side));
    let error = report.error.lines().next().unwrap_or_default();
    let written = std::fs::write(
        dir.join(CRASHED),
        format!("report\n{}\n{error}\n", file.display()),
    );
    log::info!("watchdog: crash marker written: {written:?}");
}

/// The last run's crash, once (the marker goes with it).
fn take_notice() -> Option<CrashNotice> {
    let dir = DIR.get()?;
    let path = dir.join(CRASHED);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(parse_marker(&text))
}

/// The crash screen's text from a marker.
fn parse_marker(text: &str) -> CrashNotice {
    let mut lines = text.lines();
    match lines.next() {
        Some("report") => {
            let report = lines.next().map(PathBuf::from);
            let error = lines.next().unwrap_or_default();
            CrashNotice {
                summary: error.to_owned(),
                report,
            }
        }
        Some(kind) if kind.starts_with("signal ") => CrashNotice {
            summary: format!("The game's process was stopped by the system ({kind})."),
            report: None,
        },
        _ => CrashNotice {
            summary: "The game stopped unexpectedly.".into(),
            report: None,
        },
    }
}

/// What the signal handler writes for each signal the game's own code can raise.
const SIGNALS: [libc::c_int; 5] = [
    libc::SIGSEGV,
    libc::SIGABRT,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,
];

/// The handlers the system had, called after ours (they write the tombstone).
static mut PREVIOUS: [Option<libc::sigaction>; 32] = [None; 32];

fn install_signal_handlers() {
    // SAFETY: `sigaction` with a handler that only calls async-signal-safe functions (`open`,
    // `write`, `close`) and then the previous handler; `PREVIOUS` is filled here, before any of
    // the handlers can run, and only read by them.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_signal as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in SIGNALS {
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, &action, &mut previous) == 0 {
                (*std::ptr::addr_of_mut!(PREVIOUS))[signal as usize] = Some(previous);
            }
        }
    }
}

extern "C" fn on_signal(signal: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
    // SAFETY: only async-signal-safe calls, and the previous handler as the system installed it.
    unsafe {
        if !CLEAN.load(Ordering::Relaxed)
            && let Some(path) = SIGNAL_MARKER.get()
        {
            let fd = libc::open(
                path.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            );
            if fd >= 0 {
                let mut text = *b"signal 00\n";
                text[7] = b'0' + (signal / 10 % 10) as u8;
                text[8] = b'0' + (signal % 10) as u8;
                libc::write(fd, text.as_ptr().cast(), text.len());
                libc::close(fd);
            }
        }
        match (*std::ptr::addr_of!(PREVIOUS))[signal as usize] {
            Some(previous) if previous.sa_flags & libc::SA_SIGINFO != 0 => {
                let handler: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(previous.sa_sigaction);
                handler(signal, info, context);
            }
            Some(previous)
                if previous.sa_sigaction != libc::SIG_DFL
                    && previous.sa_sigaction != libc::SIG_IGN =>
            {
                let handler: extern "C" fn(libc::c_int) = std::mem::transmute(previous.sa_sigaction);
                handler(signal);
            }
            _ => {
                libc::signal(signal, libc::SIG_DFL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_marker_names_the_file_and_the_error() {
        let notice = parse_marker("report\n/games/crash-reports/crash-1.txt\nboom\n");
        assert_eq!(notice.summary, "boom");
        assert_eq!(
            notice.report.as_deref(),
            Some(Path::new("/games/crash-reports/crash-1.txt"))
        );
    }

    #[test]
    fn a_signal_marker_says_which_signal() {
        let notice = parse_marker("signal 11\n");
        assert!(notice.summary.contains("signal 11"));
        assert!(notice.report.is_none());
    }
}
