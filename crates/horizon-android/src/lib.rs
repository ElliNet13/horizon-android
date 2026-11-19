//! Horizon on Android. The platform's `android_main` builds winit's event loop on the activity
//! the system gave it, points the game at the app's own folder, and runs the same entry the
//! desktop executable runs. Touch input is the game's own (`game/interface/touch_input.rs`).
//!
//! It also offers the no-prepared-files screen a way to prepare on the device (`on_device`): a
//! Java runtime fetched at run time (`provision`) runs the preparer, then goes away.

pub mod provision;

#[cfg(target_os = "android")]
mod entry;
#[cfg(target_os = "android")]
mod on_device;
#[cfg(target_os = "android")]
mod preload;
