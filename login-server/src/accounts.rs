use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use arc_swap::ArcSwap;
use mysql::prelude::Queryable;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::db;
use crate::premium::PremiumInput;
use crate::schema::Schema;

#[derive(Clone)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub salt: String,
    pub secret: String,
    pub email_verified: Option<i64>,
    pub premium_ends_at: Option<i64>,
    pub premend: Option<i64>,
    pub lastday: Option<i64>,
    pub premdays: Option<i64>,
}

impl Account {
    pub fn premium_input(&self) -> PremiumInput {
        PremiumInput {
            premium_ends_at: self.premium_ends_at,
            premend: self.premend,
            lastday: self.lastday,
            premdays: self.premdays,
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub accounts: Vec<Account>,
    by_id: HashMap<i64, usize>,
    by_name: HashMap<String, usize>,
    by_email: HashMap<String, usize>,
    pub loaded_at: Instant,
}

impl Snapshot {
    pub fn by_email(&self, email: &str) -> Option<&Account> {
        self.by_email
            .get(&email.to_lowercase())
            .map(|&index| &self.accounts[index])
    }

    pub fn by_name(&self, name: &str) -> Option<&Account> {
        self.by_name
            .get(&name.to_lowercase())
            .map(|&index| &self.accounts[index])
    }
}

/// Immutable account list swapped as a whole. Readers keep the `Arc` they loaded
/// until they drop it. The refresh thread clones that list itself, then publishes
/// the new one with a pointer swap.
pub struct AccountCache {
    current: ArcSwap<Snapshot>,
}

impl AccountCache {
    pub fn new(initial: Arc<Snapshot>) -> Self {
        Self {
            current: ArcSwap::from(initial),
        }
    }

    pub fn store(&self, snapshot: Arc<Snapshot>) {
        self.current.store(snapshot);
    }

    pub fn load_full(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }
}

pub struct InitialLoad {
    pub snapshot: Snapshot,
    /// MySQL time taken before the read. `None` when `updated_at` is missing and
    /// every refresh has to scan the whole table.
    pub watermark: Option<String>,
}

pub fn load_initial(conn: &mut impl Queryable, schema: &Schema) -> anyhow::Result<InitialLoad> {
    let watermark = if schema.has_updated_at {
        Some(mysql_now(conn)?)
    } else {
        warn!("accounts.updated_at is missing; refreshing the full account list");
        None
    };
    let snapshot = load_all(conn, schema)?;
    Ok(InitialLoad {
        snapshot,
        watermark,
    })
}

fn load_all(conn: &mut impl Queryable, schema: &Schema) -> anyhow::Result<Snapshot> {
    let mut snapshot = Snapshot::empty();
    let result = conn
        .query_iter(&schema.account_sql)
        .context("query accounts")?;
    for row in result {
        let row = row.context("read account row")?;
        if let Some(account) = read_account(&row, schema) {
            snapshot.insert_new(account);
        }
    }
    Ok(snapshot)
}

fn read_account(row: &mysql::Row, schema: &Schema) -> Option<Account> {
    let id = db::row_i64(row, "id")?;
    let name = if schema.has_name {
        db::row_string(row, "name")
    } else {
        String::new()
    };
    let email = if schema.has_email {
        db::row_string(row, "email")
    } else {
        String::new()
    };
    Some(Account {
        id,
        password: db::row_string(row, "password"),
        salt: if schema.has_salt {
            db::row_string(row, "salt")
        } else {
            String::new()
        },
        secret: if schema.has_secret {
            db::row_string(row, "secret")
        } else {
            String::new()
        },
        email_verified: optional_i64(row, "email_verified", schema.has_email_verified),
        premium_ends_at: optional_i64(row, "premium_ends_at", schema.has_premium_ends_at),
        premend: optional_i64(row, "premend", schema.has_premend),
        lastday: optional_i64(row, "lastday", schema.has_lastday),
        premdays: optional_i64(row, "premdays", schema.has_premdays),
        name,
        email,
    })
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            accounts: Vec::new(),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            by_email: HashMap::new(),
            loaded_at: Instant::now(),
        }
    }

    fn insert_new(&mut self, account: Account) {
        let index = self.accounts.len();
        let name_key = account.name.to_lowercase();
        let email_key = account.email.to_lowercase();
        self.by_id.insert(account.id, index);
        if !name_key.is_empty() {
            self.by_name.entry(name_key).or_insert(index);
        }
        if !email_key.is_empty() {
            self.by_email.entry(email_key).or_insert(index);
        }
        self.accounts.push(account);
    }

