use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Sha1,
    Md5,
    Sha256,
    Sha512,
    Plain,
    Vahash,
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: String,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    pub password: PasswordConfig,
    pub world: WorldConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub premium: PremiumConfig,
    pub vocations: VocationsConfig,
}

#[derive(Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: String,
    pub name: String,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    #[serde(default = "default_accounts_table")]
    pub accounts_table: String,
    #[serde(default = "default_players_table")]
    pub players_table: String,
}

#[derive(Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_refresh_secs")]
    pub account_refresh_secs: u64,
    #[serde(default = "default_character_ttl")]
    pub characters_ttl_secs: u64,
}

#[derive(Deserialize)]
pub struct PasswordConfig {
    pub algorithm: Algorithm,
    /// Appended after the password, same as MyAAC `database_salt` / otserv password salt.
    #[serde(default)]
    pub database_salt: String,
}

#[derive(Deserialize)]
pub struct WorldConfig {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub ip: String,
    pub port: u16,
    #[serde(default = "default_location")]
    pub location: String,
    /// `pvp`, `no-pvp`, or `pvp-enforced`.
    #[serde(default = "default_pvp")]
    pub pvp_type: String,
}

#[derive(Deserialize)]
pub struct SecurityConfig {
    #[serde(default)]
    pub email_verify: bool,
    #[serde(default = "default_true")]
    pub ip_ban: bool,
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_ban_minutes")]
    pub ban_minutes: u64,
    /// Trust `X-Real-IP` and `X-Forwarded-For` from nginx.
    #[serde(default = "default_true")]
    pub trust_proxy: bool,
}

#[derive(Deserialize)]
pub struct PremiumConfig {
    /// Canary / TFS with `server_config.db_version >= 36`: `lastday` is the premium end timestamp.
    /// Ignored when `accounts.premium_ends_at` exists.
    #[serde(default)]
    pub lastday_is_premium_end: bool,
}

#[derive(Deserialize)]
pub struct VocationsConfig {
    pub amount: i64,
    pub names: Vec<String>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            account_refresh_secs: default_refresh_secs(),
            characters_ttl_secs: default_character_ttl(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            email_verify: false,
            ip_ban: true,
            max_attempts: default_attempts(),
            ban_minutes: default_ban_minutes(),
            trust_proxy: true,
        }
    }
}

impl Default for PremiumConfig {
    fn default() -> Self {
        Self {
            lastday_is_premium_end: false,
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:8081".to_string()
}

fn default_port() -> u16 {
    3306
}

fn default_pool_size() -> usize {
    8
}

fn default_accounts_table() -> String {
    "accounts".to_string()
}

fn default_players_table() -> String {
    "players".to_string()
}

fn default_refresh_secs() -> u64 {
    1
}

fn default_character_ttl() -> u64 {
    5
}

fn default_location() -> String {
    "BRA".to_string()
}

fn default_pvp() -> String {
    "pvp".to_string()
}

fn default_true() -> bool {
    true
}

fn default_attempts() -> u32 {
    5
}

fn default_ban_minutes() -> u64 {
    30
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.listen
            .parse::<SocketAddr>()
            .with_context(|| format!("listen address {}", self.listen))?;
        if self.database.pool_size == 0 {
            bail!("database.pool_size must be >= 1");
        }
        if self.cache.account_refresh_secs == 0 {
            bail!("cache.account_refresh_secs must be >= 1");
        }
        if self.cache.characters_ttl_secs == 0 {
            bail!("cache.characters_ttl_secs must be >= 1");
        }
        if self.security.ip_ban && self.security.max_attempts == 0 {
            bail!("security.max_attempts must be >= 1");
        }
        safe_ident(&self.database.accounts_table)?;
        safe_ident(&self.database.players_table)?;
        if self.vocations.names.is_empty() {
            bail!("vocations.names must not be empty");
        }
        if self.vocations.amount < 0 {
            bail!("vocations.amount must be >= 0");
        }
        Ok(())
    }
}

pub fn safe_ident(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        bail!("unsafe SQL identifier `{name}`");
    }
    Ok(())
}

pub fn config_path() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    if let Ok(env) = std::env::var("LOGIN_CONFIG") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    PathBuf::from("config.toml")
}

pub fn load() -> anyhow::Result<Arc<Config>> {
    let path = config_path();
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read config {}", path.display()))?;
    let cfg: Config = toml::from_str(&text)
        .with_context(|| format!("parse config {}", path.display()))?;
    cfg.validate()?;
    Ok(Arc::new(cfg))
}
