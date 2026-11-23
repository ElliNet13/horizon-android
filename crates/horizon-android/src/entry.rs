use std::sync::OnceLock;
use winit::platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp};

pub(crate) static APP: OnceLock<AndroidApp> = OnceLock::new();

/// ElliNet13's server in the server list in place of the official one, without `horizon_official_servers::HOOKS`'s other
/// answer: it sends a game without its files to Horizon's desktop launcher, which has no Android
/// version (the on-device preparing is this app's own).
struct Servers;
static SERVERS: Servers = Servers;

impl horizon_hooks::Hooks for Servers {
    fn started(&self, game: &horizon_hooks::Started<'_>) {
        horizon_official_servers::update_ellinet13(game.game_dir);
    }
}

/// Starts the game on the activity: what `android_main` does.
pub fn run(app: AndroidApp) {
    // The engine runs once per process (its hooks and engine state are set for good). After a
    // crash the system can leave the process alive and start the activity in it again: end it, so
    // the next start gets a new one.
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        // SAFETY: ends the process at once, as intended.
        unsafe { libc::_exit(1) };
    }
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            // The Adreno driver's validation notes repeat every frame: only wgpu_hal's errors.
            .with_filter(
                android_logger::FilterBuilder::new()
                    .parse("info,wgpu_hal=error")
                    .build(),
            )
            .with_tag("horizon"),
    );
    // The JNI helpers (`storage`, `clipboard`) need the activity from the start.
    let _ = APP.set(app.clone());
    // The game runs on this thread: a panic here ends the process (with a crash report).
    horizon_client::crash::set_game_thread(std::thread::current().id());
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    // Which build this is, and the thread stack it asked for (the first lines of a crash log).
    log::info!(
        "horizon-android {} (RUST_MIN_STACK={:?})",
        env!("CARGO_PKG_VERSION"),
        std::env::var("RUST_MIN_STACK").ok()
    );

    // The game keeps its saves, options and prepared resources in the working directory, which
    // is read-only on Android: use the app's external folder (/sdcard/Android/data/<package>/files,
    // reachable with `adb push`), or Documents/Horizon where the Files app shows it (`storage`).
    // The app's private folder, or Documents/Horizon once the player allows all files access.
    let game_dir = app
        .external_data_path()
        .map(|private| crate::storage::game_dir(&private));
    if let Some(dir) = game_dir.clone() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::env::set_current_dir(&dir);
        // SAFETY: nothing else runs yet, so no other thread reads the environment.
        unsafe {
            std::env::set_var("HORIZON_HOME", &dir);
            std::env::set_var("HORIZON_NO_AUDIO", "1");
            // WebAssembly mods need the game's wasm standard library, `horizon-wasm-std.wasm`,
            // in the game folder (there is no program folder to put it beside).
            // The app carries the file when it was built with one; a newer game overwrites it.
            const WASM_STD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/horizon-wasm-std.wasm"));
            let kept = dir.join("horizon-wasm-std.wasm");
            if !WASM_STD.is_empty() && std::fs::read(&kept).ok().as_deref() != Some(WASM_STD) {
                match std::fs::write(&kept, WASM_STD) {
                    Ok(()) => log::info!("wrote {}", kept.display()),
                    Err(e) => log::warn!("could not write {}: {e}", kept.display()),
                }
            }
            if std::env::var_os("HORIZON_WASM_STD").is_none() {
                std::env::set_var("HORIZON_WASM_STD", dir.join("horizon-wasm-std.wasm"));
            }
        }
        log::info!("game folder {}", dir.display());
        // Settings for trying things without a rebuild: `horizon-env.txt` in the game folder,
        // one NAME=VALUE per line (# starts a comment), e.g. HORIZON_PACKED_QUADS=0.
        if let Ok(text) = std::fs::read_to_string(dir.join("horizon-env.txt")) {
            for line in text.lines() {
                let line = line.trim();
                if let Some((name, value)) = line.split_once('=')
                    && !line.starts_with('#')
                    && !name.trim().is_empty()
                {
                    log::info!("horizon-env.txt: {}={}", name.trim(), value.trim());
                    // SAFETY: as above.
                    unsafe { std::env::set_var(name.trim(), value.trim()) };
                }
            }
        }
    }

    // The app's internal storage holds the caches that need links (the mod content overlays are
    // symlinks, which shared storage refuses) and a folder only this app can write (the wasm
    // module cache).
    if let Some(internal) = app.internal_data_path() {
        let cache = internal.join("cache");
        if std::fs::create_dir_all(&cache).is_ok() {
            // SAFETY: nothing else runs yet, so no other thread reads the environment.
            unsafe { std::env::set_var("HORIZON_CACHE_DIR", &cache) };
        }
    }
    // After a crash the game starts again on the crash screen.
    if let Some(private) = app.external_data_path() {
        crate::crash_screen::start(&private);
    }
    // No launcher here: the mod browser installs WebAssembly mods itself.
    horizon_client::entry::allow_wasm_mods_in_game();
    horizon_client::entry::set_memory_hook(crate::memory::info);
    if let Some(dir) = &game_dir {
        crate::account::init(dir);
        crate::launcher_page::set_shared(crate::storage::shared_in_use(dir));
    }
    // Horizon's online parts, as `horizon_official::register` adds them on a desktop: the
    // "I'm playing" ping, Play with Friends, the official server in the server list.
    let parts: [&'static dyn horizon_hooks::Hooks; 3] = [
        &horizon_heartbeat::HOOKS,
        &horizon_friends::game::HOOKS,
        &SERVERS,
    ];
    for part in parts {
        if let Err(e) = horizon_hooks::register(part) {
            log::warn!("Horizon: an in-game part did not register: {e}");
        }
    }
    // The launcher page and its text, before the game starts.
    if let Err(e) = horizon_hooks::lang::register(crate::launcher_page::LANG)
        .and(horizon_hooks::pages::register(&crate::launcher_page::PAGE))
    {
        log::warn!("launcher page not registered: {e:?}");
    }

    // The debug app has a page of its own for testing (crashing on purpose).
    if app
        .external_data_path()
        .is_some_and(|p| p.to_string_lossy().contains(".debug"))
        && let Err(e) = horizon_hooks::lang::register(crate::debug_page::LANG)
            .and(horizon_hooks::pages::register(&crate::debug_page::PAGE))
    {
        log::warn!("debug page not registered: {e:?}");
    }

    let _ = APP.set(app);
    if let Some(app) = APP.get() {
        crate::immersive::keep(app);
    }
    horizon_client::entry::set_event_loop_hook(|builder| {
        if let Some(app) = APP.get() {
            builder.with_android_app(app.clone());
        }
    });
    horizon_client::entry::set_prepare_hook(crate::on_device::prepare);
    horizon_client::entry::set_copy_hook(crate::clipboard::copy);
    horizon_client::entry::set_files_folder_hook(|| {
        let dir = std::env::current_dir().ok()?;
        let shared = crate::storage::shared_in_use(&dir);
        Some((dir, shared))
    });
    // The chat box sits above the on-screen keyboard.
    horizon_client::entry::set_keyboard_height_hook(crate::keyboard::height);
    // "Open folder" buttons open the documents app, as there is no file manager to start.
    horizon_client::desktop::set_open_path_hook(crate::clipboard::open_folder);
    horizon_game::main();
    // Back here, the game ended by itself.
    // The process ends with the game: the engine cannot run twice in one, and a process left alive
    // is reused by the system for the next start, which then fails. `_exit` skips the
    // destructors that abort while other threads still run.
    log::info!("horizon: the game ended; ending the process");
    // SAFETY: ends the process at once, as intended.
    unsafe { libc::_exit(0) };
}
