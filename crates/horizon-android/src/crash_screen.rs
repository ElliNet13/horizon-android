//! After a crash: the game starts again by itself and opens on the crash screen.
//!
//! The engine's crash handler calls [`report_hook`] with each crash report. It leaves a marker
//! (what happened, and the report's file), starts the app again through its launcher intent (the
//! app is on screen then, so the system allows it) and the game's process ends with the crash.
//! The new process finds the marker and opens the crash screen first (`take_notice`). A crash that
//! cannot be caught (a signal from the graphics driver) leaves no marker and no restart.
use horizon_client::entry::CrashNotice;
use jni::JNIEnv;
use jni::objects::{JObject, JValue};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Left by a crash: the first line is `report`, then the report's file, then the error.
const CRASHED: &str = "crashed";

/// The folder for the marker.
static DIR: OnceLock<PathBuf> = OnceLock::new();
/// This start opened on the crash screen: a crash here does not start the game again (a screen
/// that crashes would otherwise start for ever).
static SHOWING_NOTICE: AtomicBool = AtomicBool::new(false);

/// Registers the hooks. `dir` is where the marker is kept (the app's own folder).
pub fn start(dir: &Path) {
    let dir = dir.join("crash");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("crash screen: no folder {}: {e}", dir.display());
        return;
    }
    let _ = DIR.set(dir);
    horizon_client::crash::on_report(report_hook);
    horizon_client::crash::set_exit(|code| {
        // SAFETY: ends the process at once, skipping the destructors that abort.
        unsafe { libc::_exit(code) }
    });
    horizon_client::entry::set_crash_notice_hook(take_notice);
    horizon_client::entry::set_restart_hook(restart);
}

/// What the engine's crash handler calls with each report.
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
    log::info!("crash screen: marker written: {written:?}");
    if !SHOWING_NOTICE.load(Ordering::Relaxed) {
        start_again();
    }
}

/// The last run's crash, once (the marker goes with it).
fn take_notice() -> Option<CrashNotice> {
    let dir = DIR.get()?;
    let path = dir.join(CRASHED);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    SHOWING_NOTICE.store(true, Ordering::Relaxed);
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
        _ => CrashNotice {
            summary: "The game stopped unexpectedly.".into(),
            report: None,
        },
    }
}

/// The crash screen's button: starts the game in a new process and ends this one.
fn restart() -> bool {
    if !start_again() {
        return false;
    }
    SHOWING_NOTICE.store(false, Ordering::Relaxed);
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(400));
        // SAFETY: ends the process at once, as intended.
        unsafe { libc::_exit(0) };
    });
    true
}

/// Starts the app's launcher activity in a new task, so the system starts a new process once this
/// one ends. True when the system took the request.
fn start_again() -> bool {
    let Some(app) = crate::entry::APP.get() else {
        return false;
    };
    // SAFETY: the process's JavaVM and the activity's object, valid while the app runs.
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return false;
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let Ok(mut env) = vm.attach_current_thread() else {
        return false;
    };
    let started = launch(&mut env, &activity).is_ok();
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    log::info!("crash screen: starting the game again: {started}");
    started
}

fn launch(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<()> {
    let manager = env
        .call_method(
            activity,
            "getPackageManager",
            "()Landroid/content/pm/PackageManager;",
            &[],
        )?
        .l()?;
    let package = env
        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;
    let intent = env
        .call_method(
            &manager,
            "getLaunchIntentForPackage",
            "(Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&package)],
        )?
        .l()?;
    // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK
    env.call_method(
        &intent,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(0x1000_0000 | 0x0000_8000)],
    )?;
    env.call_method(
        activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;
    Ok(())
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
    fn an_unknown_marker_still_gives_a_screen() {
        let notice = parse_marker("signal 11\n");
        assert!(notice.report.is_none());
        assert!(!notice.summary.is_empty());
    }
}
