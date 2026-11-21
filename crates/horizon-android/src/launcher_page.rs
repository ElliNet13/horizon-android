//! Options > Horizon Settings > Launcher: what a desktop launcher would set before starting the
//! game, in the game itself: the offline player name, the Microsoft account, the renderer's draw
//! mode and where the game keeps its files. A page of `horizon_hooks::pages`, so no engine
//! screen names it.
use crate::{account, envfile};
use horizon_hooks::pages::{Act, Label, Page, Place, Row};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The renderer setting this page edits (`gfx.rs`'s `multi_draw_wanted`).
const MULTI_DRAW: &str = "HORIZON_MULTI_DRAW";

/// The game folder is in the shared storage (Documents/Horizon), not the app's private folder.
static SHARED: AtomicBool = AtomicBool::new(false);

/// Says where the game folder is (set once at start).
pub fn set_shared(shared: bool) {
    SHARED.store(shared, Ordering::Relaxed);
}

/// The last name typed was refused.
static BAD_NAME: AtomicBool = AtomicBool::new(false);

/// The page's text.
pub static LANG: &horizon_hooks::lang::Files =
    &[("en_us", include_str!("../assets/lang/en_us.json"))];

pub static PAGE: Launcher = Launcher;
pub struct Launcher;

/// What a row does; rows and their acts are made from the same list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Display,
    Name,
    SignIn,
    Cancel,
    CopyCode,
    OpenLink,
    Import,
    SignOut,
    ClientId,
    Draws,
    OpenFolder,
    AllowFiles,
}

/// How the terrain is drawn: the renderer's choice, one call per section, or indirect
/// multi-draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Draws {
    Auto,
    PerSection,
    Indirect,
}

impl Draws {
    fn read() -> Self {
        let value = account::env_path().and_then(|p| envfile::get(&p, MULTI_DRAW));
        match value.as_deref() {
            Some("0") => Self::PerSection,
            Some("1") => Self::Indirect,
            _ => Self::Auto,
        }
    }
    fn next(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::Auto, true) | (Self::Indirect, false) => Self::PerSection,
            (Self::PerSection, true) | (Self::Auto, false) => Self::Indirect,
            (Self::Indirect, true) | (Self::PerSection, false) => Self::Auto,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::PerSection => "One per section",
            Self::Indirect => "Indirect multi-draws",
        }
    }
    fn value(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::PerSection => Some("0"),
            Self::Indirect => Some("1"),
        }
    }
}

/// The offline name kept in `horizon-player.txt`, read once: the page asks for its rows every
/// frame and the file must not be read on the render thread each time.
static NAME: Mutex<Option<String>> = Mutex::new(None);

fn offline_name() -> String {
    let mut cached = NAME.lock().unwrap_or_else(|e| e.into_inner());
    cached
        .get_or_insert_with(|| {
            horizon_client::offline_name::load_or_create(&horizon_client::logging::player_dir())
        })
        .clone()
}

fn set_offline_name(name: &str) {
    if !horizon_client::offline_name::is_valid(name) {
        BAD_NAME.store(true, Ordering::Relaxed);
        return;
    }
    BAD_NAME.store(false, Ordering::Relaxed);
    *NAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.to_owned());
    let path = horizon_client::logging::player_dir().join(horizon_client::offline_name::FILE);
    if let Err(e) = std::fs::write(&path, format!("name:{name}\n")) {
        log::warn!("Could not keep the player name in {}: {e}", path.display());
    }
}

/// The phone's clipboard and browser; nothing off Android.
mod platform {
    #[cfg(target_os = "android")]
    pub fn copy(text: &str) -> bool {
        crate::clipboard::copy(text)
    }
    #[cfg(target_os = "android")]
    pub fn allow_files() -> bool {
        crate::storage::request_all_files_access()
    }
    #[cfg(not(target_os = "android"))]
    pub fn allow_files() -> bool {
        false
    }
    #[cfg(target_os = "android")]
    pub fn open(url: &str) -> bool {
        crate::clipboard::open_url(url)
    }
    #[cfg(not(target_os = "android"))]
    pub fn copy(_: &str) -> bool {
        false
    }
    #[cfg(not(target_os = "android"))]
    pub fn open(_: &str) -> bool {
        false
    }
}

