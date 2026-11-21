#!/usr/bin/env python3
"""Builds the Android APK. A debug build is a separate app from the release one: it is called
"Horizon (Debug)" and its package is dev.horizon.android.debug, so both can be installed side by
side (each with its own game folder). cargo-apk takes the name and package from Cargo.toml only, so
the debug build edits crates/horizon-android/Cargo.toml for the length of the build and puts it back.

    python3 tools/android_apk.py              # debug: target/debug/apk/Horizon (Debug).apk
    python3 tools/android_apk.py --release    # release: target/release/apk/Horizon.apk
Other arguments go to `cargo apk build`.
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "crates" / "horizon-android" / "Cargo.toml"

PACKAGE = 'package = "dev.horizon.android"'
APK_NAME = 'apk_name = "Horizon"'
LABEL = 'label = "Horizon"'


def debug_manifest(text: str) -> str:
    """The manifest with the debug app's package, file name and label."""
    for line in (PACKAGE, APK_NAME, LABEL):
        if text.count(line) != 1:
            sys.exit(f"{MANIFEST}: expected exactly one `{line}`")
    return (
        text.replace(PACKAGE, 'package = "dev.horizon.android.debug"')
        .replace(APK_NAME, 'apk_name = "Horizon (Debug)"')
        .replace(LABEL, 'label = "Horizon (Debug)"')
    )


def main(args: list[str]) -> int:
    release = "--release" in args
    original = MANIFEST.read_text()
    if not release:
        MANIFEST.write_text(debug_manifest(original))
    try:
        command = ["cargo", "apk", "build", "-p", "horizon-android", *args]
        return subprocess.run(command, cwd=ROOT).returncode
    finally:
        if not release:
            MANIFEST.write_text(original)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
