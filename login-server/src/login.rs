use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::accounts::AccountCache;
use crate::characters::CharacterCache;
use crate::config::Config;
use crate::password;
use crate::premium::{self, PremiumMode};
use crate::ratelimit::RateLimiter;
use crate::schema::Schema;
use crate::totp;

pub struct App {
    pub cfg: Arc<Config>,
    pub accounts: Arc<AccountCache>,
    pub characters: Arc<CharacterCache>,
    pub limiter: Arc<RateLimiter>,
    pub schema: Arc<Schema>,
    pub premium_mode: PremiumMode,
    pub world_json: Vec<u8>,
}

#[derive(Serialize)]
struct Session {
    sessionkey: String,
    lastlogintime: i32,
    ispremium: bool,
    premiumuntil: i64,
    status: &'static str,
    returnernotification: bool,
    showrewardnews: bool,
    isreturner: bool,
    fpstracking: bool,
    optiontracking: bool,
    tournamentticketpurchasestate: i32,
    emailcoderequest: bool,
}

impl Session {
    fn new(sessionkey: String, ispremium: bool, premiumuntil: i64) -> Self {
        Self {
            sessionkey,
            lastlogintime: 0,
            ispremium,
            premiumuntil,
            status: "active",
            returnernotification: false,
            showrewardnews: true,
            isreturner: true,
            fpstracking: false,
            optiontracking: false,
            tournamentticketpurchasestate: 0,
            emailcoderequest: false,
        }
    }
}

#[derive(Serialize)]
struct World {
    id: i64,
    name: String,
    externaladdress: String,
    externalport: u16,
    externaladdressprotected: String,
    externalportprotected: u16,
    externaladdressunprotected: String,
    externalportunprotected: u16,
    previewstate: i32,
    location: String,
    anticheatprotection: bool,
    pvptype: i32,
    istournamentworld: bool,
    restrictedstore: bool,
    currenttournamentphase: i32,
}

#[derive(serde::Deserialize)]
struct LoginBody {
    #[serde(rename = "type")]
    kind: Option<Value>,
    email: Option<Value>,
    accountname: Option<Value>,
    password: Option<Value>,
    token: Option<Value>,
}

enum Ident {
    Email(String),
    Name(String),
}

impl Ident {
    fn as_str(&self) -> &str {
        match self {
            Self::Email(value) | Self::Name(value) => value,
        }
    }

    fn bad_credentials(&self) -> &'static str {
        match self {
            Self::Email(_) => "Email or password is not correct.",
            Self::Name(_) => "Account name or password is not correct.",
        }
    }
}

pub fn world_json(cfg: &Config) -> anyhow::Result<Vec<u8>> {
    let world = World {
        id: cfg.world.id,
        name: cfg.world.name.clone(),
        externaladdress: cfg.world.ip.clone(),
        externalport: cfg.world.port,
        externaladdressprotected: cfg.world.ip.clone(),
        externalportprotected: cfg.world.port,
        externaladdressunprotected: cfg.world.ip.clone(),
        externalportunprotected: cfg.world.port,
        previewstate: 0,
        location: cfg.world.location.clone(),
        anticheatprotection: false,
        pvptype: pvp_type_id(&cfg.world.pvp_type),
        istournamentworld: false,
        restrictedstore: false,
        currenttournamentphase: 2,
    };
    Ok(serde_json::to_vec(&[world])?)
}

pub fn pvp_type_id(value: &str) -> i32 {
    match value {
        "pvp" => 0,
        "no-pvp" => 1,
        "pvp-enforced" => 2,
        _ => 0,
    }
}

pub(crate) fn build_session_key(
    identity: &str,
    password: &str,
    token: &str,
    tfs_session_key: bool,
    now: i64,
) -> String {
    let mut key = String::with_capacity(identity.len() + password.len() + token.len() + 24);
    key.push_str(identity);
    key.push('\n');
    key.push_str(password);
    if tfs_session_key {
        key.push('\n');
    }
    key.push_str(token);
    if tfs_session_key {
        key.push('\n');
        key.push_str(&now.div_euclid(30).to_string());
    }
    key
}

fn success_body(session: &Session, world_json: &[u8], characters_json: &[u8]) -> Vec<u8> {
    let session_json = serde_json::to_vec(session).expect("session json");
    let mut out =
        Vec::with_capacity(64 + session_json.len() + world_json.len() + characters_json.len());
    out.extend_from_slice(b"{\"session\":");
    out.extend_from_slice(&session_json);
    out.extend_from_slice(b",\"playdata\":{\"worlds\":");
    out.extend_from_slice(world_json);
    out.extend_from_slice(b",\"characters\":");
    out.extend_from_slice(characters_json);
    out.extend_from_slice(b"}}");
    out
}

