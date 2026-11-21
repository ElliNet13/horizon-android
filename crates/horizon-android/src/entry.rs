use std::sync::OnceLock;
use winit::platform::android::{EventLoopBuilderExtAndroid, activity::AndroidApp};

pub(crate) static APP: OnceLock<AndroidApp> = OnceLock::new();

/// The official server in the server list, without `horizon_official_servers::HOOKS`'s other
/// answer: it sends a game without its files to Horizon's desktop launcher, which has no Android
/// version (the on-device preparing is this app's own).
struct Servers;
static SERVERS: Servers = Servers;

impl horizon_hooks::Hooks for Servers {
    fn started(&self, game: &horizon_hooks::Started<'_>) {
        horizon_official_servers::update(game.game_dir);
    }
}

/// Starts the game on the activity: what `android_main` does.
pub fn run(app: AndroidApp) {
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
    // "Open folder" buttons open the documents app, as there is no file manager to start.
    horizon_client::desktop::set_open_path_hook(crate::clipboard::open_folder);
    horizon_game::main();
}
