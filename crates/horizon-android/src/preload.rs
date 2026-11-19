//! A Java launcher finds its runtime from where it lives, which it reads as `/proc/self/exe`. Run
//! through Android's loader (the only way an app may start a downloaded program) that names the
//! loader instead. This library is preloaded into such a program and answers that one read with
//! the path in `HORIZON_PROC_SELF_EXE`; every other `readlink` goes to the kernel as before.
//! In the game's own process the variable is never set, so nothing changes there.

/// # Safety
/// Called by C code with `readlink`'s contract: `path` a C string, `buf` writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readlink(
    path: *const libc::c_char,
    buf: *mut libc::c_char,
    size: libc::size_t,
) -> libc::ssize_t {
    unsafe {
        if !path.is_null() && libc::strcmp(path, c"/proc/self/exe".as_ptr()) == 0 {
            let value = libc::getenv(c"HORIZON_PROC_SELF_EXE".as_ptr());
            if !value.is_null() {
                let n = libc::strlen(value).min(size);
                libc::memcpy(buf.cast(), value.cast(), n);
                return n as libc::ssize_t;
            }
        }
        libc::syscall(libc::SYS_readlinkat, libc::AT_FDCWD, path, buf, size) as libc::ssize_t
    }
}

/// This library's own path, for `LD_PRELOAD`, from the process's memory map.
pub fn own_path() -> Option<std::path::PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        .filter_map(|l| l.split_whitespace().nth(5))
        .find(|p| p.ends_with("libhorizon_android.so"))
        .map(std::path::PathBuf::from)
}
