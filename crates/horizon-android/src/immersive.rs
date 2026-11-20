//! Immersive mode: the status and navigation bars hidden, the picture under the camera cut-out.
//! The system brings the bars back after some events (coming back from another app, a swipe), so
//! the flags are applied again every second from a small thread.

use jni::JavaVM;
use jni::objects::{JObject, JValue};
use std::time::Duration;

/// `SYSTEM_UI_FLAG_` LAYOUT_STABLE | LAYOUT_HIDE_NAVIGATION | LAYOUT_FULLSCREEN | HIDE_NAVIGATION
/// | FULLSCREEN | IMMERSIVE_STICKY.
const FLAGS: i32 = 0x100 | 0x200 | 0x400 | 0x2 | 0x4 | 0x1000;
/// `LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES`.
const CUTOUT_SHORT_EDGES: i32 = 1;

fn apply(vm: &JavaVM, activity: &JObject) -> Result<(), String> {
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let result = (|| -> jni::errors::Result<()> {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        env.call_method(
            &decor,
            "setSystemUiVisibility",
            "(I)V",
            &[JValue::Int(FLAGS)],
        )?;
        let attributes = env
            .call_method(
                &window,
                "getAttributes",
                "()Landroid/view/WindowManager$LayoutParams;",
                &[],
            )?
            .l()?;
        env.set_field(
            &attributes,
            "layoutInDisplayCutoutMode",
            "I",
            JValue::Int(CUTOUT_SHORT_EDGES),
        )?;
        env.call_method(
            &window,
            "setAttributes",
            "(Landroid/view/WindowManager$LayoutParams;)V",
            &[JValue::Object(&attributes)],
        )?;
        Ok(())
    })();
    if result.is_err() {
        // A Java exception stays pending until cleared, and blocks every later call.
        let _ = env.exception_clear();
    }
    result.map_err(|e| e.to_string())
}

/// Keeps the app immersive for as long as it runs.
pub fn keep(app: &winit::platform::android::activity::AndroidApp) {
    let (vm, activity) = (app.vm_as_ptr() as usize, app.activity_as_ptr() as usize);
    std::thread::spawn(move || {
        // SAFETY: android-activity's JavaVM and activity reference live as long as the process.
        let Ok(vm) = (unsafe { JavaVM::from_raw(vm as *mut _) }) else {
            return;
        };
        let activity = unsafe { JObject::from_raw(activity as *mut _) };
        let mut reported = false;
        loop {
            if let Err(e) = apply(&vm, &activity) {
                if !reported {
                    log::warn!("immersive mode: {e}");
                    reported = true;
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}
