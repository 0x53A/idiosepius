//! Session and event logging.
//!
//! Everything the user does goes into `event`, append-only. Answers
//! additionally produce an `attempt` row and move the scheduler forward.
//!
//! The logger never fails a study session: if a write fails, the error is
//! reported through [`Session::take_errors`] and the session keeps running.
//! Losing a log line is annoying; losing your place the night before an exam
//! is not acceptable.

use crate::params;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::rc::Rc;
use web_time::Instant;

use crate::db::Store;
use crate::model::*;
use crate::scheduler;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Normal spaced study.
    Practice,
    /// Fixed set of cards, graded at the end, no feedback in between.
    Exam,
    /// Ignore spacing, hammer the weakest cards.
    Cram,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Practice => "practice",
            Mode::Exam => "exam",
            Mode::Cram => "cram",
        }
    }
}

/// How the answer was given. Worth logging separately: swipe latency and
/// click latency are not comparable, and mixing them would poison any later
/// analysis of hesitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Swipe,
    Click,
    Key,
    Touch,
}

impl Input {
    pub fn as_str(self) -> &'static str {
        match self {
            Input::Swipe => "swipe",
            Input::Click => "click",
            Input::Key => "key",
            Input::Touch => "touch",
        }
    }
}

/// Well-known event kinds. `Other` exists so the UI can log something new
/// without a core change; analysis queries filter on the string either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    SessionEnd,
    CardShown,
    /// A swipe/drag began. Paired with `SwipeCommit` or `SwipeCancel`.
    SwipeStart,
    SwipeCancel,
    AnswerCommitted,
    Revealed,
    Skipped,
    Undo,
    DeckSwitched,
    Paused,
    Resumed,
    Other(&'static str),
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::SessionStart => "session_start",
            Event::SessionEnd => "session_end",
            Event::CardShown => "card_shown",
            Event::SwipeStart => "swipe_start",
            Event::SwipeCancel => "swipe_cancel",
            Event::AnswerCommitted => "answer_committed",
            Event::Revealed => "revealed",
            Event::Skipped => "skipped",
            Event::Undo => "undo",
            Event::DeckSwitched => "deck_switched",
            Event::Paused => "paused",
            Event::Resumed => "resumed",
            Event::Other(s) => s,
        }
    }
}

/// The outcome of answering one card, handed back to the UI so it can show
/// feedback without re-querying.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub grade: Grade,
    pub attempt_id: Id,
    pub latency_ms: i64,
    /// Scheduler box after the update, for a progress indicator.
    pub box_after: i32,
    pub due_at: Millis,
}

pub struct Session {
    /// Shared rather than borrowed so a session can be stored next to the
    /// store it reads from, which a UI needs and a borrow would forbid.
    store: Rc<Store>,
    id: Id,
    deck_id: Id,
    mode: Mode,
    started_at: Millis,
    /// Monotonic origin for `event.mono_ms`.
    origin: Instant,
    /// Card currently on screen and when it appeared, for answer latency.
    shown: Option<(Id, Instant)>,
    /// Non-fatal logging failures, drained by the UI for a status line.
    errors: Vec<String>,
}

impl Session {
    pub fn start(store: Rc<Store>, deck_id: Id, mode: Mode) -> Result<Self> {
        let started_at = now_ms();
        store.conn().execute(
            "INSERT INTO session (deck_id, mode, started_at, app_version)
             VALUES (?1, ?2, ?3, ?4)",
            params![deck_id, mode.as_str(), started_at, APP_VERSION],
        )?;
        let id = store.conn().last_insert_rowid();

        let mut s = Session {
            store,
            id,
            deck_id,
            mode,
            started_at,
            origin: Instant::now(),
            shown: None,
            errors: Vec::new(),
        };
        s.log(Event::SessionStart, None, json!({ "mode": mode.as_str() }));
        Ok(s)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn deck_id(&self) -> Id {
        self.deck_id
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn started_at(&self) -> Millis {
        self.started_at
    }

    pub fn elapsed_ms(&self) -> i64 {
        self.origin.elapsed().as_millis() as i64
    }

    /// Non-fatal logging errors accumulated so far, cleared by the call.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    /// Append to the event log. Deliberately infallible from the caller's
    /// point of view — see the module docs.
    pub fn log(&mut self, kind: Event, question_id: Option<Id>, data: serde_json::Value) {
        let data = if data.is_null() {
            None
        } else {
            Some(data.to_string())
        };
        let res = self.store.conn().execute(
            "INSERT INTO event (session_id, ts, mono_ms, question_id, kind, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.id,
                now_ms(),
                self.elapsed_ms(),
                question_id,
                kind.as_str(),
                data
            ],
        );
        if let Err(e) = res {
            self.errors.push(format!("event log write failed: {e}"));
        }
    }

    /// Call when a card becomes visible. Starts the latency clock.
    pub fn show(&mut self, question_id: Id) {
        self.shown = Some((question_id, Instant::now()));
        self.log(Event::CardShown, Some(question_id), serde_json::Value::Null);
    }

    /// Milliseconds the current card has been on screen, if any.
    pub fn shown_for_ms(&self, question_id: Id) -> Option<i64> {
        self.shown
            .filter(|(id, _)| *id == question_id)
            .map(|(_, at)| at.elapsed().as_millis() as i64)
    }

    /// Grade a response, write the attempt, and advance the scheduler.
    pub fn answer(
        &mut self,
        question: &Question,
        response: &Response,
        input: Input,
    ) -> Result<Outcome> {
        let grade = question.body.grade(response);
        // If the card was never announced via `show` (a UI bug, or a resumed
        // session), latency is unknown rather than zero.
        let latency_ms = self.shown_for_ms(question.id).unwrap_or(-1);
        let box_before = scheduler::current_box(&self.store, question.id).unwrap_or(0);

        let response_json = serde_json::to_string(response)?;
        self.store.conn().execute(
            "INSERT INTO attempt
                 (session_id, question_id, ts, latency_ms, correct, score,
                  response, input_method, box_before)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                self.id,
                question.id,
                now_ms(),
                latency_ms,
                grade.correct as i32,
                grade.score,
                response_json,
                input.as_str(),
                box_before,
            ],
        )?;
        let attempt_id = self.store.conn().last_insert_rowid();

        let deck = self.store.deck(self.deck_id)?;
        let next = scheduler::advance(
            &self.store,
            question.id,
            grade,
            deck.as_ref().and_then(|d| d.exam_at),
        )?;

        self.log(
            Event::AnswerCommitted,
            Some(question.id),
            json!({
                "correct": grade.correct,
                "score": grade.score,
                "latency_ms": latency_ms,
                "input": input.as_str(),
                "response": response,
                "box_before": box_before,
                "box_after": next.box_after,
            }),
        );

        self.shown = None;
        Ok(Outcome {
            grade,
            attempt_id,
            latency_ms,
            box_after: next.box_after,
            due_at: next.due_at,
        })
    }

