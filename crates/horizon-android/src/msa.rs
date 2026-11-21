//! Microsoft sign-in for the game itself (the launcher does this on a desktop): the OAuth device
//! code flow, then Xbox Live, XSTS, the Minecraft services and the profile. The Azure app (client
//! ID) is the player's own configuration; Mojang has to approve an app before its Minecraft
//! tokens work (the services answer 403 until then).
use serde_json::{Value, json};
use std::time::Duration;

const SCOPE: &str = "XboxLive.signin offline_access";

/// Where the requests go: the real services, or a mock in tests.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The OAuth 2.0 v2 endpoints for personal accounts (`…/devicecode`, `…/token`).
    pub microsoft: String,
    pub xbox_user: String,
    pub xsts: String,
    /// api.minecraftservices.com.
    pub minecraft: String,
}

impl Endpoints {
    pub fn live() -> Self {
        Self {
            microsoft: "https://login.microsoftonline.com/consumers/oauth2/v2.0".into(),
            xbox_user: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft: "https://api.minecraftservices.com".into(),
        }
    }
}

/// What the player types at the verification page.
#[derive(Clone, PartialEq)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    /// Seconds between polls.
    pub interval: u64,
    /// Seconds until the code expires.
    pub expires_in: u64,
    device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    // The device code can be traded for tokens once the player approves: never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .finish_non_exhaustive()
    }
}

/// The Microsoft tokens: the access token only lasts for the Xbox step.
pub struct Tokens {
    access: String,
    pub refresh: String,
}

pub enum Poll {
    Pending,
    /// The service asked to poll less often.
    SlowDown,
    Done(Tokens),
}

/// A signed-in Minecraft account.
pub struct Signed {
    pub name: String,
    /// 32 hex digits.
    pub uuid: String,
    pub access_token: String,
    pub xuid: Option<String>,
    /// The Microsoft refresh token, to sign in again without the browser.
    pub refresh: String,
}

impl std::fmt::Debug for Signed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signed")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

enum Body<'a> {
    Form(Vec<(&'a str, &'a str)>),
    Json(Value),
}

/// One request: its status and JSON body (an error status's body too).
fn send(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<Body>,
    what: &str,
) -> Result<(u16, Value), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let mut request = agent.request(method, url).set("Accept", "application/json");
    if let Some(token) = bearer {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let result = match body {
        Some(Body::Form(pairs)) => request.send_form(&pairs),
        Some(Body::Json(value)) => request.send_json(value),
        None => request.call(),
    };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(e) => return Err(format!("Could not reach {what}: {e}")),
    };
    let status = response.status();
    let value = response.into_json::<Value>().unwrap_or(Value::Null);
    Ok((status, value))
}

/// The OAuth error codes as messages for the player.
fn oauth_error(v: &Value) -> String {
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_declined") => "Sign-in was declined in the browser.".into(),
        Some("expired_token") => "The sign-in code expired. Start again.".into(),
        Some("bad_verification_code") => "The sign-in code was not recognised.".into(),
        Some("invalid_grant") => "The Microsoft sign-in expired. Sign in again.".into(),
        Some("invalid_client" | "unauthorized_client") => {
            "Microsoft refused this app ID (client ID). Check it, and that the app allows \
             personal accounts and public client flows."
                .into()
        }
        Some(other) => format!("Microsoft sign-in failed ({other})."),
        None => "Microsoft sign-in failed.".into(),
    }
}

pub fn device_code(ep: &Endpoints, client_id: &str) -> Result<DeviceCode, String> {
    let (status, v) = send(
        "POST",
        &format!("{}/devicecode", ep.microsoft),
        None,
        Some(Body::Form(vec![("client_id", client_id), ("scope", SCOPE)])),
        "Microsoft",
    )?;
    match (
        status,
        str_field(&v, "user_code"),
        str_field(&v, "device_code"),
    ) {
        (200, Some(user_code), Some(device_code)) => Ok(DeviceCode {
            user_code,
            verification_uri: str_field(&v, "verification_uri")
                .unwrap_or_else(|| "https://www.microsoft.com/link".into()),
            interval: v
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .clamp(1, 60),
            expires_in: v
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(900)
                .min(3600),
            device_code,
        }),
        _ => Err(oauth_error(&v)),
    }
}

