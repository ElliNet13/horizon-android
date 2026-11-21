//! Horizon on Android. The platform's `android_main` builds winit's event loop on the activity
//! the system gave it, points the game at the app's own folder, and runs the same entry the
//! desktop executable runs. Touch input is the game's own (`game/interface/touch_input.rs`).
//!
//! It also offers the no-prepared-files screen a way to prepare on the device (`on_device`): a
//! Java runtime fetched at run time (`provision`) runs the preparer, then goes away.

pub mod account;
pub mod download;
pub mod envfile;
pub mod flow;
pub mod launcher_page;
pub mod mojang;
pub mod msa;
pub mod provision;

#[cfg(target_os = "android")]
mod clipboard;
#[cfg(target_os = "android")]
mod entry;
#[cfg(target_os = "android")]
pub use entry::run;
#[cfg(target_os = "android")]
mod immersive;
#[cfg(target_os = "android")]
mod on_device;
#[cfg(target_os = "android")]
pub mod preload;
#[cfg(target_os = "android")]
#[doc(hidden)]
pub use libc;
#[cfg(target_os = "android")]
mod storage;
#[cfg(target_os = "android")]
mod wake;

/// What the crate that builds the app's `.so` must define: `android_main`, and the C symbols the
/// Java launcher run through the loader needs from the preloaded `.so` (`preload`). A macro, so
/// they are defined in that crate itself and exported from its library, whichever app it is.
#[cfg(target_os = "android")]
#[macro_export]
macro_rules! export_app {
    () => {
        #[unsafe(no_mangle)]
        fn android_main(app: ::winit::platform::android::activity::AndroidApp) {
            $crate::run(app);
        }

        /// # Safety
        /// `readlink`'s contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn readlink(
            path: *const $crate::libc::c_char,
            buf: *mut $crate::libc::c_char,
            size: $crate::libc::size_t,
        ) -> $crate::libc::ssize_t {
            unsafe { $crate::preload::readlink_hook(path, buf, size) }
        }

        /// # Safety
        /// `execve`'s contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn execve(
            path: *const $crate::libc::c_char,
            argv: *const *const $crate::libc::c_char,
            envp: *const *const $crate::libc::c_char,
        ) -> $crate::libc::c_int {
            unsafe { $crate::preload::execve_hook(path, argv, envp) }
        }

        #[used]
        #[unsafe(link_section = ".init_array")]
        static RAISE_THREAD_STACK: extern "C" fn() = $crate::preload::raise_thread_stack;
    };
}

// This crate is the app unless `horizon-android-debug` is (it calls the macro itself).
#[cfg(all(target_os = "android", feature = "entry-point"))]
export_app!();
