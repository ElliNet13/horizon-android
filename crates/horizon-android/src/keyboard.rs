//! How tall the on-screen keyboard is, in pixels, so the chat box can sit above it.

use jni::JNIEnv;
use jni::objects::{JObject, JValue};

/// `WindowInsets.Type.ime()`.
const IME: i32 = 8;

/// The keyboard's height now; 0 when it is hidden or the system will not say.
pub fn height() -> u32 {
    let Some(app) = crate::entry::APP.get() else {
        return 0;
    };
    // SAFETY: android-activity hands out the process's JavaVM and the activity's object; both
    // stay valid for as long as the app runs.
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return 0;
    };
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let Ok(mut env) = vm.attach_current_thread() else {
        return 0;
    };
    let height = measure(&mut env, &activity).unwrap_or(0);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    height.max(0) as u32
}

fn measure(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<i32> {
    let window = env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
        .l()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
        .l()?;
    // The insets the keyboard takes (Android 11+), whatever the window's layout flags.
    let mut best = 0;
    let insets = env
        .call_method(
            &decor,
            "getRootWindowInsets",
            "()Landroid/view/WindowInsets;",
            &[],
        )?
        .l()?;
    if !insets.is_null() {
        let ime = env
            .call_method(
                &insets,
                "getInsets",
                "(I)Landroid/graphics/Insets;",
                &[JValue::Int(IME)],
            )?
            .l()?;
        best = env.get_field(&ime, "bottom", "I")?.i()?;
    }
    if best == 0 {
        // Older systems: what the keyboard leaves of the window.
        let rect = env.new_object("android/graphics/Rect", "()V", &[])?;
        env.call_method(
            &decor,
            "getWindowVisibleDisplayFrame",
            "(Landroid/graphics/Rect;)V",
            &[JValue::Object(&rect)],
        )?;
        let bottom = env.get_field(&rect, "bottom", "I")?.i()?;
        let total = env.call_method(&decor, "getHeight", "()I", &[])?.i()?;
        // A few dozen pixels are the navigation bar, not a keyboard.
        if total - bottom > 200 {
            best = total - bottom;
        }
    }
    Ok(best)
}
