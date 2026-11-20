//! A partial wake lock: the processor keeps running while the screen is off, so a long download
//! and unpack goes on when the player switches to another app. (Android may still stop an app in
//! the background; a foreground service would prevent that, and needs Java code this build does
//! not have. Everything already unpacked is kept, so a stopped prepare resumes.)

use jni::JavaVM;
use jni::objects::{GlobalRef, JObject, JValue};
use winit::platform::android::activity::AndroidApp;

/// How long the lock may be held, in milliseconds, if it is never released.
const LIMIT_MS: i64 = 2 * 60 * 60 * 1000;
/// `PowerManager.PARTIAL_WAKE_LOCK`.
const PARTIAL_WAKE_LOCK: i32 = 1;

/// The held lock; it is released when this is dropped.
pub struct WakeLock {
    vm: JavaVM,
    lock: GlobalRef,
}

pub fn acquire(app: &AndroidApp) -> Result<WakeLock, String> {
    // SAFETY: android-activity hands out the process's JavaVM and the activity's object; both
    // stay valid for as long as the app runs.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let service = env.new_string("power").map_err(|e| e.to_string())?;
    let power = env
        .call_method(
            &activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )
        .and_then(|v| v.l())
        .map_err(|e| e.to_string())?;
    let tag = env
        .new_string("horizon:prepare")
        .map_err(|e| e.to_string())?;
    let lock = env
        .call_method(
            &power,
            "newWakeLock",
            "(ILjava/lang/String;)Landroid/os/PowerManager$WakeLock;",
            &[JValue::Int(PARTIAL_WAKE_LOCK), JValue::Object(&tag)],
        )
        .and_then(|v| v.l())
        .map_err(|e| e.to_string())?;
    env.call_method(&lock, "acquire", "(J)V", &[JValue::Long(LIMIT_MS)])
        .map_err(|e| e.to_string())?;
    let lock = env.new_global_ref(&lock).map_err(|e| e.to_string())?;
    drop(env);
    Ok(WakeLock { vm, lock })
}

impl Drop for WakeLock {
    fn drop(&mut self) {
        if let Ok(mut env) = self.vm.attach_current_thread() {
            let _ = env.call_method(&self.lock, "release", "()V", &[]);
        }
    }
}
