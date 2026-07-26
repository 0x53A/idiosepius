//! Database handle, migrations, and content access.

use anyhow::{Context, Result, bail};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::model::*;
use crate::params;
use crate::sql::{Conn, Row};

/// Database format understood by this build.
const SCHEMA_VERSION: i64 = 3;

pub struct Store {
    conn: Conn,
}

impl Store {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_conn(Conn::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Conn::open_in_memory()?)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open_browser(database: Vec<u8>, wal: Vec<u8>) -> Result<Self> {
        Self::from_conn(Conn::open_browser(database, wal)?)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn browser_snapshot(&self) -> crate::browser_io::BrowserSnapshot {
        self.conn.browser_snapshot()
    }

    /// Fold the WAL into the ordinary SQLite file and return a coherent
    /// snapshot. Browser persistence uses the same checkpointed form as an
    /// explicit export, so restoring never depends on two files being replaced
    /// atomically by OPFS.
    #[cfg(target_arch = "wasm32")]
    pub fn browser_checkpoint_snapshot(&self) -> Result<crate::browser_io::BrowserSnapshot> {
        self.conn
            .query_all("PRAGMA wal_checkpoint(TRUNCATE)", vec![], |_| Ok(()))?;
        Ok(self.conn.browser_snapshot())
    }