/// The page's rows with what each does.
fn layout() -> Vec<(Row, Item)> {
    layout_with(offline_name())
}

/// The same with the offline name given (what a row does does not depend on it).
fn layout_with(name: String) -> Vec<(Row, Item)> {
    let state = account::state();
    let mut rows = vec![
        (
            Row::Heading("horizon.android.launcher.player"),
            Item::Display,
        ),
        (
            Row::field("horizon.android.launcher.name", name).max(16),
            Item::Name,
        ),
    ];
    if BAD_NAME.load(Ordering::Relaxed) {
        let text = "Not a valid name: 1 to 16 characters, no spaces.";
        rows.push((Row::text(text).quiet(), Item::Display));
    } else {
        let text = "Used offline and on servers that don't check accounts. A signed-in account's name replaces it.";
        rows.push((Row::text(text).quiet(), Item::Display));
    }
    rows.push((
        Row::Heading("horizon.android.launcher.account"),
        Item::Display,
    ));
    let working = matches!(state, account::State::Working);
    match &state {
        account::State::SignedIn(name) => {
            rows.push((Row::text(format!("Signed in as {name}")), Item::Display));
        }
        account::State::Waiting { code, address } => {
            rows.push((
                Row::text(format!("On any device, open {address} and enter this code:")),
                Item::Display,
            ));
            rows.push((Row::text(code.clone()).groups(), Item::Display));
            rows.push((
                Row::button("horizon.android.launcher.copy_code").half(),
                Item::CopyCode,
            ));
            rows.push((
                Row::button("horizon.android.launcher.open_link").half(),
                Item::OpenLink,
            ));
        }
        account::State::Working => rows.push((Row::text("Signing in..."), Item::Display)),
        account::State::Failed(message) => {
            rows.push((Row::text(message.clone()), Item::Display));
        }
        account::State::SignedOut => {
            rows.push((
                Row::text("Not signed in: playing offline.").quiet(),
                Item::Display,
            ));
        }
    }
    if matches!(state, account::State::Waiting { .. }) {
        rows.push((Row::button("horizon.android.launcher.cancel"), Item::Cancel));
    } else {
        let can_sign_in = account::client_id().is_some() && !working;
        rows.push((
            Row::button("horizon.android.launcher.sign_in").active(can_sign_in),
            Item::SignIn,
        ));
        rows.push((
            Row::button("horizon.android.launcher.import").active(!working),
            Item::Import,
        ));
        let signed_in = matches!(state, account::State::SignedIn(_));
        rows.push((
            Row::button("horizon.android.launcher.sign_out").active(signed_in),
            Item::SignOut,
        ));
    }
    rows.push((
        Row::field(
            "horizon.android.launcher.client_id",
            account::client_id().unwrap_or_default(),
        )
        .max(64),
        Item::ClientId,
    ));
    let help = format!(
        "Signing in needs an Azure app registered for personal accounts, and Mojang has to approve it for Minecraft. Without one, copy {} from the desktop launcher into the game folder and press Use session file: it lasts about a day.",
        account::SESSION_FILE
    );
    rows.push((Row::text(help).quiet(), Item::Display));
    rows.push((
        Row::Heading("horizon.android.launcher.renderer"),
        Item::Display,
    ));
    rows.push((
        Row::Cycle {
            label: "horizon.android.launcher.draws",
            value: Label::Value(Draws::read().name().into()),
        },
        Item::Draws,
    ));
    rows.push((
        Row::text("Blocks need one draw call per section on the Adreno 730; the change applies at the next start.")
            .quiet(),
        Item::Display,
    ));
    rows.push((
        Row::Heading("horizon.android.launcher.files"),
        Item::Display,
    ));
    let folder = account::dir().map_or_else(String::new, |d| d.display().to_string());
    rows.push((Row::Note(folder), Item::Display));
    if SHARED.load(Ordering::Relaxed) {
        rows.push((
            Row::button("horizon.android.launcher.open_folder"),
            Item::OpenFolder,
        ));
    } else {
        rows.push((
            Row::text(
                "The Files app cannot show the app's private folder. Allow all files access, then restart the game: it moves to Documents/Horizon.",
            )
            .quiet(),
            Item::Display,
        ));
        rows.push((
            Row::button("horizon.android.launcher.allow_files"),
            Item::AllowFiles,
        ));
    }
    rows
}

