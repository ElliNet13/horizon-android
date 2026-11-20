//! The no-prepared-files screen's "Prepare on this device" on Android: keeps the processor awake
//! while it works and runs `flow::run` in the app's private storage.

use crate::{flow, preload, wake};
use std::path::Path;

pub fn prepare(root: &Path, progress: &dyn Fn(&str, Option<f32>)) -> Result<String, String> {
    let app = crate::entry::APP.get();
    // Java needs symbolic links and files the system may load, which the shared-storage game
    // folder cannot hold (symlinks fail there with "permission denied"): the app's private
    // storage it is.
    let private = app
        .and_then(|app| app.internal_data_path())
        .ok_or("the app has no private storage")?;
    // Keep the processor running if the player switches to another app meanwhile.
    let _awake = app.map(wake::acquire).transpose().unwrap_or_else(|e| {
        log::warn!("no wake lock: {e}");
        None
    });
    flow::run(root, &private, preload::own_path(), progress)
}