    /// The whole study database as one ordinary SQLite file.
    ///
    /// The WAL is folded in first, so what comes out is self-contained: the
    /// course, the history and the scheduler state in a single file that
    /// `sqlite3` — or another copy of this app — opens directly.
    pub fn export_database(&self) -> Result<Vec<u8>> {
        #[cfg(target_arch = "wasm32")]
        {
            Ok(self.browser_checkpoint_snapshot()?.database)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.conn
                .query_all("PRAGMA wal_checkpoint(TRUNCATE)", vec![], |_| Ok(()))?;
            let path = self
                .conn
                .path()
                .ok_or_else(|| anyhow::anyhow!("this database is in memory; there is no file"))?;
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))
        }
    }

    fn from_conn(conn: Conn) -> Result<Self> {
        // WAL keeps the UI thread from blocking behind the logger, and lets a
        // separate process read the log live while a session is running.
        conn.pragma_set("journal_mode", "WAL")?;
        // NORMAL is the right trade here: a crash can lose the last few events
        // but never corrupts the file, and we write on every card.
        conn.pragma_set("synchronous", "NORMAL")?;
        conn.pragma_set("foreign_keys", "ON")?;

        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version = self.conn.pragma_int("user_version")?;

        match version {
            0 => {
                self.conn.execute_batch(include_str!("schema.sql"))?;
                self.conn.pragma_set("user_version", SCHEMA_VERSION)?;
            }
            1 => {
                // v1 predates structured explanations and the fact table.
                // Bring its table set to v2 first, then apply the content-block
                // rewrite below. Keeping this hop costs very little and avoids
                // regressing the migration the previous release already had.
                self.conn.execute_batch(include_str!("schema.sql"))?;
                self.add_column_if_missing("question", "explain", "TEXT")?;
                self.migrate_v2_to_v3()?;
            }
            2 => self.migrate_v2_to_v3()?,
            SCHEMA_VERSION => {}
            _ => {
                bail!(
                    "database schema v{version} is not supported by this build \
                     (expected v{SCHEMA_VERSION})"
                );
            }
        }
        Ok(())
    }

    fn add_column_if_missing(&self, table: &str, column: &str, decl: &str) -> Result<()> {
        if self
            .conn
            .query_row_opt(
                &format!("SELECT {column} FROM {table} LIMIT 1"),
                params![],
                |_| Ok(()),
            )
            .is_ok()
        {
            return Ok(());
        }
        self.conn
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
                params![],
            )
            .with_context(|| format!("adding column {table}.{column}"))?;
        Ok(())
    }

    /// v3 turns question prompts and fact bodies from raw prose into ordered
    /// `ContentBlock` arrays. No identity, history or scheduler row changes.
    fn migrate_v2_to_v3(&self) -> Result<()> {
        let tx = self.conn.transaction()?;

        let questions: Vec<(Id, String)> =
            tx.query_all("SELECT id, prompt FROM question", params![], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        for (id, text) in questions {
            let blocks = serde_json::to_string(&vec![ContentBlock::text(text)])?;
            tx.execute(
                "UPDATE question SET prompt = ?2 WHERE id = ?1",
                params![id, blocks],
            )
            .with_context(|| format!("migrating prompt of question {id}"))?;
        }

        let facts: Vec<(Id, String)> =
            tx.query_all("SELECT id, body FROM fact", params![], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        for (id, text) in facts {
            let blocks = serde_json::to_string(&vec![ContentBlock::text(text)])?;
            tx.execute(
                "UPDATE fact SET body = ?2 WHERE id = ?1",
                params![id, blocks],
            )
            .with_context(|| format!("migrating body of fact {id}"))?;
        }

        tx.pragma_set("user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(())
    }

    pub fn conn(&self) -> &Conn {
        &self.conn
    }

    // ------------------------------------------------------------- decks --

    pub fn upsert_deck(
        &self,
        slug: &str,
        title: &str,
        description: Option<&str>,
        exam_at: Option<Millis>,
    ) -> Result<Id> {
        self.conn.execute(
            "INSERT INTO deck (slug, title, description, exam_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(slug) DO UPDATE SET
                 title = excluded.title,
                 description = excluded.description,
                 exam_at = excluded.exam_at",
            params![slug, title, description, exam_at, now_ms()],
        )?;
        self.deck_id(slug)?
            .with_context(|| format!("deck {slug} vanished after upsert"))
    }

    pub fn deck_id(&self, slug: &str) -> Result<Option<Id>> {
        self.conn
            .query_row_opt("SELECT id FROM deck WHERE slug = ?1", params![slug], |r| {
                r.get(0)
            })
    }

    pub fn decks(&self) -> Result<Vec<Deck>> {
        self.conn.query_all(
            "SELECT id, slug, title, description, exam_at FROM deck ORDER BY title",
            params![],
            deck_from_row,
        )
    }

    pub fn deck(&self, id: Id) -> Result<Option<Deck>> {
        self.conn.query_row_opt(
            "SELECT id, slug, title, description, exam_at FROM deck WHERE id = ?1",
            params![id],
            deck_from_row,
        )
    }

    // ------------------------------------------------------------ topics --

    pub fn upsert_topic(&self, deck_id: Id, slug: &str, title: &str, ord: i64) -> Result<Id> {
        self.conn.execute(
            "INSERT INTO topic (deck_id, slug, title, ord) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(deck_id, slug) DO UPDATE SET
                 title = excluded.title, ord = excluded.ord",
            params![deck_id, slug, title, ord],
        )?;
        self.conn.query_row(
            "SELECT id FROM topic WHERE deck_id = ?1 AND slug = ?2",
            params![deck_id, slug],
            |r| r.get(0),
        )
    }

    pub fn topics(&self, deck_id: Id) -> Result<Vec<Topic>> {
        self.conn.query_all(
            "SELECT id, deck_id, slug, title, ord FROM topic
             WHERE deck_id = ?1 ORDER BY ord, title",
            params![deck_id],
            |r| {
                Ok(Topic {
                    id: r.get(0)?,
                    deck_id: r.get(1)?,
                    slug: r.get(2)?,
                    title: r.get(3)?,
                    ord: r.get(4)?,
                })
            },
        )
    }

    // --------------------------------------------------------- questions --

    /// Insert or update by `uid`. Returns the question id.
    ///
    /// Re-importing an edited pack keeps ids stable, so a fixed typo does not
    /// throw away the attempt history or the scheduler state for that card.
    pub fn upsert_question(&self, q: &NewQuestion) -> Result<Id> {
        q.body
            .validate()
            .map_err(|e| anyhow::anyhow!("question {}: {e}", q.uid))?;
        for block in &q.prompt {
            if let ContentBlock::Figure { figure } = block {
                figure
                    .validate()
                    .map_err(|e| anyhow::anyhow!("question {}: {e}", q.uid))?;
            }
        }

        let prompt = serde_json::to_string(&q.prompt)?;
        let payload = serde_json::to_string(&q.body)?;
        let tags = serde_json::to_string(&q.tags)?;
        let explain = if q.explain.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&q.explain)?)
        };
        let now = now_ms();

        self.conn.execute(
            "INSERT INTO question
                 (deck_id, topic_id, uid, kind, prompt, payload, explanation,
                  explain, difficulty, source, tags, active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)
             ON CONFLICT(uid) DO UPDATE SET
                 deck_id = excluded.deck_id,
                 topic_id = excluded.topic_id,
                 kind = excluded.kind,
                 prompt = excluded.prompt,
                 payload = excluded.payload,
                 explanation = excluded.explanation,
                 explain = excluded.explain,
                 difficulty = excluded.difficulty,
                 source = excluded.source,
                 tags = excluded.tags,
                 active = 1,
                 updated_at = excluded.updated_at",
            params![
                q.deck_id,
                q.topic_id,
                q.uid,
                q.body.kind().as_str(),
                prompt,
                payload,
                q.explanation,
                explain,
                q.difficulty,
                q.source,
                tags,
                now,
            ],
        )?;

        let id: Id = self.conn.query_row(
            "SELECT id FROM question WHERE uid = ?1",
            params![&q.uid],
            |r| r.get(0),
        )?;

        // Give every question a scheduler row immediately, due now, so "never
        // seen" and "due" are the same query.
        self.conn.execute(
            "INSERT INTO review_state (question_id, due_at) VALUES (?1, ?2)
             ON CONFLICT(question_id) DO NOTHING",
            params![id, now],
        )?;

        Ok(id)
    }

    pub fn question(&self, id: Id) -> Result<Option<Question>> {
        self.conn.query_row_opt(
            &format!("SELECT {QUESTION_COLUMNS} FROM question WHERE id = ?1"),
            params![id],
            question_from_row,
        )
    }

    pub fn question_by_uid(&self, uid: &str) -> Result<Option<Question>> {
        self.conn.query_row_opt(
            &format!("SELECT {QUESTION_COLUMNS} FROM question WHERE uid = ?1"),
            params![uid],
            question_from_row,
        )
    }

    pub fn questions(&self, deck_id: Id) -> Result<Vec<Question>> {
        self.conn.query_all(
            &format!(
                "SELECT {QUESTION_COLUMNS} FROM question
                 WHERE deck_id = ?1 AND active = 1 ORDER BY id"
            ),
            params![deck_id],
            question_from_row,
        )
    }

    pub fn question_count(&self, deck_id: Id) -> Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM question WHERE deck_id = ?1 AND active = 1",
            params![deck_id],
            |r| r.get(0),
        )
    }

    /// Mark every question of a deck inactive. Used before a full re-import so
    /// that questions dropped from the pack stop appearing, without deleting
    /// the history attached to them.
    pub fn deactivate_deck_questions(&self, deck_id: Id) -> Result<usize> {
        self.conn.execute(
            "UPDATE question SET active = 0 WHERE deck_id = ?1",
            params![deck_id],
        )
    }

    // ------------------------------------------------------------- facts --

    /// Insert or update by `uid`, exactly like a question: a fact is content,
    /// and editing the wording of one must not orphan the questions that cite
    /// it.
    pub fn upsert_fact(&self, f: &NewFact) -> Result<Id> {
        for block in &f.body {
            if let ContentBlock::Figure { figure } = block {
                figure
                    .validate()
                    .map_err(|e| anyhow::anyhow!("fact {}: {e}", f.uid))?;
            }
        }
        let body = serde_json::to_string(&f.body)?;
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO fact
                 (deck_id, uid, kind, label, name, title, body, source,
                  created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(uid) DO UPDATE SET
                 deck_id = excluded.deck_id,
                 kind = excluded.kind,
                 label = excluded.label,
                 name = excluded.name,
                 title = excluded.title,
                 body = excluded.body,
                 source = excluded.source,
                 updated_at = excluded.updated_at",
            params![
                f.deck_id,
                f.uid,
                f.kind.as_str(),
                f.label,
                f.name,
                f.title,
                body,
                f.source,
                now,
            ],
        )?;
        self.conn
            .query_row("SELECT id FROM fact WHERE uid = ?1", params![&f.uid], |r| {
                r.get(0)
            })
    }

    /// Every fact attached to a deck, plus the deck-independent ones.
    ///
    /// The UI holds the whole set for the session: it is a few hundred short
    /// rows, and an explanation may cite any of them.
    pub fn facts(&self, deck_id: Id) -> Result<Vec<Fact>> {
        self.conn.query_all(
            "SELECT id, deck_id, uid, kind, label, name, title, body, source
             FROM fact WHERE deck_id = ?1 OR deck_id IS NULL ORDER BY uid",
            params![deck_id],
            fact_from_row,
        )
    }

    pub fn fact(&self, uid: &str) -> Result<Option<Fact>> {
        self.conn.query_row_opt(
            "SELECT id, deck_id, uid, kind, label, name, title, body, source
             FROM fact WHERE uid = ?1",
            params![uid],
            fact_from_row,
        )
    }

    pub fn fact_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM fact", params![], |r| r.get(0))
    }
}