pub async fn serve(app: Arc<App>) -> anyhow::Result<()> {
    let addr: SocketAddr = app.cfg.listen.parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    let router = Router::new()
        .route("/health", get(health))
        .fallback(login)
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(app);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn health(State(app): State<Arc<App>>) -> Response {
    let snapshot = app.accounts.load_full();
    let body = serde_json::json!({
        "accounts": snapshot.accounts.len(),
        "snapshot_age_ms": snapshot.loaded_at.elapsed().as_millis() as u64,
        "character_queries": app.characters.query_count(),
    });
    json_response(body)
}

async fn login(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if method != Method::POST {
        return error_response(3, "Invalid request.");
    }
    let ip = client_ip(&headers, peer, app.cfg.security.trust_proxy);
    let request: LoginBody = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return error_response(3, "Invalid request."),
    };
    let action = scalar_string(&request.kind).unwrap_or_default();
    if action != "login" {
        return error_response(3, format!("Unrecognized event {action}."));
    }
    let Some(ident) = identity(&request) else {
        return error_response(3, "Account name or password is not correct.");
    };
    let password = scalar_string(&request.password).unwrap_or_default();
    let account = {
        let snapshot = app.accounts.load_full();
        match &ident {
            Ident::Email(email) => snapshot.by_email(email).cloned(),
            Ident::Name(name) => snapshot.by_name(name).cloned(),
        }
    };
    let Some(account) = account else {
        return fail(&app, &ip, 3, ident.bad_credentials());
    };
    let hashed = password::hash_password(&account.salt, &password, &app.cfg.password);
    if !password::hashes_equal(&hashed, &account.password) {
        return fail(&app, &ip, 3, ident.bad_credentials());
    }
    if !account.secret.is_empty() {
        match scalar_string(&request.token) {
            None => {
                return fail(
                    &app,
                    &ip,
                    6,
                    "Submit a valid two-factor authentication token.",
                );
            }
            Some(ref token) => {
                if !totp::verify(&account.secret, token) {
                    return fail(
                        &app,
                        &ip,
                        6,
                        "Two-factor authentication failed, token is wrong.",
                    );
                }
            }
        }
    }
    app.limiter.reset(&ip);
    if app.cfg.security.email_verify
        && app.schema.has_email_verified
        && account.email_verified != Some(1)
    {
        return error_response(
            3,
            "You need to verify your account, enter in our site and resend verify e-mail!",
        );
    }
    let characters = match Arc::clone(&app.characters).get(account.id).await {
        Ok(value) => value,
        Err(message) => return error_response(3, message.as_ref()),
    };
    let now = chrono::Utc::now().timestamp();
    let token_part = if account.secret.len() > 5 {
        scalar_string(&request.token).unwrap_or_default()
    } else {
        String::new()
    };
    let session_key = build_session_key(
        ident.as_str(),
        &password,
        &token_part,
        app.schema.tfs_session_key,
        now,
    );
    let premium = account.premium_input();
    let days = premium::premium_days(&premium, app.premium_mode, now);
    let session = Session::new(
        session_key,
        premium::is_premium(&premium, app.premium_mode, now),
        premium::premium_until(days, now),
    );
    bytes_response(success_body(&session, &app.world_json, &characters.json))
}

fn identity(request: &LoginBody) -> Option<Ident> {
    if let Some(email) = scalar_string(&request.email) {
        if !email.is_empty() {
            return Some(Ident::Email(email));
        }
    }
    if let Some(name) = scalar_string(&request.accountname) {
        if !name.is_empty() {
            return Some(Ident::Name(name));
        }
    }
    None
}

fn scalar_string(value: &Option<Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Bool(true)) => Some("1".to_string()),
        Some(Value::Array(_) | Value::Object(_)) => None,
    }
}

fn fail(app: &App, ip: &str, code: i32, message: &str) -> Response {
    if app.limiter.record_failure(ip) {
        return error_response(3, app.limiter.ban_message());
    }
    error_response(code, message)
}

pub(crate) fn client_ip(headers: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(ip) = header_ip(headers, "x-real-ip") {
            return ip;
        }
        if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = forwarded.split(',').next() {
                let first = first.trim();
                if !first.is_empty() {
                    return first.to_string();
                }
            }
        }
    }
    peer.ip().to_string()
}

fn header_ip(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn error_response(code: i32, message: impl AsRef<str>) -> Response {
    json_response(serde_json::json!({
        "errorCode": code,
        "errorMessage": message.as_ref(),
    }))
}

fn json_response(value: Value) -> Response {
    bytes_response(serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()))
}

fn bytes_response(body: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .expect("response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_matches_php() {
        assert_eq!(build_session_key("e", "p", "", true, 90), "e\np\n\n3");
        assert_eq!(
            build_session_key("e", "p", "123", true, 30),
            "e\np\n123\n1"
        );
        assert_eq!(build_session_key("e", "p", "123", false, 30), "e\np123");
        assert_eq!(build_session_key("name", "p", "", false, 30), "name\np");
    }

    #[test]
    fn success_body_is_json() {
        let session = Session::new("a\nb".to_string(), false, 0);
        let body = success_body(&session, br#"[{"id":0}]"#, b"[]");
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["session"]["sessionkey"], "a\nb");
        assert_eq!(value["session"]["ispremium"], false);
        assert_eq!(value["playdata"]["worlds"][0]["id"], 0);
        assert_eq!(value["playdata"]["characters"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn proxy_ip_prefers_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "1.2.3.4".parse().unwrap());
        headers.insert("x-forwarded-for", "5.5.5.5, 6.6.6.6".parse().unwrap());
        let peer: SocketAddr = "9.9.9.9:1234".parse().unwrap();
        assert_eq!(client_ip(&headers, peer, true), "1.2.3.4");
        assert_eq!(client_ip(&headers, peer, false), "9.9.9.9");
        headers.remove("x-real-ip");
        assert_eq!(client_ip(&headers, peer, true), "5.5.5.5");
    }

    #[test]
    fn pvp_types() {
        assert_eq!(pvp_type_id("pvp"), 0);
        assert_eq!(pvp_type_id("no-pvp"), 1);
        assert_eq!(pvp_type_id("pvp-enforced"), 2);
        assert_eq!(pvp_type_id("other"), 0);
    }
}