fn tokens(status: u16, v: &Value) -> Result<Tokens, String> {
    match (
        status,
        str_field(v, "access_token"),
        str_field(v, "refresh_token"),
    ) {
        (200, Some(access), Some(refresh)) => Ok(Tokens { access, refresh }),
        _ => Err(oauth_error(v)),
    }
}

/// One poll of the token endpoint while the player signs in.
pub fn poll(ep: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Poll, String> {
    let (status, v) = send(
        "POST",
        &format!("{}/token", ep.microsoft),
        None,
        Some(Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", &code.device_code),
        ])),
        "Microsoft",
    )?;
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => Ok(Poll::Pending),
        Some("slow_down") => Ok(Poll::SlowDown),
        _ => tokens(status, &v).map(Poll::Done),
    }
}

/// New tokens from a refresh token.
pub fn refresh(ep: &Endpoints, client_id: &str, refresh_token: &str) -> Result<Tokens, String> {
    let (status, v) = send(
        "POST",
        &format!("{}/token", ep.microsoft),
        None,
        Some(Body::Form(vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ])),
        "Microsoft",
    )?;
    tokens(status, &v)
}

/// The XSTS `XErr` codes, as the official launcher explains them.
fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => "This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one.".into(),
        2148916235 => "Xbox Live is not available in your country or region.".into(),
        2148916236 | 2148916237 => "This account needs adult verification on xbox.com.".into(),
        2148916238 => "This is a child account: an adult has to add it to a Microsoft family first.".into(),
        other => format!("Xbox Live refused the sign-in (error {other})."),
    }
}

