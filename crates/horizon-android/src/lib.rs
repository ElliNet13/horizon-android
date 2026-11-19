//! Horizon on Android. The platform's `android_main` builds winit's event loop on the activity
//! the system gave it, points the game at the app's own folder, and runs the same entry the
//! desktop executable runs. Touch input is the game's own (`game/interface/touch_input.rs`).
#![cfg(target_os = "android")]

use std::sync::OnceLock;
use winit::platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp};

static APP: OnceLock<AndroidApp> = OnceLock::new();

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("horizon"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));

    // The game keeps its saves, options and prepared resources in the working directory, which
    // is read-only on Android: use the app's external folder (/sdcard/Android/data/<package>/files,
    // reachable with `adb push`).
    if let Some(dir) = app.external_data_path() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::env::set_current_dir(&dir);
        // SAFETY: nothing else runs yet, so no other thread reads the environment.
        unsafe {
            std::env::set_var("HORIZON_HOME", &dir);
            std::env::set_var("HORIZON_NO_AUDIO", "1");
        }
        log::info!("game folder {}", dir.display());
    }

    let _ = APP.set(app);
    horizon_client::entry::set_event_loop_hook(|builder| {
        if let Some(app) = APP.get() {
            builder.with_android_app(app.clone());
        }
    });
    horizon_game::main();
}
