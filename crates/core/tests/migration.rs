//! Schema upgrades preserve the state that cannot be reconstructed.

use idiosepius_core::sql::Conn;
use idiosepius_core::{ContentBlock, Store, params};

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
        let mut path = std::env::temp_dir();
        path.push(format!("idiosepius-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Self(path)
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
fn the_existing_v1_upgrade_still_reaches_v3() {
    let db = TempDb::new("v1");
    let conn = Conn::open(&db.0).unwrap();
    conn.execute_batch(V1_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO deck (slug, title, created_at)
         VALUES ('cs', 'Control Systems', 0)",
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
    drop(conn);

    let store = Store::open(&db.0).unwrap();
    let question = store.question_by_uid("cs-001").unwrap().unwrap();
    assert_eq!(
        question.prompt,
        vec![ContentBlock::text("A pole in the RHP is unstable.")]
    );
    assert_eq!(question.short().len(), 1);
    assert!(question.explain.is_empty());

    let seen: i64 = store
        .conn()
        .query_row(
            "SELECT seen_count FROM review_state WHERE question_id = 1",
            params![],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seen, 7);
    assert_eq!(store.fact_count().unwrap(), 0);
    assert_eq!(store.conn().pragma_int("user_version").unwrap(), 3);
}

#[test]
fn v2_is_migrated_without_losing_history_or_identity() {
    let db = TempDb::new("old-format");
    let conn = Conn::open(&db.0).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE deck (
            id INTEGER PRIMARY KEY, slug TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL, description TEXT, exam_at INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE question (
            id INTEGER PRIMARY KEY, deck_id INTEGER NOT NULL,
            topic_id INTEGER, uid TEXT NOT NULL UNIQUE, kind TEXT NOT NULL,
            prompt TEXT NOT NULL, payload TEXT NOT NULL, explanation TEXT,
            explain TEXT, difficulty INTEGER NOT NULL, source TEXT,
            tags TEXT NOT NULL, active INTEGER NOT NULL,
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE TABLE fact (
            id INTEGER PRIMARY KEY, deck_id INTEGER, uid TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL, label TEXT, name TEXT, title TEXT,
            body TEXT NOT NULL, source TEXT,
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE TABLE review_state (
            question_id INTEGER PRIMARY KEY, box INTEGER NOT NULL,
            due_at INTEGER NOT NULL, last_seen_at INTEGER,
            streak INTEGER NOT NULL, lapses INTEGER NOT NULL,
            seen_count INTEGER NOT NULL, correct_count INTEGER NOT NULL,
            ema REAL NOT NULL
        );
        CREATE TABLE session (
            id INTEGER PRIMARY KEY, deck_id INTEGER, mode TEXT NOT NULL,
            started_at INTEGER NOT NULL, ended_at INTEGER,
            app_version TEXT NOT NULL
        );
        CREATE TABLE event (
            id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
            ts INTEGER NOT NULL, mono_ms INTEGER NOT NULL,
            question_id INTEGER, kind TEXT NOT NULL, data TEXT
        );
        CREATE TABLE attempt (
            id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL,
            question_id INTEGER NOT NULL, ts INTEGER NOT NULL,
            latency_ms INTEGER NOT NULL, correct INTEGER NOT NULL,
            score REAL NOT NULL, response TEXT NOT NULL,
            input_method TEXT, box_before INTEGER
        );

        INSERT INTO deck VALUES (7, 'cs', 'Control Systems', NULL, NULL, 100);
        INSERT INTO question VALUES (
            41, 7, NULL, 'cs-041', 'true_false',
            'The loop is stable.', '{"kind":"true_false","answer":true}',
            NULL, NULL, 3, 'Lecture 4', '["stability"]', 1, 100, 200
        );
        INSERT INTO fact VALUES (
            9, 7, 'stable-loop', 'note', NULL, NULL, 'Stability',
            'All closed-loop poles lie in the left half-plane.',
            'Lecture 4', 100, 200
        );
        INSERT INTO review_state VALUES (41, 4, 123456, 120000, 3, 2, 8, 6, 0.75);
        INSERT INTO session VALUES (3, 7, 'practice', 110000, 130000, '0.1.0');
        INSERT INTO event VALUES (12, 3, 120000, 10000, 41, 'answer', '{}');
        INSERT INTO attempt VALUES (
            5, 3, 41, 120000, 850, 1, 1.0,
            '{"kind":"true_false","value":true}', 'swipe', 3
        );
        "#,
    )
    .unwrap();
    conn.pragma_set("user_version", 2).unwrap();
    drop(conn);

    let store = Store::open(&db.0).unwrap();
    assert_eq!(store.conn().pragma_int("user_version").unwrap(), 3);

    let question = store.question_by_uid("cs-041").unwrap().unwrap();
    assert_eq!(question.id, 41);
    assert_eq!(
        question.prompt,
        vec![ContentBlock::text("The loop is stable.")]
    );
    let fact = store.fact("stable-loop").unwrap().unwrap();
    assert_eq!(
        fact.body,
        vec![ContentBlock::text(
            "All closed-loop poles lie in the left half-plane."
        )]
    );

    let state: (i64, i64, i64, i64) = store
        .conn()
        .query_row(
            "SELECT box, lapses, seen_count, correct_count
             FROM review_state WHERE question_id = 41",
            vec![],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(state, (4, 2, 8, 6));
    for table in ["event", "attempt"] {
        let count: i64 = store
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), vec![], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1, "{table} history must survive migration");
    }
}

#[test]
fn reopening_a_current_database_is_a_no_op() {
    let db = TempDb::new("current-reopen");
    let deck = {
        let store = Store::open(&db.0).unwrap();
        store
            .upsert_deck("cs", "Control Systems", None, None)
            .unwrap()
    };
    let store = Store::open(&db.0).unwrap();
    assert_eq!(store.deck_id("cs").unwrap(), Some(deck));
    assert_eq!(store.conn().pragma_int("user_version").unwrap(), 3);
}
