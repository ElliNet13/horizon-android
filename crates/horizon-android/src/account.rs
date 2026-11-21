//! The Microsoft account of the running game: signing in from the launcher page (`msa`), the
//! refresh token kept in the game folder so the next start signs in again by itself, and a
//! session file from the desktop launcher as another way in.
use crate::{envfile, msa};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The Azure app's client ID, in `horizon-env.txt` (the launcher page edits it).
pub const CLIENT_ID: &str = "HORIZON_MSA_CLIENT_ID";
/// This fork's Azure app (approved by Mojang for Minecraft sign-in). A client ID of a public
/// client is no secret: it is in every sign-in address. `HORIZON_MSA_CLIENT_ID` in
/// `horizon-env.txt`, or the launcher page's field, overrides it.
const DEFAULT_CLIENT_ID: &str = "c36a9fb6-4f2a-41ff-90bd-ae7cc92031eb";
/// The refresh token and the app ID it belongs to.
const ACCOUNT_FILE: &str = "horizon-account.json";
/// A session file from the desktop launcher (the same JSON `--session-file` reads).
pub const SESSION_FILE: &str = "horizon-session.json";

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    SignedOut,
    /// Waiting for the player to enter the code at the address.
    Waiting {
        code: String,
        address: String,
    },
    Working,
    SignedIn(String),
    Failed(String),
}

static STATE: Mutex<State> = Mutex::new(State::SignedOut);
static CANCEL: AtomicBool = AtomicBool::new(false);
static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn state() -> State {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(state: State) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = state;
}

/// The game folder, where the files are kept; set once at start.
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

pub fn env_path() -> Option<PathBuf> {
    dir().map(|d| d.join("horizon-env.txt"))
}

/// The configured Azure app's client ID.
pub fn client_id() -> Option<String> {
    let id = env_path()
        .and_then(|p| envfile::get(&p, CLIENT_ID))
        .or_else(|| std::env::var(CLIENT_ID).ok())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_owned());
    let id = id.trim().to_owned();
    (!id.is_empty()).then_some(id)
}

pub fn set_client_id(id: &str) {
    if let Some(path) = env_path() {
        let id = id.trim();
        let _ = envfile::set(&path, CLIENT_ID, (!id.is_empty()).then_some(id));
    }
}

/// Starts: remembers the folder and signs in again from the kept refresh token.
pub fn init(dir: &Path) {
    let _ = DIR.set(dir.to_owned());
    if dir.join(ACCOUNT_FILE).exists() {
        spawn("Account sign-in", resume);
    }
}

fn spawn(name: &str, work: fn()) {
    if matches!(state(), State::Working | State::Waiting { .. }) {
        return;
    }
    set(State::Working);
    CANCEL.store(false, Ordering::Relaxed);
    if std::thread::Builder::new()
        .name(name.into())
        .spawn(work)
        .is_err()
    {
        set(State::Failed("Could not start the sign-in.".into()));
    }
}

/// Signs in with the device code: the page shows the code (`State::Waiting`).
pub fn begin() {
    spawn("Microsoft sign-in", || {
        if let Err(message) = sign_in() {
            if CANCEL.load(Ordering::Relaxed) {
                set(State::SignedOut);
            } else {
                log::warn!("Microsoft sign-in: {message}");
                set(State::Failed(message));
            }
        }
    });
}

/// Stops a sign-in that waits for the player.
pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

fn sign_in() -> Result<(), String> {
    let id = client_id().ok_or("Enter your Microsoft app ID first.")?;
    let ep = msa::Endpoints::live();
    let code = msa::device_code(&ep, &id)?;
    set(State::Waiting {
        code: code.user_code.clone(),
        address: code.verification_uri.clone(),
    });
    let deadline = Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval;
    let tokens = loop {
        let until = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < until {
            if CANCEL.load(Ordering::Relaxed) {
                return Err("Sign-in cancelled.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if Instant::now() > deadline {
            return Err("The sign-in code expired. Start again.".into());
        }
        match msa::poll(&ep, &id, &code)? {
            msa::Poll::Pending => {}
            msa::Poll::SlowDown => interval = (interval + 5).min(60),
            msa::Poll::Done(tokens) => break tokens,
        }
    };
    set(State::Working);
    finish(msa::minecraft(&ep, tokens)?, &id)
}

/// Signs in again from the refresh token kept by an earlier sign-in.
fn resume() {
    let result = (|| {
        let dir = dir().ok_or("no game folder")?;
        let text = std::fs::read_to_string(dir.join(ACCOUNT_FILE)).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let field = |k: &str| v.get(k).and_then(|s| s.as_str()).map(str::to_owned);
        let (Some(id), Some(refresh)) = (field("client_id"), field("refresh_token")) else {
            return Err("the account file is incomplete".to_owned());
        };
        let ep = msa::Endpoints::live();
        let tokens = msa::refresh(&ep, &id, &refresh).inspect_err(|e| {
            // A refresh token that no longer works is of no use again.
            if e.contains("expired") {
                let _ = std::fs::remove_file(dir.join(ACCOUNT_FILE));
            }
        })?;
        finish(msa::minecraft(&ep, tokens)?, &id)
    })();
    if let Err(message) = result {
        log::warn!("Account sign-in: {message}");
        set(State::Failed(message));
    }
}

/// The account the game plays as from now on, its refresh token kept.
fn finish(account: msa::Signed, client_id: &str) -> Result<(), String> {
    if let Some(dir) = dir() {
        let kept = serde_json::json!({
            "client_id": client_id,
            "refresh_token": account.refresh,
        });
        if let Err(e) = std::fs::write(dir.join(ACCOUNT_FILE), kept.to_string()) {
            log::warn!("Could not keep the sign-in: {e}");
        }
    }
    let uuid = horizon_client::auth::parse_uuid(&account.uuid).ok_or("the account has no UUID")?;
    let mut session =
        horizon_client::auth::AuthSession::new(&account.name, uuid, &account.access_token);
    session.xuid = account.xuid;
    install(session);
    Ok(())
}

fn install(session: horizon_client::auth::AuthSession) {
    let name = session.name.clone();
    horizon_client::auth::set_current(session);
    horizon_client::auth::prepare_chat_keys(true);
    log::info!("signed in as {name}");
    set(State::SignedIn(name));
}

/// Signs in with the desktop launcher's session file in the game folder (read once, then
/// deleted): a way in without an app ID, good for as long as its token (a day).
pub fn import_session() {
    let result = (|| {
        let dir = dir().ok_or("no game folder")?;
        let path = dir.join(SESSION_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|_| format!("Put {SESSION_FILE} in the game folder first."))?;
        let session = horizon_client::auth::AuthSession::parse(&text)?;
        let _ = std::fs::remove_file(&path);
        install(session);
        Ok::<(), String>(())
    })();
    if let Err(message) = result {
        set(State::Failed(message));
    }
}

/// The game plays offline again, and the next start does not sign in.
pub fn sign_out() {
    cancel();
    horizon_client::auth::clear_current();
    if let Some(dir) = dir() {
        let _ = std::fs::remove_file(dir.join(ACCOUNT_FILE));
    }
    set(State::SignedOut);
}
