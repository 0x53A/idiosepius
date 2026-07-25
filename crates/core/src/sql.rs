//! A synchronous façade over turso.
//!
//! turso — the Rust rewrite of SQLite — is async all the way down, and
//! idiosepius is not: the scheduler is a pure function, the store is shared as
//! `Rc<Store>`, and egui's frame callback is sync. Rather than colour the whole
//! crate `async` for a database that answers in microseconds off a local file,
//! every call blocks on a current-thread runtime kept next to the connection.
//!
//! The surface here is deliberately small and shaped like the queries the rest
//! of core actually writes: `execute`, one row, or all rows. Row closures
//! return `anyhow::Result`, so a decode failure (a corrupt JSON payload, say)
//! is just an error rather than something to smuggle out through the row type.

use anyhow::{Context, Result, anyhow};
use std::path::Path;
use tokio::runtime::Runtime;
use turso::Builder;

pub use turso::Value;

/// An open database, plus the runtime its futures are driven on.
pub struct Conn {
    rt: Runtime,
    inner: turso::Connection,
}

impl Conn {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let name = path
            .to_str()
            .ok_or_else(|| anyhow!("database path {} is not valid UTF-8", path.display()))?;
        Self::build(name).with_context(|| format!("opening database at {}", path.display()))
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::build(":memory:").context("opening an in-memory database")
    }

    fn build(name: &str) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .context("starting the database runtime")?;
        let inner = rt.block_on(async {
            let db = Builder::new_local(name).build().await?;
            db.connect()
        })?;
        Ok(Conn { rt, inner })
    }

    /// Number of rows changed.
    pub fn execute(&self, sql: &str, params: Vec<Value>) -> Result<usize> {
        let n = self
            .rt
            .block_on(self.inner.execute(sql, params))
            .with_context(|| failed(sql))?;
        Ok(n as usize)
    }

    /// Several statements at once, for `schema.sql`.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.rt
            .block_on(self.inner.execute_batch(sql))
            .context("running a statement batch")
    }

    /// The first row, or an error if the query produced none. Right for
    /// aggregates and lookups by primary key, which always answer.
    pub fn query_row<T>(
        &self,
        sql: &str,
        params: Vec<Value>,
        f: impl FnOnce(&Row) -> Result<T>,
    ) -> Result<T> {
        self.query_row_opt(sql, params, f)?
            .ok_or_else(|| anyhow!("query returned no rows: {}", first_line(sql)))
    }

    /// The first row if there is one. Replaces rusqlite's `.optional()`.
    pub fn query_row_opt<T>(
        &self,
        sql: &str,
        params: Vec<Value>,
        f: impl FnOnce(&Row) -> Result<T>,
    ) -> Result<Option<T>> {
        let rows = self.fetch(sql, params)?;
        rows.first().map(|r| f(r)).transpose()
    }

    /// Every row, mapped. Rows are read eagerly: the result sets here are a
    /// deck at a time, and holding them costs less than threading the async
    /// cursor's lifetime through every caller.
    pub fn query_all<T>(
        &self,
        sql: &str,
        params: Vec<Value>,
        mut f: impl FnMut(&Row) -> Result<T>,
    ) -> Result<Vec<T>> {
        self.fetch(sql, params)?.iter().map(|r| f(r)).collect()
    }

    fn fetch(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Row>> {
        self.rt.block_on(async {
            let mut rows = self.inner.query(sql, params).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let values = (0..row.column_count())
                    .map(|i| row.get_value(i))
                    .collect::<turso::Result<Vec<_>>>()?;
                out.push(Row { values });
            }
            Ok::<_, turso::Error>(out)
        })
        .with_context(|| failed(sql))
    }

    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.last_insert_rowid()
    }

    /// `PRAGMA <name>` as an integer.
    pub fn pragma_int(&self, name: &str) -> Result<i64> {
        self.query_row(&format!("PRAGMA {name}"), vec![], |r| r.get(0))
    }

    /// `PRAGMA <name> = <value>`. Not parameterisable, so `value` must never
    /// come from user input — every caller here passes a literal.
    pub fn pragma_set(&self, name: &str, value: impl std::fmt::Display) -> Result<()> {
        // Some pragmas (journal_mode) answer with a row; reading it and
        // discarding it works for both kinds.
        self.fetch(&format!("PRAGMA {name} = {value}"), vec![])?;
        Ok(())
    }

    /// A transaction that rolls back unless it is committed.
    pub fn transaction(&self) -> Result<Tx<'_>> {
        self.execute("BEGIN", vec![])?;
        Ok(Tx { conn: self, done: false })
    }
}

fn failed(sql: &str) -> String {
    format!("running `{}`", first_line(sql))
}

fn first_line(sql: &str) -> String {
    let line = sql.trim().lines().next().unwrap_or_default().trim();
    if line.len() > 68 {
        format!("{}…", &line[..67])
    } else {
        line.to_string()
    }
}

/// A committed-or-rolled-back scope. `Deref` gives it the whole `Conn` API, so
/// statements inside a transaction read exactly like statements outside one.
pub struct Tx<'a> {
    conn: &'a Conn,
    done: bool,
}

impl Tx<'_> {
    pub fn commit(mut self) -> Result<()> {
        self.conn.execute("COMMIT", vec![])?;
        self.done = true;
        Ok(())
    }
}

impl std::ops::Deref for Tx<'_> {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        self.conn
    }
}

impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if !self.done {
            // An import that failed half way must not leave a partial pack
            // behind. Nothing useful to do if the rollback itself fails.
            let _ = self.conn.execute("ROLLBACK", vec![]);
        }
    }
}

// ----------------------------------------------------------------- rows --

/// One row, read out in full.
pub struct Row {
    values: Vec<Value>,
}

impl Row {
    pub fn get<T: FromValue>(&self, idx: usize) -> Result<T> {
        let v = self
            .values
            .get(idx)
            .ok_or_else(|| anyhow!("no column {idx} in a row of {}", self.values.len()))?;
        T::from_value(v).with_context(|| format!("reading column {idx}"))
    }
}

pub trait FromValue: Sized {
    fn from_value(v: &Value) -> Result<Self>;
}

impl FromValue for i64 {
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Integer(i) => Ok(*i),
            // COUNT/SUM over an empty set can come back REAL.
            Value::Real(f) => Ok(*f as i64),
            other => Err(mismatch("an integer", other)),
        }
    }
}

impl FromValue for f64 {
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            other => Err(mismatch("a number", other)),
        }
    }
}

impl FromValue for bool {
    fn from_value(v: &Value) -> Result<Self> {
        i64::from_value(v).map(|i| i != 0)
    }
}

impl FromValue for i32 {
    fn from_value(v: &Value) -> Result<Self> {
        i64::from_value(v)?
            .try_into()
            .context("integer does not fit in an i32")
    }
}

impl FromValue for u8 {
    fn from_value(v: &Value) -> Result<Self> {
        Ok(i64::from_value(v)?.clamp(0, u8::MAX as i64) as u8)
    }
}

impl FromValue for String {
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            other => Err(mismatch("text", other)),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

fn mismatch(want: &str, got: &Value) -> anyhow::Error {
    let kind = match got {
        Value::Null => "NULL",
        Value::Integer(_) => "an integer",
        Value::Real(_) => "a real",
        Value::Text(_) => "text",
        Value::Blob(_) => "a blob",
    };
    anyhow!("expected {want}, found {kind}")
}

// --------------------------------------------------------------- params --

pub trait ToValue {
    fn to_value(&self) -> Value;
}

macro_rules! to_value_int {
    ($($t:ty),*) => {$(
        impl ToValue for $t {
            fn to_value(&self) -> Value { Value::Integer(*self as i64) }
        }
    )*};
}
to_value_int!(i64, i32, u8, u32, usize);

impl ToValue for f64 {
    fn to_value(&self) -> Value {
        Value::Real(*self)
    }
}

/// `Grade::score` is an `f32`; SQLite REAL is always 64-bit.
impl ToValue for f32 {
    fn to_value(&self) -> Value {
        Value::Real(*self as f64)
    }
}

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Integer(*self as i64)
    }
}

impl ToValue for str {
    fn to_value(&self) -> Value {
        Value::Text(self.to_string())
    }
}

impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}

impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null,
        }
    }
}

/// Bind values for a statement, in `?1`, `?2`, … order.
#[macro_export]
macro_rules! params {
    () => { ::std::vec::Vec::<$crate::sql::Value>::new() };
    ($($v:expr),+ $(,)?) => {
        ::std::vec![$($crate::sql::ToValue::to_value(&$v)),+]
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Conn {
        let c = Conn::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL)")
            .unwrap();
        c
    }

    #[test]
    fn roundtrips_every_value_kind() {
        let c = conn();
        c.execute(
            "INSERT INTO t (id, name, score) VALUES (?1, ?2, ?3)",
            params![1, "one", 0.5],
        )
        .unwrap();

        let (id, name, score): (i64, String, f64) = c
            .query_row("SELECT id, name, score FROM t", params![], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((id, name, score), (1, "one".to_string(), 0.5));
    }

    #[test]
    fn null_reads_as_none() {
        let c = conn();
        c.execute("INSERT INTO t (id, name) VALUES (1, NULL)", params![])
            .unwrap();
        let name: Option<String> = c
            .query_row("SELECT name FROM t", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(name, None);
    }

    #[test]
    fn a_missing_row_is_none_not_an_error() {
        let c = conn();
        let got: Option<i64> = c
            .query_row_opt("SELECT id FROM t WHERE id = 99", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(got, None);
        assert!(
            c.query_row("SELECT id FROM t WHERE id = 99", params![], |r| r
                .get::<i64>(0))
                .is_err()
        );
    }

    #[test]
    fn a_row_closure_may_fail_on_its_own_terms() {
        let c = conn();
        c.execute("INSERT INTO t (id, name) VALUES (1, 'x')", params![])
            .unwrap();
        let err = c
            .query_row("SELECT name FROM t", params![], |r| {
                let _: i64 = r.get(0)?;
                Ok(())
            })
            .unwrap_err()
            .to_string();
        assert!(err.contains("column 0"), "{err}");
    }

    #[test]
    fn an_uncommitted_transaction_rolls_back() {
        let c = conn();
        {
            let tx = c.transaction().unwrap();
            tx.execute("INSERT INTO t (id) VALUES (1)", params![])
                .unwrap();
        }
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM t", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "dropping a transaction must undo it");
    }

    #[test]
    fn a_committed_transaction_sticks() {
        let c = conn();
        let tx = c.transaction().unwrap();
        tx.execute("INSERT INTO t (id) VALUES (1)", params![])
            .unwrap();
        tx.commit().unwrap();
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM t", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
