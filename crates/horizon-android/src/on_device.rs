//! Preparing the game files on the device (stage 1: the Java runtime). Fetches Java 25 from
//! Termux's repository and checks it starts; the preparer itself follows.

use crate::{preload, provision};
use std::path::Path;

/// The no-prepared-files screen's "Prepare on this device".
pub fn prepare(root: &Path, progress: &dyn Fn(&str)) -> Result<String, String> {
    let base = root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .unwrap_or(Path::new("."));
    let prefix = base.join("java-runtime");
    let home = match provision::java_home(&prefix) {
        Some(home) => home,
        None => provision::install_java(&prefix, progress)?,
    };
    progress("starting Java");
    let preload = preload::own_path();
    let output = provision::java_command(&prefix, &home, preload.as_deref())
        .arg("-version")
        .output()
        .map_err(|e| format!("cannot start Java: {e}"))?;
    let text = String::from_utf8_lossy(&[output.stdout, output.stderr].concat()).into_owned();
    let first = text.lines().next().unwrap_or_default().to_owned();
    if output.status.success() {
        Ok(format!("Java runs: {first}. (Preparing the game files is the next step.)"))
    } else {
        let tail: Vec<_> = text.lines().take(4).collect();
        Err(format!("Java did not start ({}): {}", output.status, tail.join(" | ")))
    }
}
