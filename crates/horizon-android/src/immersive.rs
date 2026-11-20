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

/// Describes the pending Java exception (its `toString`) and clears it.
fn describe(env: &mut jni::JNIEnv) -> String {
    let Ok(exception) = env.exception_occurred() else {
        return "unknown".into();
    };
    let _ = env.exception_clear();
    env.call_method(&exception, "toString", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .ok()
        .and_then(|s| env.get_string(&s.into()).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown".into())
}

/// Runs one step; a Java exception becomes its description, and does not stop later steps.
fn step(
    env: &mut jni::JNIEnv,
    name: &str,
    run: impl FnOnce(&mut jni::JNIEnv) -> jni::errors::Result<()>,
) -> Option<String> {
    match run(env) {
        Ok(()) => None,
        Err(jni::errors::Error::JavaException) => Some(format!("{name}: {}", describe(env))),
        Err(e) => Some(format!("{name}: {e}")),
    }
}

fn apply(vm: &JavaVM, activity: &JObject) -> Vec<String> {
    let mut env = match vm.attach_current_thread() {
        Ok(env) => env,
        Err(e) => return vec![e.to_string()],
    };
    let mut problems = Vec::new();
    let window = match env
        .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])
        .and_then(|v| v.l())
    {
        Ok(window) => window,
        Err(_) => return vec![format!("getWindow: {}", describe(&mut env))],
    };
    // The picture under the camera cut-out.
    problems.extend(step(&mut env, "cutout", |env| {
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
    }));
    // The bars, the older way.
    problems.extend(step(&mut env, "flags", |env| {
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        env.call_method(
            &decor,
            "setSystemUiVisibility",
            "(I)V",
            &[JValue::Int(FLAGS)],
        )?;
        Ok(())
    }));
    // And the newer way (Android 11 and up).
    problems.extend(step(&mut env, "insets", |env| {
        let controller = env
            .call_method(
                &window,
                "getInsetsController",
                "()Landroid/view/WindowInsetsController;",
                &[],
            )?
            .l()?;
        if controller.is_null() {
            return Ok(());
        }
        env.call_method(
            &controller,
            "setSystemBarsBehavior",
            "(I)V",
            &[JValue::Int(2)],
        )?;
        env.call_method(&controller, "hide", "(I)V", &[JValue::Int(7)])?;
        Ok(())
    }));
    problems
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
            let problems = apply(&vm, &activity);
            if !problems.is_empty() && !reported {
                log::warn!("immersive mode: {}", problems.join("; "));
                reported = true;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}
