mod accounts;
mod characters;
mod config;
mod db;
mod login;
mod password;
mod premium;
mod ratelimit;
mod schema;
mod totp;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use tracing::{info, warn};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = config::load()?;
    let (conn, schema, snapshot, watermark) = wait_for_snapshot(&cfg);
    let loaded = snapshot.accounts.len();
    let schema = Arc::new(schema);
    let premium_mode = premium::resolve(schema.as_ref(), cfg.premium.lastday_is_premium_end);
    info!(
        accounts = loaded,
        incremental = schema.has_updated_at,
        tfs_session_key = schema.tfs_session_key,
        premium = ?premium_mode,
        "loaded account snapshot"
    );
    let published = Arc::new(snapshot);
    let account_cache = Arc::new(accounts::AccountCache::new(Arc::clone(&published)));
    accounts::spawn_refresh(
        conn,
        Arc::clone(&account_cache),
        Arc::clone(&schema),
        Arc::clone(&cfg),
        published,
        watermark,
    )?;
    let pool = db::pool(&cfg)?;
    let characters = Arc::new(characters::CharacterCache::new(
        pool,
        Arc::clone(&schema),
        Arc::clone(&cfg),
    ));
    let limiter = Arc::new(ratelimit::RateLimiter::new(
        cfg.security.ip_ban,
        cfg.security.max_attempts,
        cfg.security.ban_minutes,
    ));
    let world_json = login::world_json(&cfg)?;
    let worker_threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    info!(worker_threads, "tokio worker threads");
    let app = Arc::new(login::App {
        cfg: Arc::clone(&cfg),
        accounts: account_cache,
        characters,
        limiter,
        schema,
        premium_mode,
        world_json,
    });
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .context("tokio runtime")?
        .block_on(login::serve(app))
}

fn wait_for_snapshot(
    cfg: &Arc<config::Config>,
) -> (
    mysql::Conn,
    schema::Schema,
    accounts::Snapshot,
    Option<String>,
) {
    loop {
        match load_once(cfg) {
            Ok(loaded) => return loaded,
            Err(err) => {
                warn!(error = %err, "waiting for database");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn load_once(
    cfg: &config::Config,
) -> anyhow::Result<(
    mysql::Conn,
    schema::Schema,
    accounts::Snapshot,
    Option<String>,
)> {
    let mut conn = db::open(cfg)?;
    let detected = schema::detect(&mut conn, cfg)?;
    let initial = accounts::load_initial(&mut conn, &detected)?;
    Ok((conn, detected, initial.snapshot, initial.watermark))
}