    fn upsert(&mut self, account: Account) {
        if self.by_id.contains_key(&account.id) {
            self.replace_existing(account);
        } else {
            self.insert_new(account);
        }
    }

    fn replace_existing(&mut self, account: Account) {
        let index = self.by_id[&account.id];
        let old_name = self.accounts[index].name.to_lowercase();
        let old_email = self.accounts[index].email.to_lowercase();
        let name_key = account.name.to_lowercase();
        let email_key = account.email.to_lowercase();
        if old_name != name_key && self.by_name.get(&old_name).copied() == Some(index) {
            self.by_name.remove(&old_name);
        }
        if old_email != email_key && self.by_email.get(&old_email).copied() == Some(index) {
            self.by_email.remove(&old_email);
        }
        self.accounts[index] = account;
        if !name_key.is_empty() {
            self.by_name.insert(name_key, index);
        }
        if !email_key.is_empty() {
            self.by_email.insert(email_key, index);
        }
    }

    fn remove_id(&mut self, id: i64) {
        let Some(index) = self.by_id.remove(&id) else {
            return;
        };
        let removed = self.accounts.swap_remove(index);
        let removed_name = removed.name.to_lowercase();
        let removed_email = removed.email.to_lowercase();
        if self.by_name.get(&removed_name).copied() == Some(index) {
            self.by_name.remove(&removed_name);
        }
        if self.by_email.get(&removed_email).copied() == Some(index) {
            self.by_email.remove(&removed_email);
        }
        let swapped_from = self.accounts.len();
        if index < self.accounts.len() {
            let swapped = &self.accounts[index];
            self.by_id.insert(swapped.id, index);
            let name = swapped.name.to_lowercase();
            let email = swapped.email.to_lowercase();
            if self.by_name.get(&name).copied() == Some(swapped_from) {
                self.by_name.insert(name, index);
            }
            if self.by_email.get(&email).copied() == Some(swapped_from) {
                self.by_email.insert(email, index);
            }
        }
    }
}

fn optional_i64(row: &mysql::Row, column: &str, enabled: bool) -> Option<i64> {
    if !enabled {
        return None;
    }
    Some(db::row_i64(row, column).unwrap_or(0))
}

pub fn spawn_refresh(
    conn: mysql::Conn,
    cache: Arc<AccountCache>,
    schema: Arc<Schema>,
    cfg: Arc<Config>,
    published: Arc<Snapshot>,
    watermark: Option<String>,
) -> anyhow::Result<()> {
    let secs = cfg.cache.account_refresh_secs;
    thread::Builder::new()
        .name("account-refresh".into())
        .spawn(move || refresh_loop(conn, cache, schema, cfg, secs, published, watermark))
        .context("spawn account refresh thread")?;
    Ok(())
}

fn refresh_loop(
    mut conn: mysql::Conn,
    cache: Arc<AccountCache>,
    schema: Arc<Schema>,
    cfg: Arc<Config>,
    secs: u64,
    mut published: Arc<Snapshot>,
    mut watermark: Option<String>,
) {
    let interval = Duration::from_secs(secs);
    loop {
        thread::sleep(interval);
        match refresh_once(&mut conn, &schema, &published, watermark.as_deref()) {
            Ok(Refresh::Unchanged { watermark: next }) => {
                watermark = Some(next);
            }
            Ok(Refresh::Replaced {
                snapshot,
                watermark: next,
            }) => {
                debug!(accounts = snapshot.accounts.len(), "refreshed accounts");
                published = snapshot;
                cache.store(Arc::clone(&published));
                watermark = next;
            }
            Err(err) => {
                warn!(error = %err, "account refresh failed");
                if let Ok(next) = db::open(&cfg) {
                    conn = next;
                }
                // The failed read may have missed rows. Rebuild from the whole table.
                watermark = None;
            }
        }
    }
}

enum Refresh {
    Unchanged { watermark: String },
    Replaced {
        snapshot: Arc<Snapshot>,
        watermark: Option<String>,
    },
}

