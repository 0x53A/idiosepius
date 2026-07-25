//! Database handle, migrations, and content access.

use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::model::*;
use crate::params;
use crate::sql::{Conn, Row};

/// Bumped whenever `schema.sql` changes in a way that needs a migration step.
const SCHEMA_VERSION: i64 = 1;

pub struct Store {
    conn: Conn,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_conn(Conn::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Conn::open_in_memory()?)
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

        if version > SCHEMA_VERSION {
            bail!(
                "database was written by a newer version of idiosepius \
                 (schema v{version}, this build understands v{SCHEMA_VERSION})"
            );
        }

        self.conn.execute_batch(include_str!("schema.sql"))?;
        // Future migrations slot in here, keyed on `version`.
        self.conn.pragma_set("user_version", SCHEMA_VERSION)?;
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

        let payload = serde_json::to_string(&q.body)?;
        let tags = serde_json::to_string(&q.tags)?;
        let now = now_ms();

        self.conn.execute(
            "INSERT INTO question
                 (deck_id, topic_id, uid, kind, prompt, payload, explanation,
                  difficulty, source, tags, active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11)
             ON CONFLICT(uid) DO UPDATE SET
                 deck_id = excluded.deck_id,
                 topic_id = excluded.topic_id,
                 kind = excluded.kind,
                 prompt = excluded.prompt,
                 payload = excluded.payload,
                 explanation = excluded.explanation,
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
                q.prompt,
                payload,
                q.explanation,
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
            "SELECT id, deck_id, topic_id, uid, prompt, payload, explanation,
                    difficulty, source, tags
             FROM question WHERE id = ?1",
            params![id],
            question_from_row,
        )
    }

    pub fn questions(&self, deck_id: Id) -> Result<Vec<Question>> {
        self.conn.query_all(
            "SELECT id, deck_id, topic_id, uid, prompt, payload, explanation,
                    difficulty, source, tags
             FROM question WHERE deck_id = ?1 AND active = 1 ORDER BY id",
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
}

/// A question as authored, before it has an id.
#[derive(Debug, Clone)]
pub struct NewQuestion {
    pub deck_id: Id,
    pub topic_id: Option<Id>,
    pub uid: String,
    pub prompt: String,
    pub body: Body,
    pub explanation: Option<String>,
    pub difficulty: u8,
    pub source: Option<String>,
    pub tags: Vec<String>,
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

fn question_from_row(r: &Row) -> Result<Question> {
    let payload: String = r.get(5)?;
    let tags: String = r.get(9)?;
    let id: Id = r.get(0)?;

    Ok(Question {
        id,
        deck_id: r.get(1)?,
        topic_id: r.get(2)?,
        uid: r.get(3)?,
        prompt: r.get(4)?,
        body: serde_json::from_str(&payload)
            .with_context(|| format!("payload of question {id} is not valid"))?,
        explanation: r.get(6)?,
        difficulty: r.get(7)?,
        source: r.get(8)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            prompt: "Is this a test?".into(),
            body: Body::TrueFalse { answer: true },
            explanation: None,
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
    fn reimport_updates_in_place_and_keeps_the_id() {
        let (s, deck) = store_with_deck();
        let first = s.upsert_question(&q(deck, "u1")).unwrap();

        let mut edited = q(deck, "u1");
        edited.prompt = "Fixed typo?".into();
        let second = s.upsert_question(&edited).unwrap();

        assert_eq!(
            first, second,
            "uid must pin the row identity across imports"
        );
        assert_eq!(s.question_count(deck).unwrap(), 1);
        assert_eq!(s.question(first).unwrap().unwrap().prompt, "Fixed typo?");
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
    fn a_newer_schema_is_refused_rather_than_downgraded() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn().pragma_set("user_version", 99).unwrap();
        let err = s.migrate().unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
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
