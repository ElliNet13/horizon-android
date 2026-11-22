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
    // On a worker: the loading screen shows its bar, and the loader waits for it (`entry`).
    if std::fs::read_dir(private).is_ok_and(|mut d| d.next().is_some()) {
        horizon_client::entry::set_boot_progress("Moving game files", 0.0);
        let (from, to) = (private.to_owned(), shared.clone());
        let spawned = std::thread::Builder::new()
            .name("move-game-files".into())
            .spawn(move || {
                let tally = Tally {
                    total: files_in(&from).max(1),
                    done: std::cell::Cell::new(0),
                };
                move_in(&from, &to, &tally);
                horizon_client::entry::end_boot_progress();
            });
        if let Err(e) = spawned {
            log::warn!("could not start the move: {e}");
            horizon_client::entry::end_boot_progress();
        }
    }
    shared
}

/// How far a move is: files moved of the files to move.
struct Tally {
    total: u64,
    done: std::cell::Cell<u64>,
}

impl Tally {
    fn add(&self, files: u64) {
        self.done.set(self.done.get() + files);
        horizon_client::entry::set_boot_progress(
            "Moving game files",
            self.done.get() as f32 / self.total as f32,
        );
    }
}

/// The files under `path` (one for a file).
fn files_in(path: &Path) -> u64 {
    match std::fs::read_dir(path) {
        Ok(entries) => entries.flatten().map(|e| files_in(&e.path())).sum(),
        Err(_) => 1,
    }
}

/// Moves what an earlier start kept in `from` into `to`, folder by folder: what `to` lacks moves,
/// a folder both have is merged, a file both have stays as it is in `to`. A move inside the same
/// storage is a rename, so it is quick and leaves nothing half-copied.
fn move_in(from: &Path, to: &Path, tally: &Tally) {
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if target.exists() {
            if source.is_dir() && target.is_dir() {
                log::info!("merging {} into {}", source.display(), target.display());
                move_in(&source, &target, tally);
                let _ = std::fs::remove_dir(&source);
            } else {
                // Stays where it is: counted as done.
                tally.add(files_in(&source));
            }
            continue;
        }
        let files = files_in(&source);
        match std::fs::rename(&source, &target) {
            Ok(()) => {
                log::info!("moved {}", source.display());
                tally.add(files);
            }
            Err(e) => {
                // A rename between the private folder and Documents can be refused: copy, then
                // delete what was copied (a failed copy leaves the original where it was).
                log::warn!("rename {}: {e}; copying instead", source.display());
                match copy_all(&source, &target, tally) {
                    Ok(()) => {
                        let _ = if source.is_dir() {
                            std::fs::remove_dir_all(&source)
                        } else {
                            std::fs::remove_file(&source)
                        };
                    }
                    Err(e) => {
                        log::warn!("could not move {}: {e}", source.display());
                        let _ = std::fs::remove_dir_all(&target);
                    }
                }
            }
        }
    }
}

/// Copies a file or a whole folder (links followed).
fn copy_all(from: &Path, to: &Path, tally: &Tally) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()), tally)?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to)?;
        tally.add(1);
        Ok(())
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
