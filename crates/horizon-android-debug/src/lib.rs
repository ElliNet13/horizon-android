//! "Horizon (Debug)": the same game as `horizon-android`, as an app of its own
//! (`dev.horizon.android.debug`), so a debug build installs beside the release one and keeps its
//! own game folder. Only the app's name and package (Cargo.toml) differ.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    horizon_android::run(app);
}
