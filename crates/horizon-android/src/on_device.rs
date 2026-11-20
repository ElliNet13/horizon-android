//! Preparing the game files on the device (stage 1: the Java runtime). Fetches Java 25 from
//! Termux's repository and checks it starts; the preparer itself follows.

use crate::{preload, provision, wake};
use std::path::Path;

/// The no-prepared-files screen's "Prepare on this device".
pub fn prepare(root: &Path, progress: &dyn Fn(&str, Option<f32>)) -> Result<String, String> {
    let base = root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or(Path::new("."));
    // Java needs symbolic links and files the system may load, which the shared-storage game
    // folder cannot hold (symlinks fail there with "permission denied"): use the app's private
    // storage, and delete it afterwards.
    let prefix = crate::entry::APP
        .get()
        .and_then(|app| app.internal_data_path())
        .unwrap_or_else(|| base.to_path_buf())
        .join("java-runtime");
    // Keep the processor running if the player switches to another app meanwhile.
    let _awake = crate::entry::APP
        .get()
        .map(wake::acquire)
        .transpose()
        .unwrap_or_else(|e| {
            log::warn!("no wake lock: {e}");
            None
        });
    // `.installed` is left only by an install that did not finish: it carries on from there.
    let finished = !prefix.join(".installed").exists();
    let home = match provision::java_home(&prefix).filter(|_| finished) {
        Some(home) => home,
        None => provision::install_java(&prefix, progress)?,
    };
    progress("starting Java", Some(0.97));
    let preload = preload::own_path();
    // With the preload (the launcher then finds its runtime), and without it if that fails, so
    // the error says which part broke.
    let attempt = |preload: Option<&Path>| -> Result<String, String> {
        let output = provision::java_command(&prefix, &home, preload)
            .arg("-version")
            .output()
            .map_err(|e| format!("cannot start {}: {e}", provision::LOADER))?;
        let text = String::from_utf8_lossy(&[output.stdout, output.stderr].concat()).into_owned();
        let lines: Vec<_> = text.lines().take(4).collect();
        if output.status.success() {
            Ok(lines.first().copied().unwrap_or_default().to_owned())
        } else {
            Err(format!("{}: {}", output.status, lines.join(" | ")))
        }
    };
    let java = home.join("bin/java");
    let mode = std::fs::metadata(&java)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            format!("{:o}", m.permissions().mode() & 0o777)
        })
        .unwrap_or_else(|e| e.to_string());
    match attempt(preload.as_deref()) {
        Ok(first) => Ok(format!(
            "Java runs: {first}. (Preparing the game files is the next step.)"
        )),
        Err(with) => {
            log::error!("java with preload failed: {with} (java mode {mode}, preload {preload:?})");
            match attempt(None) {
                Ok(first) => Err(format!(
                    "Java only starts without the preload ({first}); with it: {with}"
                )),
                Err(without) => {
                    log::error!("java without preload failed: {without}");
                    Err(format!(
                        "Java did not start. With preload: {with}. Without: {without}. java mode {mode}"
                    ))
                }
            }
        }
    }
}
