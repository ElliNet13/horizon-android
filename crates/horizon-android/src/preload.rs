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

/// The Java launcher starts itself again when it wants to change its environment, with `execve`
/// on its own file, which Android refuses for files in an app's storage. A program under
/// `HORIZON_EXEC_PREFIX` is started through the loader instead (`/system/bin/linker64 program
/// args...`), which is how it ran in the first place.
///
/// # Safety
/// Called by C code with `execve`'s contract: `path` a C string, `argv` and `envp` NULL-terminated
/// arrays of C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    path: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
) -> libc::c_int {
    unsafe {
        let prefix = libc::getenv(c"HORIZON_EXEC_PREFIX".as_ptr());
        if !prefix.is_null()
            && !path.is_null()
            && libc::strncmp(path, prefix, libc::strlen(prefix)) == 0
        {
            let loader = c"/system/bin/linker64";
            let mut args: Vec<*const libc::c_char> = vec![loader.as_ptr(), path];
            if !argv.is_null() {
                // The program's own name (argv[0]) is replaced by the path above.
                let mut i = 1;
                while !(*argv.add(i)).is_null() {
                    args.push(*argv.add(i));
                    i += 1;
                }
            }
            args.push(std::ptr::null());
            return libc::syscall(libc::SYS_execve, loader.as_ptr(), args.as_ptr(), envp)
                as libc::c_int;
        }
        libc::syscall(libc::SYS_execve, path, argv, envp) as libc::c_int
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

/// Rust threads get this much stack. The activity's `android_main` thread is started by
/// `std::thread::spawn` with the 2 MiB default, which an unoptimized (debug) build overflows in
/// `loading_renderer` alone; the game cannot move to a bigger thread, since winit must poll the
/// activity's own thread. `RUST_MIN_STACK` is read when the first thread is spawned, so it is
/// set when this library loads, before the activity starts.
const THREAD_STACK: &core::ffi::CStr = c"67108864";

extern "C" fn raise_thread_stack() {
    // Not overriding a value the player set (`setenv`'s last argument 0).
    // SAFETY: both strings are NUL-terminated and live for the whole program.
    unsafe {
        libc::setenv(c"RUST_MIN_STACK".as_ptr(), THREAD_STACK.as_ptr(), 0);
    }
}

#[used]
#[unsafe(link_section = ".init_array")]
static RAISE_THREAD_STACK: extern "C" fn() = raise_thread_stack;
