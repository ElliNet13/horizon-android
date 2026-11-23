/// At the reporter's measured ceiling of about 1.8 KB per frame, 128 MiB
/// covers about 70k plain frames, including Java's deepest reported 59k case.
#[cfg(not(target_os = "android"))]
pub const GUEST_STACK_BYTES: usize = 128 * 1024 * 1024;
/// Android's game runs on the activity's main thread, which has about 64 MiB and cannot be
/// enlarged: a guest gets what fits there (a mod that recurses very deeply runs out sooner).
#[cfg(target_os = "android")]
pub const GUEST_STACK_BYTES: usize = 48 * 1024 * 1024;
/// Room for Rust host frames below the guest and synchronous host imports above it.
pub const HOST_HEADROOM: usize = 8 * 1024 * 1024;