/// The Minecraft account behind Microsoft `tokens`: Xbox Live, XSTS, the Minecraft token, the
/// profile.
pub fn minecraft(ep: &Endpoints, tokens: Tokens) -> Result<Signed, String> {
    let (status, xbl) = send(
        "POST",
        &ep.xbox_user,
        None,
        Some(Body::Json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={}", tokens.access),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")
        .filter(|_| status == 200)
        .ok_or("Xbox Live refused the sign-in.")?;
    let (status, xsts) = send(
        "POST",
        &ep.xsts,
        None,
        Some(Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    if let Some(code) = xsts.get("XErr").and_then(Value::as_u64) {
        return Err(xsts_error(code));
    }
    let claims = xsts.pointer("/DisplayClaims/xui/0");
    let (Some(xsts_token), Some(uhs)) = (
        str_field(&xsts, "Token").filter(|_| status == 200),
        claims.and_then(|c| str_field(c, "uhs")),
    ) else {
        return Err("Xbox Live refused the sign-in.".into());
    };
    let xuid = claims.and_then(|c| str_field(c, "xid"));
    let access_token = minecraft_token(ep, &format!("XBL3.0 x={uhs};{xsts_token}"))?;
    let (status, profile) = send(
        "GET",
        &format!("{}/minecraft/profile", ep.minecraft),
        Some(&access_token),
        None,
        "the Minecraft services",
    )?;
    if status == 404 {
        return Err("This account has no Minecraft: Java Edition profile.".into());
    }
    let (Some(uuid), Some(name)) = (str_field(&profile, "id"), str_field(&profile, "name")) else {
        return Err("Could not read the Minecraft profile.".into());
    };
    Ok(Signed {
        name,
        uuid: uuid.replace('-', "").to_lowercase(),
        access_token,
        xuid,
        refresh: tokens.refresh,
    })
}

/// The Minecraft access token from the XSTS one (`XBL3.0 x=<uhs>;<token>`): `launcher/login`
/// first, as the official launcher does (a new account's `login_with_xbox` token names no
/// profile), `login_with_xbox` as the fallback.
fn minecraft_token(ep: &Endpoints, xtoken: &str) -> Result<String, String> {
    let launcher = send(
        "POST",
        &format!("{}/launcher/login", ep.minecraft),
        None,
        Some(Body::Json(
            json!({ "xtoken": xtoken, "platform": "PC_LAUNCHER" }),
        )),
        "the Minecraft services",
    );
    if let Ok((200, v)) = &launcher
        && let Some(token) = str_field(v, "access_token")
    {
        return Ok(token);
    }
    let (status, mc) = send(
        "POST",
        &format!("{}/authentication/login_with_xbox", ep.minecraft),
        None,
        Some(Body::Json(json!({ "identityToken": xtoken }))),
        "the Minecraft services",
    )?;
    match (status, str_field(&mc, "access_token")) {
        (200, Some(token)) => Ok(token),
        (403, _) => Err(
            "The Minecraft services refused this app ID: Mojang has to approve an app before \
             it can sign players in."
                .into(),
        ),
        (429, _) => Err("Too many sign-ins in a short time. Wait a minute and try again.".into()),
        _ => Err("The Minecraft services refused the sign-in.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    /// A server answering by the end of the request's path: `(suffix, status, body)`.
    fn serve(routes: Vec<(&'static str, u16, String)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut stream = stream;
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                let _ = reader.read_exact(&mut body);
                let (status, text) = routes
                    .iter()
                    .find(|(suffix, ..)| path.ends_with(suffix))
                    .map_or((404, "{}".to_owned()), |(_, s, b)| (*s, b.clone()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        base
    }

    fn endpoints(base: &str) -> Endpoints {
        Endpoints {
            microsoft: format!("{base}/ms"),
            xbox_user: format!("{base}/xbl"),
            xsts: format!("{base}/xsts"),
            minecraft: format!("{base}/mc"),
        }
    }

    #[test]
    fn the_device_code_flow_ends_in_a_minecraft_account() {
        let base = serve(vec![
            ("/ms/devicecode", 200, r#"{"user_code":"ABCD1234","device_code":"dc","verification_uri":"https://example.test/link","interval":1,"expires_in":900}"#.into()),
            ("/ms/token", 200, r#"{"access_token":"msa","refresh_token":"rt"}"#.into()),
            ("/xbl", 200, r#"{"Token":"xbl"}"#.into()),
            ("/xsts", 200, r#"{"Token":"xs","DisplayClaims":{"xui":[{"uhs":"hash","xid":"2535"}]}}"#.into()),
            ("/mc/launcher/login", 200, r#"{"access_token":"mc-token"}"#.into()),
            ("/mc/minecraft/profile", 200, r#"{"id":"069A79F4-44E9-4726-A5BE-FCA90E38AAF5","name":"Notch"}"#.into()),
        ]);
        let ep = endpoints(&base);
        let code = device_code(&ep, "client").unwrap();
        assert_eq!(code.user_code, "ABCD1234");
        assert_eq!(code.verification_uri, "https://example.test/link");
        let Poll::Done(t) = poll(&ep, "client", &code).unwrap() else {
            panic!("the mock approves at once");
        };
        let account = minecraft(&ep, t).unwrap();
        assert_eq!(account.name, "Notch");
        assert_eq!(account.uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(account.access_token, "mc-token");
        assert_eq!(account.xuid.as_deref(), Some("2535"));
        assert_eq!(account.refresh, "rt");
    }

    #[test]
    fn what_goes_wrong_reads_as_a_message() {
        let pending = serve(vec![(
            "/token",
            400,
            r#"{"error":"authorization_pending"}"#.into(),
        )]);
        let code = DeviceCode {
            user_code: String::new(),
            verification_uri: String::new(),
            interval: 1,
            expires_in: 1,
            device_code: "dc".into(),
        };
        assert!(matches!(
            poll(&endpoints(&format!("{pending}")), "c", &code),
            Ok(Poll::Pending)
        ));
        let declined = serve(vec![(
            "/token",
            400,
            r#"{"error":"authorization_declined"}"#.into(),
        )]);
        let err = poll(&endpoints(&declined), "c", &code).err().unwrap();
        assert!(err.contains("declined"), "{err}");
        let blocked = serve(vec![
            ("/xbl", 200, r#"{"Token":"x"}"#.into()),
            (
                "/xsts",
                200,
                r#"{"Token":"y","DisplayClaims":{"xui":[{"uhs":"h"}]}}"#.into(),
            ),
            ("/mc/launcher/login", 403, "{}".into()),
            ("/mc/authentication/login_with_xbox", 403, "{}".into()),
        ]);
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
        };
        let err = minecraft(&endpoints(&blocked), t).unwrap_err();
        assert!(err.contains("approve"), "{err}");
        let no_xbox = serve(vec![
            ("/xbl", 200, r#"{"Token":"x"}"#.into()),
            ("/xsts", 401, r#"{"XErr":2148916233}"#.into()),
        ]);
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
        };
        let err = minecraft(&endpoints(&no_xbox), t).unwrap_err();
        assert!(err.contains("no Xbox profile"), "{err}");
    }
}
