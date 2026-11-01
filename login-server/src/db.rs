use std::time::Duration;

use anyhow::Context;
use mysql::prelude::Queryable;
use mysql::{Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, Value};

use crate::config::Config;

pub fn mysql_opts(cfg: &Config) -> Opts {
    let constraints = PoolConstraints::new(1, cfg.database.pool_size).expect("pool size");
    let pool_opts = PoolOpts::new().with_constraints(constraints);
    Opts::from(
        OptsBuilder::new()
            .ip_or_hostname(Some(cfg.database.host.clone()))
            .tcp_port(cfg.database.port)
            .user(Some(cfg.database.user.clone()))
            .pass(Some(cfg.database.password.clone()))
            .db_name(Some(cfg.database.name.clone()))
            .prefer_socket(false)
            .tcp_connect_timeout(Some(Duration::from_secs(5)))
            .pool_opts(Some(pool_opts)),
    )
}

pub fn open(cfg: &Config) -> anyhow::Result<mysql::Conn> {
    mysql::Conn::new(mysql_opts(cfg)).with_context(|| {
        format!(
            "connect to mysql {}:{}/{}",
            cfg.database.host, cfg.database.port, cfg.database.name
        )
    })
}

pub fn pool(cfg: &Config) -> anyhow::Result<Pool> {
    Pool::new(mysql_opts(cfg)).with_context(|| {
        format!(
            "create mysql pool {}:{}/{}",
            cfg.database.host, cfg.database.port, cfg.database.name
        )
    })
}

pub fn column_names(conn: &mut impl Queryable, table: &str) -> anyhow::Result<Vec<String>> {
    let rows: Vec<mysql::Row> = conn
        .exec(
            "SELECT COLUMN_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            (table,),
        )
        .with_context(|| format!("read columns of `{table}`"))?;
    let mut names = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(value) = row_index(&row, 0) {
            names.push(value_string(value).to_ascii_lowercase());
        }
    }
    Ok(names)
}

pub fn row_index(row: &mysql::Row, index: usize) -> Option<Value> {
    row.get_opt(index).and_then(|result| result.ok())
}

pub fn row_string(row: &mysql::Row, column: &str) -> String {
    match row.get_opt::<Value, _>(column) {
        Some(Ok(value)) => value_string(value),
        _ => String::new(),
    }
}

pub fn row_i64(row: &mysql::Row, column: &str) -> Option<i64> {
    match row.get_opt::<Value, _>(column) {
        Some(Ok(Value::NULL)) | None => None,
        Some(Ok(value)) => value_i64(value),
        Some(Err(_)) => None,
    }
}

pub fn value_i64(value: Value) -> Option<i64> {
    match value {
        Value::Int(v) => Some(v),
        Value::UInt(v) => i64::try_from(v).ok(),
        Value::Bytes(v) => std::str::from_utf8(&v)
            .ok()
            .and_then(|text| text.trim().parse().ok()),
        Value::Float(v) => Some(v as i64),
        Value::Double(v) => Some(v as i64),
        _ => None,
    }
}

pub fn value_string(value: Value) -> String {
    match value {
        Value::Bytes(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        Value::NULL => String::new(),
        _ => String::new(),
    }
}
