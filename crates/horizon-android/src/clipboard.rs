//! Puts text on Android's clipboard (the "Copy error" button) and opens a link in the browser.

use jni::JavaVM;
use jni::objects::{JObject, JValue};

pub fn copy(text: &str) -> bool {
    let Some(app) = crate::entry::APP.get() else {
        return false;
    };
    let result = (|| -> Result<(), String> {
        // SAFETY: android-activity hands out the process's JavaVM and the activity's object; both
        // stay valid for as long as the app runs.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let service = env.new_string("clipboard").map_err(|e| e.to_string())?;
        let manager = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&service)],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        let label = env.new_string("Horizon").map_err(|e| e.to_string())?;
        let content = env.new_string(text).map_err(|e| e.to_string())?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&content)],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        env.call_method(
            &manager,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::Object(&clip)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = &result {
        log::warn!("clipboard: {e}");
    }
    result.is_ok()
}

/// Opens `url` in the phone's browser (an `ACTION_VIEW` intent from the activity).
pub fn open_url(url: &str) -> bool {
    let Some(app) = crate::entry::APP.get() else {
        return false;
    };
    let result = (|| -> Result<(), String> {
        // SAFETY: as in `copy`.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let address = env.new_string(url).map_err(|e| e.to_string())?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&address)],
            )
            .and_then(|v| v.l())
            .map_err(|e| e.to_string())?;
        let action = env
            .new_string("android.intent.action.VIEW")
            .map_err(|e| e.to_string())?;
        let intent = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )
            .map_err(|e| e.to_string())?;
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )
        .map_err(|e| e.to_string())?;
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return Err("no app can open the link".into());
        }
        Ok(())
    })();
    if let Err(e) = &result {
        log::warn!("open link: {e}");
    }
    result.is_ok()
}
