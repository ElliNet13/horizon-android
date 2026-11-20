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
    let prefix = base.join("java-runtime");
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
    let output = provision::java_command(&prefix, &home, preload.as_deref())
        .arg("-version")
        .output()
        .map_err(|e| format!("cannot start Java: {e}"))?;
    let text = String::from_utf8_lossy(&[output.stdout, output.stderr].concat()).into_owned();
    let first = text.lines().next().unwrap_or_default().to_owned();
    if output.status.success() {
        Ok(format!(
            "Java runs: {first}. (Preparing the game files is the next step.)"
        ))
    } else {
        let tail: Vec<_> = text.lines().take(4).collect();
        Err(format!(
            "Java did not start ({}): {}",
            output.status,
            tail.join(" | ")
        ))
    }
}
