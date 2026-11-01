use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use dashmap::mapref::entry::Entry;
use futures::future::FutureExt;
use mysql::prelude::Queryable;
use mysql::Pool;
use serde::Serialize;
use tracing::warn;

use crate::config::Config;
use crate::db;
use crate::schema::{PlayerFlags, Schema};

type LoadResult = Result<Arc<CachedChars>, Arc<str>>;
type SharedLoad = futures::future::Shared<Pin<Box<dyn Future<Output = LoadResult> + Send>>>;

struct CacheEntry {
    expires: Instant,
    data: Arc<CachedChars>,
}

pub struct CachedChars {
    pub json: Vec<u8>,
}

struct Draft {
    id: i64,
    name: String,
    level: i64,
    sex: i64,
    vocation: i64,
    promotion: i64,
    experience: i64,
    looktype: i64,
    lookhead: i64,
    lookbody: i64,
    looklegs: i64,
    lookfeet: i64,
    lookaddons: i64,
    istutorial: bool,
    isreward: i64,
}

#[derive(Serialize)]
struct Character {
    worldid: i64,
    name: String,
    ismale: bool,
    tutorial: bool,
    level: i32,
    vocation: String,
    outfitid: i32,
    headcolor: i32,
    torsocolor: i32,
    legscolor: i32,
    detailcolor: i32,
    addonsflags: i32,
    ishidden: bool,
    istournamentparticipant: bool,
    ismaincharacter: bool,
    dailyrewardstate: i32,
    remainingdailytournamentplaytime: i32,
}

pub struct CharacterCache {
    pool: Pool,
    player_sql: String,
    flags: PlayerFlags,
    vocation_names: Arc<Vec<String>>,
    vocation_amount: i64,
    world_id: i64,
    ttl: Duration,
    cache: dashmap::DashMap<i64, CacheEntry>,
    inflight: dashmap::DashMap<i64, SharedLoad>,
    queries: AtomicU64,
}

impl CharacterCache {
    pub fn new(pool: Pool, schema: Arc<Schema>, cfg: Arc<Config>) -> Self {
        Self {
            pool,
            player_sql: schema.player_sql.clone(),
            flags: schema.player,
            vocation_names: Arc::new(cfg.vocations.names.clone()),
            vocation_amount: cfg.vocations.amount,
            world_id: cfg.world.id,
            ttl: Duration::from_secs(cfg.cache.characters_ttl_secs),
            cache: dashmap::DashMap::new(),
            inflight: dashmap::DashMap::new(),
            queries: AtomicU64::new(0),
        }
    }

    pub fn query_count(&self) -> u64 {
        self.queries.load(Ordering::Relaxed)
    }

    pub async fn get(self: Arc<Self>, account_id: i64) -> LoadResult {
        let fresh = self.cache.get(&account_id).and_then(|hit| {
            if hit.expires > Instant::now() {
                Some(Arc::clone(&hit.data))
            } else {
                None
            }
        });
        if let Some(data) = fresh {
            return Ok(data);
        }
        self.cache.remove(&account_id);
        let shared = match self.inflight.entry(account_id) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let this = Arc::clone(&self);
                let shared: SharedLoad = async move { this.fetch(account_id).await }.boxed().shared();
                entry.insert(shared.clone());
                shared
            }
        };
        shared.await
    }

    async fn fetch(&self, account_id: i64) -> LoadResult {
        let job = QueryJob {
            pool: self.pool.clone(),
            sql: self.player_sql.clone(),
            flags: self.flags,
            names: Arc::clone(&self.vocation_names),
            amount: self.vocation_amount,
            world_id: self.world_id,
            account_id,
        };
        self.queries.fetch_add(1, Ordering::Relaxed);
        let loaded = tokio::task::spawn_blocking(move || query_characters(job)).await;
        let result = match loaded {
            Ok(Ok(chars)) => Ok(Arc::new(chars)),
            Ok(Err(err)) => {
                warn!(account_id, error = %err, "character query failed");
                Err(Arc::<str>::from("Internal error."))
            }
            Err(err) => {
                warn!(account_id, error = %err, "character query task failed");
                Err(Arc::<str>::from("Internal error."))
            }
        };
        if let Ok(ref data) = result {
            self.cache.insert(
                account_id,
                CacheEntry {
                    expires: Instant::now() + self.ttl,
                    data: Arc::clone(data),
                },
            );
        }
        self.inflight.remove(&account_id);
        result
    }
}

