//! The screen of a game started without its prepared files: the game build
//! run by hand instead of through a launcher. It says where to get them, and
//! quits. Drawn in Horizon's design language with the bundled fonts: none of
//! Minecraft's files are there.
//!
//! The page it sends the player to comes from a hook (`horizon_hooks::Hooks::get_the_game`,
//! Horizon's launcher in the official executable). With none, it says how to prepare the files
//! and has only Quit.
use crate::lang;
use crate::ui_theme::{self, Face, Ui, color};
use horizon_hooks::GetTheGame;

const COLUMN: f32 = 560.0;
const BUTTON_GAP: f32 = 12.0;

/// What a click on the screen does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the hook's page.
    Open(&'static str),
    /// Unpack a prepared resources zip the player put in the game folder (phones, where the
    /// preparer cannot run).
    ImportZip,
    /// Get the files on this device, through the platform's hook.
    Prepare,
    Quit,
}

pub struct NoResources {
    title: String,
    paragraphs: Vec<String>,
    /// The buttons' labels and actions, the hook's first.
    buttons: Vec<(String, Action)>,
    /// Where the prepared files were looked for, for whoever starts the game by hand on purpose.
    looked_in: String,
    /// What the import is doing or how it ended, below the paragraphs.
    status: Option<String>,
    /// A bar under the status, 0 to 1.
    progress: Option<f32>,
}

impl NoResources {
    /// The screen in the current language, with `get`'s page when a hook answered
    /// (`horizon_hooks::engine::get_the_game`).
    pub fn new(looked_in: &std::path::Path, get: Option<GetTheGame>) -> Self {
        let shown = std::path::absolute(looked_in).unwrap_or_else(|_| looked_in.to_path_buf());
        let quit = (lang::get("horizon.no_resources.quit"), Action::Quit);
        let (title, paragraphs, buttons) = match get {
            Some(get) => {
                let site = get
                    .address
                    .split_once("://")
                    .map_or(get.address, |(_, s)| s);
                (
                    lang::get(get.title),
                    get.paragraphs
                        .iter()
                        .map(|p| lang::tr(p, &[&site]))
                        .collect(),
                    vec![(lang::get(get.button), Action::Open(get.address)), quit],
                )
            }
            None => (
                lang::get("horizon.no_resources.title"),
                vec![lang::get("horizon.no_resources.prepare")],
                vec![quit],
            ),
        };
        Self {
            title,
            paragraphs,
            buttons,
            looked_in: lang::tr("horizon.no_resources.looked_in", &[&shown.display()]),
            status: None,
            progress: None,
        }
    }

    /// Adds the Import button and says where the zip goes (`zip` is shown to the player).
    pub fn offer_import(&mut self, zip: &std::path::Path) {
        let shown = std::path::absolute(zip).unwrap_or_else(|_| zip.to_path_buf());
        self.paragraphs.push(lang::tr(
            "horizon.no_resources.import_about",
            &[&shown.display()],
        ));
        self.buttons.insert(
            0,
            (lang::get("horizon.no_resources.import"), Action::ImportZip),
        );
    }

    /// Adds the button for the platform's own preparing (`entry::set_prepare_hook`).
    pub fn offer_prepare(&mut self) {
        self.paragraphs
            .push(lang::get("horizon.no_resources.prepare_about"));
        self.buttons.insert(
            0,
            (
                lang::get("horizon.no_resources.prepare_button"),
                Action::Prepare,
            ),
        );
    }

    /// The line under the paragraphs; `None` clears it.
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// The bar under the status, as a fraction; `None` hides it.
    pub fn set_progress(&mut self, progress: Option<f32>) {
        self.progress = progress.map(|p| p.clamp(0.0, 1.0));
    }

    /// How much larger than the design size the screen can be drawn and still fit the window
    /// (1 to 1.8): a phone held at arm's length wants bigger text and buttons.
    pub fn fit_zoom(&self, ui: &Ui) -> f32 {
        (ui.h * SAFE_FRACTION / self.layout(ui).height).clamp(1.0, 1.8)
    }