/// A question as authored, before it has an id.
#[derive(Debug, Clone)]
pub struct NewQuestion {
    pub deck_id: Id,
    pub topic_id: Option<Id>,
    pub uid: String,
    pub prompt: Vec<ContentBlock>,
    pub body: Body,
    pub explanation: Option<String>,
    pub explain: Explain,
    pub difficulty: u8,
    pub source: Option<String>,
    pub tags: Vec<String>,
}

/// A fact as authored, before it has an id.
#[derive(Debug, Clone)]
pub struct NewFact {
    pub deck_id: Option<Id>,
    pub uid: String,
    pub kind: FactKind,
    pub label: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub body: Vec<ContentBlock>,
    pub source: Option<String>,
}

fn deck_from_row(r: &Row) -> Result<Deck> {
    Ok(Deck {
        id: r.get(0)?,
        slug: r.get(1)?,
        title: r.get(2)?,
        description: r.get(3)?,
        exam_at: r.get(4)?,
    })
}

/// Every column `question_from_row` expects, in order. One list, so the three
/// queries that read a question cannot drift apart.
const QUESTION_COLUMNS: &str = "id, deck_id, topic_id, uid, prompt, payload, \
                                explanation, explain, difficulty, source, tags";

fn question_from_row(r: &Row) -> Result<Question> {
    let id: Id = r.get(0)?;
    let prompt: String = r.get(4)?;
    let payload: String = r.get(5)?;
    let prompt: Vec<ContentBlock> = serde_json::from_str(&prompt)
        .with_context(|| format!("prompt of question {id} is not valid"))?;
    let body: Body = serde_json::from_str(&payload)
        .with_context(|| format!("payload of question {id} is not valid"))?;
    let explain: Option<String> = r.get(7)?;
    let tags: String = r.get(10)?;

    Ok(Question {
        id,
        deck_id: r.get(1)?,
        topic_id: r.get(2)?,
        uid: r.get(3)?,
        prompt,
        body,
        explanation: r.get(6)?,
        // A malformed explanation must not make a card unanswerable: the
        // question is still perfectly usable without its notes.
        explain: explain
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        difficulty: r.get(8)?,
        source: r.get(9)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
    })
}