struct QueryJob {
    pool: Pool,
    sql: String,
    flags: PlayerFlags,
    names: Arc<Vec<String>>,
    amount: i64,
    world_id: i64,
    account_id: i64,
}

fn query_characters(job: QueryJob) -> anyhow::Result<CachedChars> {
    let mut conn = job.pool.get_conn().context("mysql pool")?;
    let rows: Vec<mysql::Row> = conn
        .exec(job.sql.as_str(), (job.account_id,))
        .context("query players")?;
    let mut drafts = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(id) = db::row_i64(&row, "id") else {
            continue;
        };
        drafts.push(Draft {
            id,
            name: db::row_string(&row, "name"),
            level: db::row_i64(&row, "level").unwrap_or(1),
            sex: db::row_i64(&row, "sex").unwrap_or(0),
            vocation: db::row_i64(&row, "vocation").unwrap_or(0),
            promotion: if job.flags.promotion {
                db::row_i64(&row, "promotion").unwrap_or(0)
            } else {
                0
            },
            experience: if job.flags.experience {
                db::row_i64(&row, "experience").unwrap_or(0)
            } else {
                0
            },
            looktype: db::row_i64(&row, "looktype").unwrap_or(0),
            lookhead: db::row_i64(&row, "lookhead").unwrap_or(0),
            lookbody: db::row_i64(&row, "lookbody").unwrap_or(0),
            looklegs: db::row_i64(&row, "looklegs").unwrap_or(0),
            lookfeet: db::row_i64(&row, "lookfeet").unwrap_or(0),
            lookaddons: db::row_i64(&row, "lookaddons").unwrap_or(0),
            istutorial: job.flags.istutorial && db::row_i64(&row, "istutorial").unwrap_or(0) != 0,
            isreward: if job.flags.isreward {
                db::row_i64(&row, "isreward").unwrap_or(0)
            } else {
                0
            },
        });
    }
    drafts.sort_by(|left, right| {
        right
            .experience
            .cmp(&left.experience)
            .then(left.id.cmp(&right.id))
    });
    let main_id = drafts.first().map(|player| player.id);
    let characters: Vec<Character> = drafts
        .into_iter()
        .map(|player| Character {
            worldid: job.world_id,
            name: player.name,
            ismale: player.sex == 1,
            tutorial: player.istutorial,
            level: clamp_i32(player.level),
            vocation: vocation_name(player.vocation, player.promotion, job.amount, &job.names),
            outfitid: clamp_i32(player.looktype),
            headcolor: clamp_i32(player.lookhead),
            torsocolor: clamp_i32(player.lookbody),
            legscolor: clamp_i32(player.looklegs),
            detailcolor: clamp_i32(player.lookfeet),
            addonsflags: clamp_i32(player.lookaddons),
            ishidden: false,
            istournamentparticipant: false,
            ismaincharacter: Some(player.id) == main_id,
            dailyrewardstate: clamp_i32(player.isreward),
            remainingdailytournamentplaytime: 0,
        })
        .collect();
    let json = serde_json::to_vec(&characters).context("encode characters")?;
    Ok(CachedChars { json })
}

pub(crate) fn vocation_name(vocation: i64, promotion: i64, amount: i64, names: &[String]) -> String {
    let mut id = vocation;
    if promotion > 0 {
        id = id.saturating_add(promotion.saturating_mul(amount));
    }
    if id < 0 {
        return "Unknown".to_string();
    }
    names
        .get(id as usize)
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string())
}

fn clamp_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_shifts_vocation_id() {
        let names = ["None", "Sorcerer", "Druid", "Paladin", "Knight", "Master Sorcerer", "Elder Druid", "Royal Paladin", "Elite Knight"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(vocation_name(4, 1, 4, &names), "Elite Knight");
        assert_eq!(vocation_name(1, 0, 4, &names), "Sorcerer");
        assert_eq!(vocation_name(9, 0, 4, &names), "Unknown");
    }
}
