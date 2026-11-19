# Horizon on Android

This fork is [upstream Horizon](https://github.com/horizon-rust/horizon) plus a small overlay, kept
rebased on upstream's `main` (a daily workflow does it; see below). The overlay:

- `crates/horizon-android`: the NativeActivity entry (`android_main`) and the APK metadata;
- `crates/horizon-client/src/game/interface/touch_input.rs`: connects upstream's touch layer to the game;
- a hook in `entry.rs` so the platform can build winit's event loop on the activity;
- upstream's own Android touch PRs, at the bottom of the branch, until upstream merges them.

## When upstream gets Android support

Use upstream. This fork exists only because upstream has no Android build yet. When it ships one (an
official APK, or an Android target in upstream's `main`):

1. Switch to upstream's build and its instructions.
2. Your worlds carry over: they are in the game folder (below), in vanilla's format.
3. This fork's commits that upstream merged drop out of the rebase by themselves. Whatever is left
   is only this overlay, which you can then delete or archive.

## Build the APK

You need Rust, the Android SDK and NDK r26 or newer, and a phone with USB debugging. Aimed at 64-bit ARM
phones (arm64-v8a, Vulkan), such as the Galaxy S22 Ultra.

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
export ANDROID_HOME=~/Android/Sdk
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/<version>
cargo apk build --release -p horizon-android      # target/release/apk/Horizon.apk
adb install -r target/release/apk/Horizon.apk
```

The first build compiles the whole game and takes a while.

## Add the game data

The APK holds no Minecraft files. Prepare them on a PC from your own Minecraft install with
`horizon-prepare`, then copy the result into the app's folder:

```sh
adb shell mkdir -p /sdcard/Android/data/dev.horizon.android/files/.local/horizon-client
adb push .local/horizon-client/26.3 /sdcard/Android/data/dev.horizon.android/files/.local/horizon-client/
```

Saves, `options.txt` and the touch layout (`horizon-touch.json`) live in that same folder.
Logs: `adb logcat -s horizon RustStdoutStderr`.

## Stay in sync with upstream

- Automatic: `.github/workflows/sync-upstream.yml` rebases `android` onto upstream daily. It needs
  `android` as the default branch and, if upstream changed its workflows, a `SYNC_TOKEN` secret (a
  personal access token with `repo` and `workflow` scopes). A conflict opens an issue.
- By hand: `tools/sync-upstream.sh`, then `git push --force-with-lease origin android`.

## Known gaps

Not tested on a device when written, so expect rough edges. Audio is off (`HORIZON_NO_AUDIO`). The layout
editor, the perspective button and the virtual-mouse buttons are not wired to the game yet.