    /// The buttons' labels, actions and rectangles in dp, and the paragraphs' wrapped
    /// lines: one layout for drawing and clicks.
    fn layout(&self, ui: &Ui) -> Layout {
        let width = COLUMN.min(ui.w - 2.0 * SIDE_MARGIN).max(160.0);
        let x = ((ui.w - width) / 2.0).round();
        let lines: Vec<Vec<String>> = self
            .paragraphs
            .iter()
            .chain(self.status.iter())
            .map(|p| wrap(ui, Face::Regular, body(), p, width))
            .collect();
        let text_h = lines.iter().map(|l| l.len() as f32 * LINE).sum::<f32>()
            + (lines.len() - 1) as f32 * PARAGRAPH_GAP;
        let bar_h = if self.progress.is_some() {
            BAR_GAP + BAR
        } else {
            0.0
        };
        let height = TITLE_LINE + 16.0 + text_h + bar_h + 28.0 + control() + 20.0 + SMALL_LINE;
        let y = ((ui.h - height) / 2.0).max(EDGE_MARGIN).round();
        let buttons_y = y + TITLE_LINE + 16.0 + text_h + bar_h + 28.0;
        let mut buttons = Vec::new();
        let mut bx = x;
        for (label, action) in &self.buttons {
            let w = (ui.measure(Face::Medium, button_text(), label) + 40.0).round();
            buttons.push(Button {
                label: label.clone(),
                action: *action,
                x: bx,
                y: buttons_y,
                w,
            });
            bx += w + BUTTON_GAP;
        }
        Layout {
            x,
            y,
            width,
            height,
            lines,
            buttons,
        }
    }

    pub fn draw(&self, ui: &mut Ui) {
        let layout = self.layout(ui);
        ui.rounded(0.0, 0.0, ui.w, ui.h, 0.0, color::bg());
        let mut y = layout.y;
        ui.text_line(
            Face::Medium,
            title_size(),
            TITLE_LINE,
            layout.x,
            y,
            color::text(),
            &self.title,
        );
        y += TITLE_LINE + 16.0;
        for paragraph in &layout.lines {
            for line in paragraph {
                ui.text_line(
                    Face::Regular,
                    body(),
                    LINE,
                    layout.x,
                    y,
                    color::text(),
                    line,
                );
                y += LINE;
            }
            y += PARAGRAPH_GAP;
        }
        if let Some(fraction) = self.progress {
            let top = y - PARAGRAPH_GAP + BAR_GAP;
            ui.rounded(layout.x, top, layout.width, BAR, BAR / 2.0, color::raised());
            let filled = (layout.width * fraction).max(BAR).min(layout.width);
            ui.rounded(layout.x, top, filled, BAR, BAR / 2.0, color::accent());
        }
        for button in &layout.buttons {
            let hovered = button.contains(ui.cursor);
            let (plate, ink) = match (button.action, hovered) {
                (Action::Open(_) | Action::ImportZip | Action::Prepare, false) => {
                    (color::accent(), color::on_accent())
                }
                (Action::Open(_) | Action::ImportZip | Action::Prepare, true) => {
                    (color::with_alpha(color::accent(), 0.85), color::on_accent())
                }
                (Action::Quit, false) => (color::raised(), color::text()),
                (Action::Quit, true) => (color::hover(), color::text()),
            };
            let h = control();
            ui.rounded(button.x, button.y, button.w, h, h / 2.0, plate);
            let lw = ui.measure(Face::Medium, button_text(), &button.label);
            let lx = button.x + ((button.w - lw) / 2.0).round();
            ui.text_line(
                Face::Medium,
                button_text(),
                h,
                lx,
                button.y,
                ink,
                &button.label,
            );
        }
        let small_y = layout.buttons[0].y + control() + 20.0;
        for (i, line) in wrap(
            ui,
            Face::Regular,
            ui_theme::small(),
            &self.looked_in,
            layout.width,
        )
        .iter()
        .enumerate()
        {
            let line_y = small_y + i as f32 * SMALL_LINE;
            ui.text_line(
                Face::Regular,
                ui_theme::small(),
                SMALL_LINE,
                layout.x,
                line_y,
                color::muted(),
                line,
            );
        }
    }

