# Horizon on Android

An overlay on upstream Horizon: the `horizon-android` crate (NativeActivity entry), touch wiring in
`game/interface/touch_input.rs`, and an event-loop hook in `entry.rs`. Everything else is upstream's.
Update with `tools/sync-upstream.sh`.

## Build

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
export ANDROID_HOME=~/Android/Sdk ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/<version>   # NDK r26+
cargo apk build --release -p horizon-android          # target/release/apk/Horizon.apk
adb install -r target/release/apk/Horizon.apk
```

## Resources

Prepare on a PC (`horizon-prepare`, from your own Minecraft install), then copy the result:

```sh
adb shell mkdir -p /sdcard/Android/data/dev.horizon.android/files/.local/horizon-client
adb push .local/horizon-client/26.3 /sdcard/Android/data/dev.horizon.android/files/.local/horizon-client/
```

Launch the app. Logs: `adb logcat -s horizon RustStdoutStderr`.

## Known gaps

Untested on a device when written. Audio is off (`HORIZON_NO_AUDIO`); the layout editor, perspective
button, auxiliary keys and on-screen keyboard are not wired.
