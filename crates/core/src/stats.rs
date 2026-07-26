//! Read-only queries over the log, for the progress screen and for after the
//! fact evaluation.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::db::Store;
use crate::model::*;
use crate::params;

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeckStats {
    pub questions: i64,
    pub attempted: i64,
    pub attempts: i64,
    pub correct: i64,
    /// Share of attempts that were right, over the whole history.
    pub accuracy: f64,
    /// Share of the deck sitting in a box that means "this is sticking".
    pub readiness: f64,
    pub median_latency_ms: i64,
    pub time_studied_ms: i64,
}

/// Boxes at or above this are treated as learned for the readiness figure.
const SOLID_BOX: i32 = 3;

pub fn deck_stats(store: &Store, deck_id: Id) -> Result<DeckStats> {
    let c = store.conn();

    let questions: i64 = c.query_row(
        "SELECT COUNT(*) FROM question WHERE deck_id = ?1 AND active = 1",
        params![deck_id],
        |r| r.get(0),
    )?;

    let (attempts, correct, attempted): (i64, i64, i64) = c.query_row(
        "SELECT COUNT(*), COALESCE(SUM(a.correct), 0), COUNT(DISTINCT a.question_id)
         FROM attempt a JOIN question q ON q.id = a.question_id
         WHERE q.deck_id = ?1",
        params![deck_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;

    let solid: i64 = c.query_row(
        "SELECT COUNT(*) FROM review_state r JOIN question q ON q.id = r.question_id
         WHERE q.deck_id = ?1 AND q.active = 1 AND r.box >= ?2",
        params![deck_id, SOLID_BOX],
        |r| r.get(0),
    )?;

    // Median, not mean: a card left on screen while making coffee should not
    // drag the typical response time with it.
    let median_latency_ms: i64 = c
        .query_row(
            "SELECT a.latency_ms FROM attempt a JOIN question q ON q.id = a.question_id
             WHERE q.deck_id = ?1 AND a.latency_ms >= 0
             ORDER BY a.latency_ms
             LIMIT 1 OFFSET (
                 SELECT COUNT(*) / 2 FROM attempt a2 JOIN question q2 ON q2.id = a2.question_id
                 WHERE q2.deck_id = ?1 AND a2.latency_ms >= 0
             )",
            params![deck_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let time_studied_ms: i64 = c.query_row(
        "SELECT COALESCE(SUM(COALESCE(ended_at, started_at) - started_at), 0)
         FROM session WHERE deck_id = ?1",
        params![deck_id],
        |r| r.get(0),
    )?;

    Ok(DeckStats {
        questions,
        attempted,
        attempts,
        correct,
        accuracy: if attempts > 0 {
            correct as f64 / attempts as f64
        } else {
            0.0
        },
        readiness: if questions > 0 {
            solid as f64 / questions as f64
        } else {
            0.0
        },
        median_latency_ms,
        time_studied_ms,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TopicStat {
    pub topic_id: Option<Id>,
    pub title: String,
    pub questions: i64,
    pub attempts: i64,
    pub correct: i64,
    pub accuracy: f64,
    pub solid: i64,
}

pub fn topic_stats(store: &Store, deck_id: Id) -> Result<Vec<TopicStat>> {
    store.conn().query_all(
        "SELECT t.id,
                COALESCE(t.title, 'Uncategorised'),
                COUNT(DISTINCT q.id),
                COUNT(a.id),
                COALESCE(SUM(a.correct), 0),
                COUNT(DISTINCT CASE WHEN r.box >= ?2 THEN q.id END)
         FROM question q
         LEFT JOIN topic t        ON t.id = q.topic_id
         LEFT JOIN attempt a      ON a.question_id = q.id
         LEFT JOIN review_state r ON r.question_id = q.id
         WHERE q.deck_id = ?1 AND q.active = 1
         GROUP BY t.id
         ORDER BY COALESCE(t.ord, 999), t.title",
        params![deck_id, SOLID_BOX],
        |r| {
            let attempts: i64 = r.get(3)?;
            let correct: i64 = r.get(4)?;
            Ok(TopicStat {
                topic_id: r.get(0)?,
                title: r.get(1)?,
                questions: r.get(2)?,
                attempts,
                correct,
                accuracy: if attempts > 0 {
                    correct as f64 / attempts as f64
                } else {
                    0.0
                },
                solid: r.get(5)?,
            })
        },
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct WeakQuestion {
    pub question_id: Id,
    pub prompt: String,
    pub attempts: i64,
    pub correct: i64,
    pub lapses: i64,
    pub ema: f64,
}

/// The cards most worth another look, worst first.
pub fn weakest(store: &Store, deck_id: Id, limit: usize) -> Result<Vec<WeakQuestion>> {
    store.conn().query_all(
        "SELECT q.id, q.prompt, r.seen_count, r.correct_count, r.lapses, r.ema
         FROM question q JOIN review_state r ON r.question_id = q.id
         WHERE q.deck_id = ?1 AND q.active = 1 AND r.seen_count > 0
         ORDER BY r.ema ASC, r.lapses DESC
         LIMIT ?2",
        params![deck_id, limit as i64],
        |r| {
            let prompt: String = r.get(1)?;
            let prompt: Vec<ContentBlock> =
                serde_json::from_str(&prompt).context("weak-question prompt is not valid")?;
            Ok(WeakQuestion {
                question_id: r.get(0)?,
                prompt: content_text(&prompt),
                attempts: r.get(2)?,
                correct: r.get(3)?,
                lapses: r.get(4)?,
                ema: r.get(5)?,
            })
        },
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    pub session_id: Id,
    pub started_at: Millis,
    pub duration_ms: i64,
    pub answered: i64,
    pub correct: i64,
    pub skipped: i64,
    pub accuracy: f64,
}

pub fn session_stats(store: &Store, session_id: Id) -> Result<SessionStats> {
    let c = store.conn();
    let (started_at, ended_at): (Millis, Option<Millis>) = c.query_row(
        "SELECT started_at, ended_at FROM session WHERE id = ?1",
        params![session_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let (answered, correct): (i64, i64) = c.query_row(
        "SELECT COUNT(*), COALESCE(SUM(correct), 0) FROM attempt WHERE session_id = ?1",
        params![session_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let skipped: i64 = c.query_row(
        "SELECT COUNT(*) FROM event WHERE session_id = ?1 AND kind = 'skipped'",
        params![session_id],
        |r| r.get(0),
    )?;

    // An in-flight session has no end time yet; measure to now instead.
    let end = ended_at.unwrap_or_else(now_ms);

    Ok(SessionStats {
        session_id,
        started_at,
        duration_ms: end - started_at,
        answered,
        correct,
        skipped,
        accuracy: if answered > 0 {
            correct as f64 / answered as f64
        } else {
            0.0
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewQuestion;
    use crate::session::{Input, Mode, Session};

    fn setup() -> (std::rc::Rc<Store>, Id, Vec<Question>) {
        let store = std::rc::Rc::new(Store::open_in_memory().unwrap());
        let deck = store.upsert_deck("d", "D", None, None).unwrap();
        let topic = store.upsert_topic(deck, "t1", "Topic One", 1).unwrap();
        let qs = (0..3)
            .map(|i| {
                let id = store
                    .upsert_question(&NewQuestion {
                        deck_id: deck,
                        topic_id: Some(topic),
                        uid: format!("q{i}"),
                        prompt: vec![ContentBlock::text(format!("prompt {i}"))],
                        body: Body::TrueFalse { answer: true },
                        explanation: None,
                        explain: Default::default(),
                        difficulty: 2,
                        source: None,
                        tags: vec![],
                    })
                    .unwrap();
                store.question(id).unwrap().unwrap()
            })
            .collect();
        (store, deck, qs)
    }

    #[test]
    fn deck_stats_reflect_answers() {
        let (store, deck, qs) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        s.show(qs[0].id);
        s.answer(&qs[0], &Response::TrueFalse { value: true }, Input::Swipe)
            .unwrap();
        s.show(qs[1].id);
        s.answer(&qs[1], &Response::TrueFalse { value: false }, Input::Swipe)
            .unwrap();

        let st = deck_stats(&store, deck).unwrap();
        assert_eq!(st.questions, 3);
        assert_eq!(st.attempts, 2);
        assert_eq!(st.correct, 1);
        assert_eq!(st.attempted, 2);
        assert!((st.accuracy - 0.5).abs() < 1e-9);
    }

    #[test]
    fn stats_on_an_untouched_deck_do_not_divide_by_zero() {
        let (store, deck, _) = setup();
        let st = deck_stats(&store, deck).unwrap();
        assert_eq!(st.accuracy, 0.0);
        assert_eq!(st.readiness, 0.0);
        assert!(topic_stats(&store, deck).unwrap().len() == 1);
        assert!(weakest(&store, deck, 10).unwrap().is_empty());
    }

    #[test]
    fn weakest_puts_the_worst_card_first() {
        let (store, deck, qs) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        for _ in 0..3 {
            s.show(qs[0].id);
            s.answer(&qs[0], &Response::TrueFalse { value: true }, Input::Key)
                .unwrap();
            s.show(qs[1].id);
            s.answer(&qs[1], &Response::TrueFalse { value: false }, Input::Key)
                .unwrap();
        }
        let w = weakest(&store, deck, 5).unwrap();
        assert_eq!(w[0].question_id, qs[1].id);
        assert!(w[0].ema < w[1].ema);
    }

    #[test]
    fn session_stats_count_skips_separately() {
        let (store, deck, qs) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        let sid = s.id();
        s.show(qs[0].id);
        s.answer(&qs[0], &Response::TrueFalse { value: true }, Input::Swipe)
            .unwrap();
        s.show(qs[1].id);
        s.skip(qs[1].id);
        s.end().unwrap();

        let st = session_stats(&store, sid).unwrap();
        assert_eq!(st.answered, 1);
        assert_eq!(st.correct, 1);
        assert_eq!(st.skipped, 1);
        assert!(st.duration_ms >= 0);
    }

    #[test]
    fn topic_stats_group_by_topic() {
        let (store, deck, qs) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        s.show(qs[0].id);
        s.answer(&qs[0], &Response::TrueFalse { value: true }, Input::Swipe)
            .unwrap();

        let ts = topic_stats(&store, deck).unwrap();
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].title, "Topic One");
        assert_eq!(ts[0].questions, 3);
        assert_eq!(ts[0].attempts, 1);
    }
}