fn fact_from_row(r: &Row) -> Result<Fact> {
    let uid: String = r.get(2)?;
    let kind: String = r.get(3)?;
    let body: String = r.get(7)?;
    let body: Vec<ContentBlock> = serde_json::from_str(&body)
        .with_context(|| format!("body of fact {uid:?} is not valid"))?;
    Ok(Fact {
        id: r.get(0)?,
        deck_id: r.get(1)?,
        uid,
        kind: FactKind::parse(&kind),
        label: r.get(4)?,
        name: r.get(5)?,
        title: r.get(6)?,
        body,
        source: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Figure;

    fn store_with_deck() -> (Store, Id) {
        let s = Store::open_in_memory().unwrap();
        let d = s.upsert_deck("test", "Test", None, None).unwrap();
        (s, d)
    }

    fn q(deck_id: Id, uid: &str) -> NewQuestion {
        NewQuestion {
            deck_id,
            topic_id: None,
            uid: uid.into(),
            prompt: vec![ContentBlock::text("Is this a test?")],
            body: Body::TrueFalse { answer: true },
            explanation: None,
            explain: Default::default(),
            difficulty: 1,
            source: None,
            tags: vec!["tag".into()],
        }
    }

    #[test]
    fn roundtrips_a_question() {
        let (s, deck) = store_with_deck();
        let id = s.upsert_question(&q(deck, "u1")).unwrap();
        let got = s.question(id).unwrap().unwrap();
        assert_eq!(got.difficulty, 1);
        assert_eq!(got.uid, "u1");
        assert_eq!(got.body, Body::TrueFalse { answer: true });
        assert_eq!(got.tags, vec!["tag".to_string()]);
    }

    #[test]
    fn question_and_fact_content_blocks_round_trip_in_order() {
        let (s, deck) = store_with_deck();
        let bode = Figure::Bode {
            num: vec![1.0],
            den: vec![1.0, 10.0, 0.0],
            phase: true,
        };
        let step = Figure::Step {
            num: vec![4.0],
            den: vec![1.0, 0.4, 4.0],
            t: [0.0, 20.0],
        };
        let mut authored = q(deck, "with-figure");
        authored.prompt = vec![
            ContentBlock::text("Read the frequency response."),
            ContentBlock::figure(bode.clone()),
            ContentBlock::text("Now compare its transient response."),
            ContentBlock::figure(step.clone()),
        ];
        let id = s.upsert_question(&authored).unwrap();
        assert_eq!(s.question(id).unwrap().unwrap().prompt, authored.prompt);

        let mut authored_fact = fact("step-shape", "A lightly damped response.");
        authored_fact.deck_id = Some(deck);
        authored_fact.body = vec![
            ContentBlock::text("Frequency domain:"),
            ContentBlock::figure(bode),
            ContentBlock::text("Time domain:"),
            ContentBlock::figure(step),
        ];
        s.upsert_fact(&authored_fact).unwrap();
        assert_eq!(
            s.fact("step-shape").unwrap().unwrap().body,
            authored_fact.body
        );
    }

    /// An export has to be a file another copy of the app can open, which
    /// means the WAL must be folded in rather than left beside it.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn exporting_yields_a_self_contained_sqlite_file() {
        let dir = std::env::temp_dir().join(format!("idiosepius-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("study.db");

        let store = Store::open(&path).unwrap();
        let deck = store.upsert_deck("test", "Test", None, None).unwrap();
        store.upsert_question(&q(deck, "u1")).unwrap();
        let bytes = store.export_database().unwrap();
        drop(store);

        assert!(bytes.starts_with(b"SQLite format 3\0"));

        let copy = dir.join("copy.db");
        std::fs::write(&copy, bytes).unwrap();
        let reopened = Store::open(&copy).unwrap();
        assert_eq!(
            reopened.question_by_uid("u1").unwrap().map(|q| q.uid),
            Some("u1".to_string()),
            "the export must carry writes that were still only in the WAL"
        );
        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_in_memory_database_has_no_file_to_export() {
        let (store, _) = store_with_deck();
        assert!(store.export_database().is_err());
    }

    #[test]
    fn reimport_updates_in_place_and_keeps_the_id() {
        let (s, deck) = store_with_deck();
        let first = s.upsert_question(&q(deck, "u1")).unwrap();

        let mut edited = q(deck, "u1");
        edited.prompt = vec![ContentBlock::text("Fixed typo?")];
        let second = s.upsert_question(&edited).unwrap();

        assert_eq!(
            first, second,
            "uid must pin the row identity across imports"
        );
        assert_eq!(s.question_count(deck).unwrap(), 1);
        assert_eq!(
            s.question(first).unwrap().unwrap().prompt_text(),
            "Fixed typo?"
        );
    }

    #[test]
    fn every_question_starts_due() {
        let (s, deck) = store_with_deck();
        let id = s.upsert_question(&q(deck, "u1")).unwrap();
        let due: Millis = s
            .conn()
            .query_row(
                "SELECT due_at FROM review_state WHERE question_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(due <= now_ms());
    }

    #[test]
    fn an_explanation_survives_the_round_trip() {
        let (s, deck) = store_with_deck();
        let mut authored = q(deck, "u1");
        authored.explain = Explain {
            short: vec![Seg::text("Poles in the left half plane.")],
            deep: vec![Seg::text("Because "), Seg::fact("sym-zeta"), Seg::text(".")],
        };
        let id = s.upsert_question(&authored).unwrap();

        let got = s.question(id).unwrap().unwrap();
        assert_eq!(got.explain, authored.explain);
        assert_eq!(got.explain.referenced_facts(), vec!["sym-zeta"]);
    }

    #[test]
    fn the_short_reading_falls_back_to_the_plain_explanation() {
        let (s, deck) = store_with_deck();
        let mut authored = q(deck, "u1");
        authored.explanation = Some("The old way of writing one.".into());
        let id = s.upsert_question(&authored).unwrap();

        let got = s.question(id).unwrap().unwrap();
        assert_eq!(
            got.short(),
            vec![Seg::text("The old way of writing one.")],
            "content written before Explain existed must still explain itself"
        );
        assert!(got.deep().is_empty());
    }

    fn fact(uid: &str, body: &str) -> NewFact {
        NewFact {
            deck_id: None,
            uid: uid.into(),
            kind: FactKind::Symbol,
            label: Some("ζ".into()),
            name: Some("zeta".into()),
            title: None,
            body: vec![ContentBlock::text(body)],
            source: None,
        }
    }

    #[test]
    fn facts_upsert_by_uid_like_questions() {
        let (s, _) = store_with_deck();
        let first = s.upsert_fact(&fact("sym-zeta", "damping ratio")).unwrap();
        let second = s
            .upsert_fact(&fact("sym-zeta", "the damping ratio, dimensionless"))
            .unwrap();

        assert_eq!(first, second, "editing a fact must not fork it");
        assert_eq!(s.fact_count().unwrap(), 1);
        assert_eq!(
            content_text(&s.fact("sym-zeta").unwrap().unwrap().body),
            "the damping ratio, dimensionless"
        );
    }

    #[test]
    fn deck_facts_include_the_deck_independent_ones() {
        let (s, deck) = store_with_deck();
        s.upsert_fact(&fact("sym-zeta", "damping ratio")).unwrap();
        let mut owned = fact("sym-omega", "natural frequency");
        owned.deck_id = Some(deck);
        s.upsert_fact(&owned).unwrap();

        let uids: Vec<String> = s.facts(deck).unwrap().into_iter().map(|f| f.uid).collect();
        assert_eq!(uids, vec!["sym-omega", "sym-zeta"]);
    }

    #[test]
    fn a_symbol_is_recognised_by_glyph_or_by_latex() {
        let (s, _) = store_with_deck();
        s.upsert_fact(&fact("sym-zeta", "damping ratio")).unwrap();
        let f = s.fact("sym-zeta").unwrap().unwrap();

        assert!(f.appears_in("the damping ratio ζ is 0.5"));
        assert!(f.appears_in(r"$2\zeta\omega_0 s$"));
        assert!(!f.appears_in("no such symbol here"));
    }

    #[test]
    fn a_latin_symbol_is_not_found_inside_a_word() {
        let (s, _) = store_with_deck();
        let mut f = fact("sym-ess", "steady-state error");
        f.label = Some("e_ss".into());
        f.name = None;
        s.upsert_fact(&f).unwrap();
        let f = s.fact("sym-ess").unwrap().unwrap();

        assert!(f.appears_in("then e_ss = 0"));
        assert!(f.appears_in("e_ss"));
        assert!(
            !f.appears_in("it is necessary but not sufficient"),
            "a glossary entry for a symbol the card never used is noise"
        );
        assert!(!f.appears_in("the value e_ssx"));
    }

    #[test]
    fn a_different_schema_version_is_refused() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn().pragma_set("user_version", 99).unwrap();
        let err = s.migrate().unwrap_err().to_string();
        assert!(err.contains("not supported"), "{err}");
    }

    #[test]
    fn broken_content_is_rejected_at_import() {
        let (s, deck) = store_with_deck();
        let mut bad = q(deck, "u1");
        bad.body = Body::MultipleChoice {
            options: vec![Choice::new("a", false), Choice::new("b", false)],
            multi: false,
        };
        let err = s.upsert_question(&bad).unwrap_err().to_string();
        assert!(err.contains("no option is marked correct"), "{err}");
    }

    #[test]
    fn deactivating_hides_questions_without_losing_them() {
        let (s, deck) = store_with_deck();
        s.upsert_question(&q(deck, "u1")).unwrap();
        s.deactivate_deck_questions(deck).unwrap();
        assert_eq!(s.question_count(deck).unwrap(), 0);
        assert!(s.questions(deck).unwrap().is_empty());

        // Re-importing brings it back, same row.
        s.upsert_question(&q(deck, "u1")).unwrap();
        assert_eq!(s.question_count(deck).unwrap(), 1);
    }
}