fn refresh_once(
    conn: &mut impl Queryable,
    schema: &Schema,
    published: &Arc<Snapshot>,
    watermark: Option<&str>,
) -> anyhow::Result<Refresh> {
    let Some(delta_sql) = schema.account_delta_sql.as_deref() else {
        let snapshot = Arc::new(load_all(conn, schema)?);
        info!(accounts = snapshot.accounts.len(), "reloaded all accounts");
        return Ok(Refresh::Replaced {
            snapshot,
            watermark: None,
        });
    };
    let Some(since) = watermark else {
        let as_of = mysql_now(conn)?;
        let snapshot = Arc::new(load_all(conn, schema)?);
        info!(accounts = snapshot.accounts.len(), "reloaded all accounts");
        return Ok(Refresh::Replaced {
            snapshot,
            watermark: Some(as_of),
        });
    };

    let as_of = mysql_now(conn)?;
    let changed = load_changed(conn, delta_sql, schema, since, &as_of)?;
    let removed = load_removed(conn, schema.removed_sql.as_deref(), since, &as_of)?;
    if changed.is_empty() && removed.is_empty() {
        return Ok(Refresh::Unchanged { watermark: as_of });
    }

    // Copy the published list in this thread, then hand the new Arc to logins.
    let mut built = (**published).clone();
    let changed_count = changed.len();
    let removed_count = removed.len();
    for account in changed {
        built.upsert(account);
    }
    for id in removed {
        built.remove_id(id);
    }
    built.loaded_at = Instant::now();
    info!(
        changed = changed_count,
        removed = removed_count,
        accounts = built.accounts.len(),
        "applied account delta"
    );
    if schema.removed_sql.is_some() && removed_count > 0 {
        conn.exec_drop(
            "DELETE FROM `accounts_removed` WHERE `updated_at` <= ?",
            (&as_of,),
        )
        .context("trim removed accounts")?;
    }
    Ok(Refresh::Replaced {
        snapshot: Arc::new(built),
        watermark: Some(as_of),
    })
}

fn load_changed(
    conn: &mut impl Queryable,
    sql: &str,
    schema: &Schema,
    since: &str,
    as_of: &str,
) -> anyhow::Result<Vec<Account>> {
    let rows: Vec<mysql::Row> = conn
        .exec(sql, (since, as_of))
        .context("query changed accounts")?;
    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(account) = read_account(&row, schema) {
            accounts.push(account);
        }
    }
    Ok(accounts)
}

fn load_removed(
    conn: &mut impl Queryable,
    sql: Option<&str>,
    since: &str,
    as_of: &str,
) -> anyhow::Result<Vec<i64>> {
    let Some(sql) = sql else {
        return Ok(Vec::new());
    };
    let rows: Vec<mysql::Row> = conn
        .exec(sql, (since, as_of))
        .context("query removed accounts")?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(id) = db::row_i64(&row, "account_id") {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn mysql_now(conn: &mut impl Queryable) -> anyhow::Result<String> {
    let value: Option<String> = conn
        .query_first("SELECT DATE_FORMAT(NOW(6), '%Y-%m-%d %H:%i:%s.%f')")
        .context("mysql time")?;
    value.context("mysql time was empty")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: i64, name: &str, email: &str) -> Account {
        Account {
            id,
            name: name.to_string(),
            email: email.to_string(),
            password: "x".to_string(),
            salt: String::new(),
            secret: String::new(),
            email_verified: None,
            premium_ends_at: None,
            premend: None,
            lastday: None,
            premdays: None,
        }
    }

    #[test]
    fn remove_keeps_remaining_lookups() {
        let mut snapshot = Snapshot::empty();
        snapshot.insert_new(account(1, "a", "a@b"));
        snapshot.insert_new(account(2, "b", "b@b"));
        snapshot.insert_new(account(3, "c", "c@b"));
        snapshot.remove_id(1);
        assert_eq!(snapshot.by_name("c").unwrap().id, 3);
        assert_eq!(snapshot.by_name("b").unwrap().id, 2);
        assert!(snapshot.by_name("a").is_none());
        assert_eq!(snapshot.accounts.len(), 2);
    }

    #[test]
    fn replace_moves_name_and_email() {
        let mut snapshot = Snapshot::empty();
        snapshot.insert_new(account(1, "old", "old@b"));
        snapshot.upsert(account(1, "new", "new@b"));
        assert!(snapshot.by_name("old").is_none());
        assert_eq!(snapshot.by_name("new").unwrap().id, 1);
        assert!(snapshot.by_email("old@b").is_none());
        assert_eq!(snapshot.by_email("new@b").unwrap().id, 1);
    }
}