    /// The button under the cursor (`ui.cursor`, in dp), if any.
    pub fn click(&self, ui: &Ui) -> Option<Action> {
        self.layout(ui)
            .buttons
            .iter()
            .find(|b| b.contains(ui.cursor))
            .map(|b| b.action)
    }
}

fn title_size() -> f32 {
    ui_theme::sizes().title
}
const TITLE_LINE: f32 = 40.0;
fn body() -> f32 {
    ui_theme::sizes().body
}
const LINE: f32 = 24.0;
const PARAGRAPH_GAP: f32 = 10.0;
fn button_text() -> f32 {
    ui_theme::sizes().ui
}
const SMALL_LINE: f32 = 18.0;
/// The progress bar's thickness and its gap above, in dp.
/// Room kept clear at the sides and at the top and bottom, in dp: a phone's rounded corners
/// and edge display cut into the picture. The screen is sized to fill this fraction of the height.
const SIDE_MARGIN: f32 = 48.0;
const EDGE_MARGIN: f32 = 40.0;
const SAFE_FRACTION: f32 = 0.74;
const BAR: f32 = 10.0;
const BAR_GAP: f32 = 10.0;
fn control() -> f32 {
    ui_theme::sizes().control_large
}

struct Layout {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    lines: Vec<Vec<String>>,
    buttons: Vec<Button>,
}

struct Button {
    label: String,
    action: Action,
    x: f32,
    y: f32,
    w: f32,
}

impl Button {
    fn contains(&self, (cx, cy): (f32, f32)) -> bool {
        cx >= self.x && cx < self.x + self.w && cy >= self.y && cy < self.y + control()
    }
}

