#!/usr/bin/env python3
"""Try a Horizon-for-Android setting quickly: stop the app, write horizon-env.txt in its game
folder, start the app again, and (optionally) show the log and take a screenshot.

    python tools/android_env.py HORIZON_PACKED_QUADS=0
    python tools/android_env.py HORIZON_GPU_CULL=0 HORIZON_FLAT_DRAWS=1 --wait 40 --shot out.png
    python tools/android_env.py --clear          # no settings
    python tools/android_env.py --show           # what the file holds now
    python tools/android_env.py --cycle          # step through the renderer tests with Space

Needs `adb` on the PATH and one device (or pass --serial). Settings are NAME=VALUE; the app reads
them once at startup and logs each as `horizon-env.txt: NAME=VALUE`.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PACKAGE = "dev.horizon.android"
GAME_DIR = f"/sdcard/Android/data/{PACKAGE}/files"
ENV_FILE = f"{GAME_DIR}/horizon-env.txt"
LOG_TAGS = ["horizon", "RustStdoutStderr", "AndroidRuntime", "DEBUG", "libc"]
NOISE = ("VKDBGUTILWARN003", "wgpu_hal", "Access denied finding property")


def adb(serial: str | None, *args: str, capture: bool = True, check: bool = True) -> str:
    command = ["adb"] + (["-s", serial] if serial else []) + list(args)
    result = subprocess.run(command, capture_output=capture, text=True, check=False)
    if check and result.returncode != 0:
        sys.exit(f"adb {' '.join(args)} failed: {(result.stderr or result.stdout).strip()}")
    return result.stdout if capture else ""


def valid(setting: str) -> bool:
    name, sep, _ = setting.partition("=")
    return bool(sep) and bool(name.strip()) and not name.startswith("#")


def write_settings(serial: str | None, settings: list[str]) -> None:
    with tempfile.TemporaryDirectory() as folder:
        local = Path(folder) / "horizon-env.txt"
        local.write_text("".join(f"{s}\n" for s in settings), encoding="utf-8")
        adb(serial, "shell", "mkdir", "-p", GAME_DIR)
        adb(serial, "push", str(local), ENV_FILE)


def show_log(serial: str | None, seconds: float, everything: bool) -> None:
    print(f"--- log after {seconds:.0f} s ---")
    output = adb(serial, "logcat", "-d", "-s", *LOG_TAGS)
    for line in output.splitlines():
        if everything or not any(noise in line for noise in NOISE):
            print(line)


def screenshot(serial: str | None, path: Path) -> None:
    command = ["adb"] + (["-s", serial] if serial else []) + ["exec-out", "screencap", "-p"]
    result = subprocess.run(command, capture_output=True, check=False)
    if result.returncode != 0 or not result.stdout:
        print("screenshot failed (a screen the app hides may not be capturable)")
        return
    path.write_bytes(result.stdout)
    print(f"screenshot saved to {path}")


# The renderer experiments --cycle steps through, each a list of NAME=VALUE.
TESTS: list[list[str]] = [
    [],
    ["HORIZON_PACKED_QUADS=0"],
    ["HORIZON_GPU_CULL=0"],
    ["HORIZON_FLAT_DRAWS=0"],
    ["HORIZON_FLAT_DRAWS=1"],
    ["HORIZON_SUBMIT_THREAD=0"],
    ["HORIZON_PACKED_QUADS=0", "HORIZON_GPU_CULL=0"],
    ["HORIZON_PACKED_QUADS=0", "HORIZON_FLAT_DRAWS=1"],
    ["HORIZON_PACKED_QUADS=0", "HORIZON_GPU_CULL=0", "HORIZON_FLAT_DRAWS=0", "HORIZON_SUBMIT_THREAD=0"],
    # The driver itself: OpenGL ES instead of Vulkan.
    ["HORIZON_GRAPHICS_API=opengl"],
    # Block textures in smaller arrays, and without HDR output.
    ["HORIZON_SPRITE_SPLIT=512"],
    ["HORIZON_HDR_OUTPUT=scrgb"],
]


def read_key() -> str:
    """One key press, without Enter."""
    if sys.platform == "win32":
        import msvcrt

        return msvcrt.getwch()
    import termios
    import tty

    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    try:
        tty.setcbreak(fd)
        return sys.stdin.read(1)
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)


def start(serial: str | None, settings: list[str]) -> None:
    adb(serial, "shell", "am", "force-stop", PACKAGE)
    write_settings(serial, settings)
    adb(serial, "logcat", "-c")
    adb(serial, "shell", "monkey", "-p", PACKAGE, "-c", "android.intent.category.LAUNCHER", "1")


def cycle(serial: str | None, first: int) -> None:
    """Space (or Enter) runs the next test, b the one before, r the same again, q quits."""
    position = first
    while True:
        settings = TESTS[position]
        start(serial, settings)
        label = ", ".join(settings) if settings else "no settings (baseline)"
        print(f"\n[{position + 1}/{len(TESTS)}] {label}")
        while True:
            print("  space = next test   b = back   r = restart this   l = log   q = quit", end="\r", flush=True)
            key = read_key()
            if key in (" ", "\r", "\n"):
                if position + 1 >= len(TESTS):
                    print("\nThat was the last test.")
                    return
                position += 1
                break
            if key in ("b", "B"):
                position = max(position - 1, 0)
                break
            if key in ("r", "R"):
                break
            if key in ("l", "L"):
                print()
                show_log(serial, 0, False)
            if key in ("q", "Q", "\x03"):
                print()
                return


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("settings", nargs="*", help="NAME=VALUE, any number")
    parser.add_argument("--clear", action="store_true", help="write an empty settings file")
    parser.add_argument("--show", action="store_true", help="print the settings file and exit")
    parser.add_argument("--wait", type=float, default=0, metavar="SECONDS",
                        help="after starting, wait this long, then print the log")
    parser.add_argument("--shot", type=Path, metavar="PNG", help="with --wait: save a screenshot after it")
    parser.add_argument("--all-log", action="store_true", help="do not hide the wgpu_hal noise")
    parser.add_argument("--cycle", nargs="?", const=1, type=int, metavar="FIRST",
                        help="step through the renderer tests with Space (start at test FIRST, default 1)")
    parser.add_argument("--serial", help="the device, when several are connected")
    args = parser.parse_args()

    if shutil.which("adb") is None:
        sys.exit("adb is not on the PATH")
    if args.cycle is not None:
        cycle(args.serial, min(max(args.cycle, 1), len(TESTS)) - 1)
        return
    if args.show:
        print(adb(args.serial, "shell", "cat", ENV_FILE, check=False) or "(no settings file)")
        return
    bad = [s for s in args.settings if not valid(s)]
    if bad:
        sys.exit(f"not NAME=VALUE: {', '.join(bad)}")
    if not args.settings and not args.clear:
        parser.error("give NAME=VALUE settings, or --clear, or --show")

    adb(args.serial, "shell", "am", "force-stop", PACKAGE)
    write_settings(args.serial, args.settings)
    adb(args.serial, "logcat", "-c")
    print(f"settings: {', '.join(args.settings) if args.settings else '(none)'}")
    adb(args.serial, "shell", "monkey", "-p", PACKAGE, "-c", "android.intent.category.LAUNCHER", "1")
    print("app started")
    if args.wait > 0:
        time.sleep(args.wait)
        show_log(args.serial, args.wait, args.all_log)
        if args.shot:
            screenshot(args.serial, args.shot)


if __name__ == "__main__":
    main()
