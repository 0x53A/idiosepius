//! Opening a database written by an older build must not lose anything.
//!
//! The whole application state is one file: a study history the night before
//! an exam is not something a schema change is allowed to throw away. This
//! builds a v1 file by hand — the schema as it shipped, without
//! `question.explain` and without the `fact` table — and checks that opening
//! it upgrades in place with the content and the log intact.

use idiosepius_core::sql::Conn;
use idiosepius_core::{Store, params};

/// The parts of the v1 schema this test needs, exactly as they were.
const V1_SCHEMA: &str = "
CREATE TABLE deck (
    id INTEGER PRIMARY KEY, slug TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
    description TEXT, exam_at INTEGER, created_at INTEGER NOT NULL);
CREATE TABLE question (
    id INTEGER PRIMARY KEY, deck_id INTEGER NOT NULL, topic_id INTEGER,
    uid TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, prompt TEXT NOT NULL,
    payload TEXT NOT NULL, explanation TEXT, difficulty INTEGER NOT NULL DEFAULT 2,
    source TEXT, tags TEXT NOT NULL DEFAULT '[]', active INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE review_state (
    question_id INTEGER PRIMARY KEY, box INTEGER NOT NULL DEFAULT 0,
    due_at INTEGER NOT NULL, last_seen_at INTEGER, streak INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0, seen_count INTEGER NOT NULL DEFAULT 0,
    correct_count INTEGER NOT NULL DEFAULT 0, ema REAL NOT NULL DEFAULT 0.0);
";

struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!("idiosepius-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        TempDb(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

#[test]
fn a_v1_database_upgrades_in_place() {
    let db = TempDb::new("v1");

    {
        let conn = Conn::open(db.path()).unwrap();
        conn.execute_batch(V1_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO deck (slug, title, created_at) VALUES ('cs', 'Control Systems', 0)",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO question
                 (deck_id, uid, kind, prompt, payload, explanation, created_at, updated_at)
             VALUES (1, 'cs-001', 'true_false', 'A pole in the RHP is unstable.',
                     '{\"kind\":\"true_false\",\"answer\":true}',
                     'Re(p) > 0 grows without bound.', 0, 0)",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review_state (question_id, due_at, seen_count, box)
             VALUES (1, 0, 7, 3)",
            params![],
        )
        .unwrap();
        conn.pragma_set("user_version", 1).unwrap();
    }

    let store = Store::open(db.path()).unwrap();

    // Content survived, and the old plain explanation still reads.
    let q = store.question_by_uid("cs-001").unwrap().unwrap();
    assert_eq!(q.prompt, "A pole in the RHP is unstable.");
    assert_eq!(q.short().len(), 1);
    assert!(q.explain.is_empty(), "there was nothing structured to find");

    // Scheduler state survived: the seven times this card was answered are
    // not something a schema change may quietly reset.
    let seen: i64 = store
        .conn()
        .query_row(
            "SELECT seen_count FROM review_state WHERE question_id = 1",
            params![],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seen, 7);

    // And the new machinery is there.
    assert_eq!(store.fact_count().unwrap(), 0);
    assert_eq!(store.conn().pragma_int("user_version").unwrap(), 2);
}

#[test]
fn reopening_an_upgraded_database_is_a_no_op() {
    let db = TempDb::new("v2-reopen");

    let deck = {
        let store = Store::open(db.path()).unwrap();
        store
            .upsert_deck("cs", "Control Systems", None, None)
            .unwrap()
    };
    {
        let store = Store::open(db.path()).unwrap();
        assert_eq!(store.deck_id("cs").unwrap(), Some(deck));
        assert_eq!(store.conn().pragma_int("user_version").unwrap(), 2);
    }
}