impl Page for Launcher {
    fn id(&self) -> &'static str {
        "horizon-android.launcher"
    }
    fn title(&self) -> &'static str {
        "horizon.android.launcher"
    }
    fn place(&self) -> Place {
        Place::HorizonSettings
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.extend(layout().into_iter().map(|(row, _)| row));
    }
    fn act(&self, row: usize, act: Act) {
        let Some((_, item)) = layout_with(String::new()).into_iter().nth(row) else {
            return;
        };
        match (item, act) {
            (Item::Name, Act::Edit(text)) => set_offline_name(&text),
            (Item::ClientId, Act::Edit(text)) => account::set_client_id(&text),
            (Item::SignIn, Act::Press) => account::begin(),
            (Item::Cancel, Act::Press) => account::cancel(),
            (Item::CopyCode, Act::Press) => {
                if let account::State::Waiting { code, .. } = account::state() {
                    platform::copy(&code);
                }
            }
            // The code goes on the clipboard first, so it can be pasted on the page that opens.
            (Item::OpenLink, Act::Press) => {
                if let account::State::Waiting { code, address } = account::state() {
                    platform::copy(&code);
                    platform::open(&address);
                }
            }
            (Item::Import, Act::Press) => account::import_session(),
            (Item::SignOut, Act::Press) => account::sign_out(),
            (Item::OpenFolder, Act::Press) => {
                if let Some(dir) = account::dir() {
                    let _ = horizon_client::desktop::open_path(dir);
                }
            }
            (Item::AllowFiles, Act::Press) => {
                platform::allow_files();
            }
            (Item::Draws, act @ (Act::Press | Act::PressBack)) => {
                let next = Draws::read().next(act == Act::Press);
                if let Some(path) = account::env_path() {
                    let _ = envfile::set(&path, MULTI_DRAW, next.value());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(item: Item) -> usize {
        layout_with(String::new())
            .iter()
            .position(|(_, i)| *i == item)
            .unwrap()
    }

    #[test]
    fn the_page_edits_the_env_file_and_signing_in_uses_the_app_id() {
        let dir =
            std::env::temp_dir().join(format!("horizon-launcher-page-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        account::init(&dir);
        let sign_in = |rows: &[(Row, Item)]| {
            rows.iter()
                .find_map(|(row, item)| match (row, item) {
                    (Row::Button { active, .. }, Item::SignIn) => Some(*active),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            account::client_id().as_deref(),
            Some("c36a9fb6-4f2a-41ff-90bd-ae7cc92031eb"),
            "the fork's own app until another is entered"
        );
        assert!(sign_in(&layout_with(String::new())));
        PAGE.act(index(Item::ClientId), Act::Edit("abc-123".into()));
        assert_eq!(account::client_id().as_deref(), Some("abc-123"));
        assert!(sign_in(&layout_with(String::new())));
        // The draw mode steps through its three values and the file follows.
        let env = dir.join("horizon-env.txt");
        PAGE.act(index(Item::Draws), Act::Press);
        assert_eq!(envfile::get(&env, MULTI_DRAW).as_deref(), Some("0"));
        PAGE.act(index(Item::Draws), Act::Press);
        assert_eq!(envfile::get(&env, MULTI_DRAW).as_deref(), Some("1"));
        PAGE.act(index(Item::Draws), Act::Press);
        assert_eq!(envfile::get(&env, MULTI_DRAW), None);
        assert_eq!(
            envfile::get(&env, account::CLIENT_ID).as_deref(),
            Some("abc-123")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
