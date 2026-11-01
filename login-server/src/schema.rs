use std::collections::HashSet;

use anyhow::{bail, Context};
use mysql::prelude::Queryable;

use crate::config::Config;
use crate::db;

#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerFlags {
    pub experience: bool,
    pub promotion: bool,
    pub isreward: bool,
    pub istutorial: bool,
}

#[derive(Debug)]
pub struct Schema {
    pub has_name: bool,
    pub has_email: bool,
    pub has_salt: bool,
    pub has_secret: bool,
    pub has_email_verified: bool,
    pub has_premium_ends_at: bool,
    pub has_premend: bool,
    pub has_lastday: bool,
    pub has_premdays: bool,
    pub has_updated_at: bool,
    pub account_sql: String,
    /// Same columns as `account_sql`, limited to rows changed after the previous read.
    pub account_delta_sql: Option<String>,
    pub removed_sql: Option<String>,
    pub player_sql: String,
    pub player: PlayerFlags,
    /// TFS 1.x session key (extra newline and `floor(time/30)`) when `players.istutorial` is absent.
    pub tfs_session_key: bool,
}

pub fn detect(conn: &mut impl Queryable, cfg: &Config) -> anyhow::Result<Schema> {
    let accounts = column_set(conn, &cfg.database.accounts_table)?;
    if accounts.is_empty() {
        bail!("table `{}` was not found", cfg.database.accounts_table);
    }
    let players = column_set(conn, &cfg.database.players_table)?;
    if players.is_empty() {
        bail!("table `{}` was not found", cfg.database.players_table);
    }
    for column in ["id", "password"] {
        if !accounts.contains(column) {
            bail!("accounts.{column} column is required");
        }
    }
    let has_name = accounts.contains("name");
    let has_email = accounts.contains("email");
    if !has_name && !has_email {
        bail!("accounts table needs a name or email column");
    }
    for column in [
        "id",
        "name",
        "account_id",
        "level",
        "sex",
        "vocation",
        "looktype",
        "lookhead",
        "lookbody",
        "looklegs",
        "lookfeet",
        "lookaddons",
    ] {
        if !players.contains(column) {
            bail!("players.{column} column is required");
        }
    }

    let has_updated_at = accounts.contains("updated_at");
    let select_sql = account_select(&cfg.database.accounts_table, &accounts);
    let player = PlayerFlags {
        experience: players.contains("experience"),
        promotion: players.contains("promotion"),
        isreward: players.contains("isreward"),
        istutorial: players.contains("istutorial"),
    };

    Ok(Schema {
        has_name,
        has_email,
        has_salt: accounts.contains("salt"),
        has_secret: accounts.contains("secret"),
        has_email_verified: accounts.contains("email_verified"),
        has_premium_ends_at: accounts.contains("premium_ends_at"),
        has_premend: accounts.contains("premend"),
        has_lastday: accounts.contains("lastday"),
        has_premdays: accounts.contains("premdays"),
        has_updated_at,
        account_sql: format!("{select_sql} ORDER BY `id`"),
        account_delta_sql: has_updated_at.then(|| {
            format!("{select_sql} WHERE `updated_at` > ? AND `updated_at` <= ?")
        }),
        removed_sql: if table_exists(conn, "accounts_removed")? {
            Some(
                "SELECT `account_id` FROM `accounts_removed` WHERE `updated_at` > ? AND `updated_at` <= ?"
                    .to_string(),
            )
        } else {
            None
        },
        player_sql: player_sql(&cfg.database.players_table, &players, player),
        player,
        tfs_session_key: !player.istutorial,
    })
}

fn column_set(conn: &mut impl Queryable, table: &str) -> anyhow::Result<HashSet<String>> {
    let names = db::column_names(conn, table).with_context(|| format!("columns of `{table}`"))?;
    Ok(names.into_iter().collect())
}

fn table_exists(conn: &mut impl Queryable, table: &str) -> anyhow::Result<bool> {
    let row: Option<mysql::Row> = conn
        .exec_first(
            "SELECT COUNT(*) AS n FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            (table,),
        )
        .with_context(|| format!("check table `{table}`"))?;
    Ok(row.as_ref().and_then(|row| db::row_i64(row, "n")).unwrap_or(0) > 0)
}

fn account_select(table: &str, columns: &HashSet<String>) -> String {
    let mut selected = vec!["id", "password"];
    for column in [
        "name",
        "email",
        "salt",
        "secret",
        "email_verified",
        "premium_ends_at",
        "premend",
        "lastday",
        "premdays",
    ] {
        if columns.contains(column) {
            selected.push(column);
        }
    }
    let list = quote_list(&selected);
    format!("SELECT {list} FROM `{table}`")
}

fn player_sql(table: &str, columns: &HashSet<String>, flags: PlayerFlags) -> String {
    let mut selected = vec![
        "id",
        "name",
        "level",
        "sex",
        "vocation",
        "looktype",
        "lookhead",
        "lookbody",
        "looklegs",
        "lookfeet",
        "lookaddons",
    ];
    if flags.experience {
        selected.push("experience");
    }
    if flags.promotion {
        selected.push("promotion");
    }
    if flags.isreward {
        selected.push("isreward");
    }
    if flags.istutorial {
        selected.push("istutorial");
    }
    let filter = if columns.contains("deletion") {
        " AND `deletion` = 0"
    } else if columns.contains("deleted") {
        " AND `deleted` = 0"
    } else {
        ""
    };
    let list = quote_list(&selected);
    format!("SELECT {list} FROM `{table}` WHERE `account_id` = ?{filter}")
}

fn quote_list(columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| format!("`{column}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
