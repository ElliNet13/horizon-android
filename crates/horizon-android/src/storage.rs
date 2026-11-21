//! Where the game keeps its files. Android 11+ hides an app's own `Android/data` folder from the
//! Files app, so with "All files access" the game lives in `Documents/Horizon` instead, where the
//! Files app shows it (and a PC sees it over USB). Without the permission it keeps using the
//! app's private folder, as before; nothing is lost either way.

use jni::JavaVM;
use jni::objects::{JObject, JString, JValue};
use std::path::{Path, PathBuf};

/// The shared storage's root as apps see it.
const ROOT: &str = "/storage/emulated/0";

/// Whether the game folder is in the shared storage (the launcher page says so).
pub fn shared_in_use(dir: &Path) -> bool {
    dir.starts_with(ROOT) && !dir.starts_with(format!("{ROOT}/Android"))
}

/// The folder the game runs in: `Documents/Horizon` (`Horizon Debug` for the debug app) once
/// "All files access" is granted, with an earlier private folder's contents moved there the first
/// time; otherwise `private`.
pub fn game_dir(private: &Path) -> PathBuf {
    if !all_files_access() {
        return private.to_owned();
    }
    let debug = private.to_string_lossy().contains(".debug");
    let shared = Path::new(ROOT)
        .join("Documents")
        .join(if debug { "Horizon Debug" } else { "Horizon" });
    if let Err(e) = std::fs::create_dir_all(&shared) {
        log::warn!("game folder {}: {e}", shared.display());
        return private.to_owned();
    }
    move_in(private, &shared);
    shared
}

/// Moves what an earlier start kept in `from` into `to`, when `to` has none of it yet. A move
/// inside the same storage is a rename, so it is quick and leaves nothing half-copied.
fn move_in(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let target = to.join(entry.file_name());
        if target.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(entry.path(), &target) {
            log::warn!("could not move {}: {e}", entry.path().display());
        }
    }
}

/// Runs `f` with the JNI environment and the activity.
fn with_activity<T>(
    f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> Result<T, String>,
) -> Result<T, String> {
    let app = crate::entry::APP.get().ok_or("no activity yet")?;
    // SAFETY: android-activity hands out the process's JavaVM and the activity's object; both
    // stay valid for as long as the app runs.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    f(&mut env, &activity)
}

/// `Environment.isExternalStorageManager()`: the "All files access" setting (Android 11+).
pub fn all_files_access() -> bool {
    with_activity(|env, _| {
        env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])
            .and_then(|v| v.z())
            .map_err(|e| e.to_string())
    })
    .unwrap_or(false)
}

/// Opens this app's "All files access" page in Settings.
pub fn request_all_files_access() -> bool {
    let result = with_activity(|env, activity| {
        let package = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        let package: String = env
            .get_string(&JString::from(package))
            .map_err(|e| e.to_string())?
            .into();
        let address = env
            .new_string(format!("package:{package}"))
            .map_err(|e| e.to_string())?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&address)],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        let action = env
            .new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")
            .map_err(|e| e.to_string())?;
        let intent = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )
            .map_err(|e| e.to_string())?;
        env.call_method(
            activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .map_err(|e| e.to_string())?;
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err("Settings has no such page".into());
        }
        Ok(())
    });
    if let Err(e) = &result {
        log::warn!("all files access: {e}");
    }
    result.is_ok()
}