    /// Push a card away without answering. Not graded, but it *is* recorded —
    /// a card you keep skipping is a card you do not know.
    pub fn skip(&mut self, question_id: Id) {
        let latency_ms = self.shown_for_ms(question_id).unwrap_or(-1);
        self.log(
            Event::Skipped,
            Some(question_id),
            json!({ "latency_ms": latency_ms }),
        );
        self.shown = None;
    }

    /// Undo the most recent attempt of this session: removes the attempt row
    /// and rolls the scheduler state back to what it was before it.
    ///
    /// The event log keeps both the original answer and the undo. The log is
    /// the record of what happened, and the answer did happen.
    pub fn undo_last(&mut self) -> Result<Option<Id>> {
        let row: Option<(Id, Id, i32)> = self.store.conn().query_row_opt(
            "SELECT id, question_id, COALESCE(box_before, 0) FROM attempt
             WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
            params![self.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        let Some((attempt_id, question_id, box_before)) = row else {
            return Ok(None);
        };

        self.store
            .conn()
            .execute("DELETE FROM attempt WHERE id = ?1", params![attempt_id])?;
        scheduler::rewind(&self.store, question_id, box_before)?;

        self.log(
            Event::Undo,
            Some(question_id),
            json!({ "attempt_id": attempt_id }),
        );
        Ok(Some(question_id))
    }

    /// Close the session. Takes `&mut self` rather than consuming, so a UI
    /// can end a session it only holds a borrow of.
    pub fn end(&mut self) -> Result<()> {
        self.log(Event::SessionEnd, None, serde_json::Value::Null);
        self.store.conn().execute(
            "UPDATE session SET ended_at = ?1 WHERE id = ?2",
            params![now_ms(), self.id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewQuestion;

    fn setup() -> (Rc<Store>, Id, Question) {
        let store = Rc::new(Store::open_in_memory().unwrap());
        let deck = store.upsert_deck("d", "D", None, None).unwrap();
        let id = store
            .upsert_question(&NewQuestion {
                deck_id: deck,
                topic_id: None,
                uid: "q1".into(),
                prompt: vec![ContentBlock::text("?")],
                body: Body::TrueFalse { answer: true },
                explanation: None,
                explain: Default::default(),
                difficulty: 1,
                source: None,
                tags: vec![],
            })
            .unwrap();
        let q = store.question(id).unwrap().unwrap();
        (store, deck, q)
    }

    #[test]
    fn answering_writes_attempt_and_events() {
        let (store, deck, q) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        s.show(q.id);
        let out = s
            .answer(&q, &Response::TrueFalse { value: true }, Input::Swipe)
            .unwrap();
        assert!(out.grade.correct);

        let attempts: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM attempt", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 1);

        let shown: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM event WHERE kind = 'card_shown'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(shown, 1);
    }

    #[test]
    fn latency_is_unknown_when_the_card_was_never_shown() {
        let (store, deck, q) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        // No show() call.
        let out = s
            .answer(&q, &Response::TrueFalse { value: true }, Input::Key)
            .unwrap();
        assert_eq!(out.latency_ms, -1, "must not report a fabricated 0 ms");
    }

    #[test]
    fn undo_removes_the_attempt_but_keeps_the_log() {
        let (store, deck, q) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        s.show(q.id);
        s.answer(&q, &Response::TrueFalse { value: false }, Input::Swipe)
            .unwrap();

        assert_eq!(s.undo_last().unwrap(), Some(q.id));

        let attempts: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM attempt", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 0);

        let committed: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM event WHERE kind = 'answer_committed'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(committed, 1, "the answer still happened; the log keeps it");

        assert_eq!(s.undo_last().unwrap(), None, "nothing left to undo");
    }

    #[test]
    fn skipping_is_recorded_but_not_graded() {
        let (store, deck, q) = setup();
        let mut s = Session::start(store.clone(), deck, Mode::Practice).unwrap();
        s.show(q.id);
        s.skip(q.id);

        let attempts: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM attempt", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 0);

        let skips: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM event WHERE kind = 'skipped'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(skips, 1);
    }
}
