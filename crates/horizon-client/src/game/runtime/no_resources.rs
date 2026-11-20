//! A game started without its prepared files (the game build run by hand, not
//! through a launcher): a window that says where to get them
//! (`horizon_client::no_resources`) and quits when it is closed. No loader, no
//! crash report: nothing went wrong in the game.
use super::*;
use horizon_client::lang;
use horizon_client::no_resources::{Action, NoResources};
use std::sync::Mutex;
use winit::event::{MouseButton, TouchPhase};

/// Where an import of a prepared resources zip stands (`import_zip`, on its own thread).
enum Import {
    Running(String, Option<f32>),
    /// Finished; the platform's message, or none for an import.
    Done(Option<String>),
    Failed(String),
}

/// The zip the player puts in the game folder (the working directory) on a phone.
const IMPORT_ZIP: &str = "horizon-resources.zip";

fn import_offered() -> bool {
    cfg!(target_os = "android") || std::env::var_os("HORIZON_IMPORT").is_some()
}

struct Missing {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    screen: NoResources,
    /// The pointer in physical pixels.
    pointer: (f64, f64),
    api: horizon_client::graphics::GraphicsApi,
    settings: Settings,
    error: Option<Box<dyn std::error::Error>>,
    /// The running or finished import, shared with its thread.
    import: Option<Arc<Mutex<Import>>>,
    /// Whether that job is the platform's preparing, not a zip import.
    preparing: bool,
    /// Where the prepared files go.
    root: std::path::PathBuf,
    /// The app is in the background: its surface is gone, so nothing is drawn.
    suspended: bool,
    /// The error was copied: say so instead of the error's own text.
    copied: bool,
}

/// Shows the screen until the player closes it.
pub(super) fn show(looked_in: &std::path::Path, settings: &Settings) -> Result<()> {
    // The player's language, from the strings compiled in: there are no prepared ones.
    horizon_client::lang::set(horizon_client::lang::Language::load(
        &[],
        &settings.language,
    ));
    let mut app = Missing {
        window: None,
        renderer: None,
        gui: UiResources {
            atlas: HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
                looked_in.to_path_buf(),
            ))?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::colour([0; 4])),
            splashes: Arc::from(""),
        },
        screen: NoResources::new(looked_in, horizon_hooks::engine::get_the_game()),
        pointer: (0.0, 0.0),
        api: settings.horizon.api,
        settings: settings.clone(),
        error: None,
        import: None,
        preparing: false,
        root: looked_in.to_path_buf(),
        suspended: false,
        copied: false,
    };
    if crate::entry::prepare_hook().is_some() {
        app.screen.offer_prepare();
    }
    if import_offered() {
        app.screen.offer_import(std::path::Path::new(IMPORT_ZIP));
    }

    crate::entry::new_event_loop()?.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

impl Missing {
    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    /// `f` over a Ui of the window's size, the cursor where the pointer is.
    fn with_ui<R>(
        &self,
        batch: &mut HudBatch,
        f: impl FnOnce(&mut horizon_client::ui_theme::Ui) -> R,
    ) -> R {
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        let mut ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, batch, cursor);
        if import_offered() {
            // On a phone: as large as fits.
            let zoom = self.screen.fit_zoom(&ui);
            ui.zoom(zoom);
        }
        let result = f(&mut ui);
        ui.finish();
        result
    }

    fn draw(&mut self) {
        if self.suspended {
            return;
        }
        let mut batch = HudBatch::new(self.size(), self.settings.gui_scale_inputs());
        self.with_ui(&mut batch, |ui| self.screen.draw(ui));
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => log::warn!("Horizon: frame failed: {e}"),
        }
        // The frame was not shown (the first one often isn't): the glyphs it
        // rasterized never reached the atlas, so draw them again next frame.
        horizon_client::ui_theme::reset_atlas();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let mut batch = HudBatch::new(self.size(), self.settings.gui_scale_inputs());
        match self.with_ui(&mut batch, |ui| self.screen.click(ui)) {
            Some(Action::Open(address)) => packs::open_uri(address),
            Some(Action::Quit) => event_loop.exit(),
            Some(Action::ImportZip) => self.start_import(),
            Some(Action::Prepare) => self.start_prepare(),
            Some(Action::CopyError) => {
                if let Some(text) = self.screen.copyable().map(str::to_owned) {
                    if crate::entry::copy_text(&text) {
                        self.copied = true;
                    }
                }
            }
            None => {}
        }
    }

    fn start_import(&mut self) {
        if self
            .import
            .as_ref()
            .is_some_and(|i| matches!(*i.lock().unwrap(), Import::Running(..)))
        {
            return;
        }
        let zip = std::path::PathBuf::from(IMPORT_ZIP);
        if !zip.is_file() {
            let shown = std::path::absolute(&zip).unwrap_or(zip);
            self.screen.set_status(Some(lang::tr(
                "horizon.no_resources.import_missing",
                &[&shown.display()],
            )));
            return;
        }
        let state = Arc::new(Mutex::new(Import::Running(String::new(), None)));
        self.import = Some(state.clone());
        let root = self.root.clone();
        std::thread::spawn(move || {
            let progress = |entry: &str, fraction: Option<f32>| {
                *state.lock().unwrap() = Import::Running(entry.to_owned(), fraction)
            };
            let end = match import_zip(&zip, &root, &progress) {
                Ok(()) => Import::Done(None),
                Err(error) => Import::Failed(error),
            };
            *state.lock().unwrap() = end;
        });
    }

    fn start_prepare(&mut self) {
        let Some(hook) = crate::entry::prepare_hook() else {
            return;
        };
        if self
            .import
            .as_ref()
            .is_some_and(|i| matches!(*i.lock().unwrap(), Import::Running(..)))
        {
            return;
        }
        let state = Arc::new(Mutex::new(Import::Running(String::new(), None)));
        self.import = Some(state.clone());
        self.preparing = true;
        let root = self.root.clone();
        std::thread::spawn(move || {
            let progress = |text: &str, fraction: Option<f32>| {
                *state.lock().unwrap() = Import::Running(text.to_owned(), fraction)
            };
            let end = match hook(&root, &progress) {
                Ok(message) => Import::Done(Some(message)),
                Err(error) => Import::Failed(error),
            };
            *state.lock().unwrap() = end;
        });
    }

    /// Copies the import's state onto the screen; true while it is still working.
    fn show_import(&mut self) -> bool {
        let Some(state) = &self.import else {
            return false;
        };
        let preparing = self.preparing;
        let (text, running, fraction) = match &*state.lock().unwrap() {
            Import::Running(entry, fraction) => {
                let key = if preparing {
                    "horizon.no_resources.preparing"
                } else {
                    "horizon.no_resources.importing"
                };
                let shown = match fraction {
                    Some(f) => format!("{}%  {entry}", (f * 100.0).floor() as u32),
                    None => entry.clone(),
                };
                (lang::tr(key, &[&shown]), true, *fraction)
            }
            Import::Done(Some(message)) => (
                lang::tr("horizon.no_resources.prepare_done", &[message]),
                false,
                None,
            ),
            Import::Done(None) => (lang::get("horizon.no_resources.import_done"), false, None),
            Import::Failed(error) => {
                let key = if preparing {
                    "horizon.no_resources.prepare_failed"
                } else {
                    "horizon.no_resources.import_failed"
                };
                (lang::tr(key, &[error]), false, None)
            }
        };
        let failed = matches!(&*state.lock().unwrap(), Import::Failed(_));
        // The copied message keeps the error's text; the button is there while it is a failure.
        self.screen.set_copyable(failed.then(|| text.clone()));
        if !failed {
            self.copied = false;
        }
        let text = if self.copied {
            format!("{text} {}", lang::get("horizon.no_resources.copied"))
        } else {
            text
        };
        self.screen.set_status(Some(text));
        self.screen.set_progress(fraction);
        running
    }
}