/// `text` broken at spaces into lines no wider than `width` dp (a longer word,
/// such as a path, gets a line of its own).
fn wrap(ui: &Ui, face: Face, size: f32, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && ui.measure(face, size, &candidate) > width {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{HudAtlas, HudBatch};

    /// A Ui over a `size` window at GUI scale 1, the cursor at `cursor` pixels.
    fn with_ui<R>(size: [u32; 2], cursor: (f32, f32), f: impl FnOnce(&mut Ui) -> R) -> R {
        // Fonts cache glyphs internally, so reuse the atlas only on this test thread.
        // Window size and cursor still belong to a fresh batch for every call.
        thread_local! {
            static ATLAS: HudAtlas = HudAtlas::build(
                &horizon_assets::resource_source::ResourceSource::directory(
                    std::path::Path::new("/nonexistent/horizon-test-assets").to_owned(),
                ),
            ).unwrap();
        }
        ATLAS.with(|atlas| {
            let mut batch = HudBatch::scratch(size);
            let cursor = (cursor.0 / batch.scale, cursor.1 / batch.scale);
            let mut ui = Ui::new(atlas, &mut batch, cursor);
            f(&mut ui)
        })
    }

    /// A hook's answer, with the game's own Horizon strings and another page.
    const GET: GetTheGame = GetTheGame {
        address: "https://example.org/get",
        title: "horizon.get_the_game.title",
        paragraphs: &[
            "horizon.get_the_game.about",
            "horizon.get_the_game.minecraft",
            "horizon.get_the_game.download",
        ],
        button: "horizon.get_the_game.button",
    };

    /// The screen in English, every key it asks for there.
    fn screen(get: Option<GetTheGame>) -> NoResources {
        let (screen, missed) = lang::scoped(lang::Language::load(&[], "en_us"), || {
            NoResources::new(std::path::Path::new(".local/horizon-client/26.3"), get)
        });
        assert_eq!(missed, Vec::<String>::new());
        screen
    }

    #[test]
    fn without_a_hook_it_names_no_site_and_opens_no_page() {
        let screen = screen(None);
        let all = [&screen.title, &screen.looked_in]
            .into_iter()
            .chain(&screen.paragraphs)
            .chain(screen.buttons.iter().map(|(label, _)| label));
        for text in all {
            assert!(!text.to_lowercase().contains("playhorizon"), "{text}");
            assert!(!text.contains("launcher"), "{text}");
        }
        assert!(screen.paragraphs[0].contains("horizon-prepare --prepare"));
        assert_eq!(
            screen.buttons,
            [("Quit".to_owned(), Action::Quit)],
            "only Quit"
        );
    }

    #[test]
    fn a_hook_gives_the_text_and_the_page() {
        let screen = screen(Some(GET));
        assert_eq!(screen.title, "Start Horizon from the launcher");
        assert_eq!(
            screen.paragraphs[2],
            "Download the launcher at example.org/get, then press Play there."
        );
        assert_eq!(
            screen.buttons[0],
            (
                "Download the launcher".to_owned(),
                Action::Open("https://example.org/get")
            )
        );
    }

    #[test]
    fn the_buttons_are_where_they_are_drawn() {
        let screen = screen(Some(GET));
        for size in [[1280, 720], [800, 600], [2560, 1440]] {
            let (open, quit, density) = with_ui(size, (0.0, 0.0), |ui| {
                let layout = screen.layout(ui);
                let center = |b: &Button| (b.x + b.w / 2.0, b.y + control() / 2.0);
                (
                    center(&layout.buttons[0]),
                    center(&layout.buttons[1]),
                    ui.density,
                )
            });
            let at = |p: (f32, f32)| (p.0 * density, p.1 * density);
            assert_eq!(
                with_ui(size, at(open), |ui| screen.click(ui)),
                Some(Action::Open(GET.address)),
                "{size:?}"
            );
            assert_eq!(
                with_ui(size, at(quit), |ui| screen.click(ui)),
                Some(Action::Quit)
            );
            assert_eq!(with_ui(size, (1.0, 1.0), |ui| screen.click(ui)), None);
            // Everything fits in the window.
            with_ui(size, (0.0, 0.0), |ui| {
                let layout = screen.layout(ui);
                let last = layout.buttons.last().unwrap();
                assert!(
                    last.x + last.w <= ui.w && last.y + control() <= ui.h,
                    "{size:?}"
                );
            });
        }
    }

    #[test]
    fn scaled_draw_and_click_agree() {
        let screen = screen(Some(GET));
        let atlas = HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/horizon-test-assets"),
        ))
        .unwrap();
        for unicode in [false, true] {
            let settings = crate::hud::GuiScale {
                requested: 3,
                unicode,
            };
            let mut draw = HudBatch::new([960, 720], settings);
            assert_eq!(draw.scale, if unicode { 4.0 } else { 3.0 });
            let mut ui = Ui::new(&atlas, &mut draw, (0.0, 0.0));
            let layout = screen.layout(&ui);
            let button = &layout.buttons[0];
            let physical = (
                (button.x + button.w / 2.0) * ui.density,
                (button.y + control() / 2.0) * ui.density,
            );
            screen.draw(&mut ui);
            let mut hit = HudBatch::new([960, 720], settings);
            let cursor = (physical.0 / hit.scale, physical.1 / hit.scale);
            let ui = Ui::new(&atlas, &mut hit, cursor);
            assert_eq!(screen.click(&ui), Some(Action::Open(GET.address)));
        }
    }

    #[test]
    fn text_wraps_within_the_column_and_keeps_every_word() {
        let screen = screen(Some(GET));
        with_ui([1280, 720], (0.0, 0.0), |ui| {
            for paragraph in &screen.paragraphs {
                let lines = wrap(ui, Face::Regular, body(), paragraph, 300.0);
                assert!(lines.len() > 1, "{paragraph}");
                for line in &lines {
                    assert!(ui.measure(Face::Regular, body(), line) <= 300.0, "{line}");
                }
                assert_eq!(&lines.join(" "), paragraph);
            }
        });
    }
}