impl ApplicationHandler for Missing {
    /// Android destroys the window's surface when the app goes to the background.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.suspended = true;
        if let Some(renderer) = &mut self.renderer {
            renderer.lose_surface();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = false;
        if let Some(window) = &self.window {
            // Back from the background: the window's new surface.
            if let Some(renderer) = &mut self.renderer {
                if let Err(e) = renderer.recreate_surface() {
                    self.error = Some(e);
                    event_loop.exit();
                    return;
                }
            }
            window.request_redraw();
            return;
        }
        let attributes =
            horizon_client::window::with_icon(Window::default_attributes().with_title("Horizon"))
                .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer = loading_renderer(
                    window.clone(),
                    false,
                    Some(&self.gui.atlas),
                    Some(&self.gui.panorama),
                    None,
                    self.api,
                    false,
                )?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.click(event_loop),
            WindowEvent::Touch(touch) => {
                self.pointer = (touch.location.x, touch.location.y);
                if touch.phase == TouchPhase::Ended {
                    self.click(event_loop);
                }
                window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if horizon_threads::stop_requests() > 0 {
            event_loop.exit();
            return;
        }
        if self.show_import() && !self.suspended {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        // Wake now and then for Ctrl+C in the console; redraws come with events.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(250),
        ));
    }
}

/// Unpacks a prepared resources zip into `root`: the zip holds the contents of the prepared
/// folder (`horizon-resources.json` and the rest), at its top or inside one folder. It is
/// extracted beside `root`, checked like a normal start would, and only then moved in place.
fn import_zip(
    zip: &std::path::Path,
    root: &std::path::Path,
    progress: &dyn Fn(&str, Option<f32>),
) -> std::result::Result<(), String> {
    let file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut archive = ::zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let name = root.file_name().ok_or("no folder name")?.to_owned();
    let partial = root.with_file_name(format!("{}.partial", name.to_string_lossy()));
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
    }
    // A zip made by zipping the prepared folder itself has one folder around everything.
    let wrapped = (0..archive.len()).all(|i| {
        archive.by_index(i).ok().is_some_and(|f| {
            f.enclosed_name().is_some_and(|p| {
                p.components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == name.as_os_str())
            })
        })
    });
    let count = archive.len();
    for i in 0..count {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let path = if wrapped {
            path.components().skip(1).collect::<std::path::PathBuf>()
        } else {
            path
        };
        if path.as_os_str().is_empty() {
            continue;
        }
        let target = partial.join(&path);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        progress(
            &format!("{}/{} {}", i + 1, count, path.display()),
            Some((i + 1) as f32 / count as f32),
        );
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    if let Some(problem) = missing_resources(&partial, None) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(problem.lines().next().unwrap_or_default().to_owned());
    }
    if root.exists() {
        std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&partial, root).map_err(|e| e.to_string())
}
