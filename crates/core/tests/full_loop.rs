//! End-to-end: import a pack, study a whole session through the public API,
//! and check the log, scheduler, and stats all agree afterwards.
//!
//! This is the path the UI drives. If it holds, the UI's job is only to turn
//! gestures into these same calls.

use std::rc::Rc;

use idiosepius_core::content::{self, Pack};
use idiosepius_core::model::Response;
use idiosepius_core::session::{Input, Mode, Session};
use idiosepius_core::{Store, params, scheduler, stats};

const PACK: &str = r#"{
  "deck": {
    "slug": "cs",
    "title": "Control Systems",
    "exam_at": "2026-07-27T09:00:00+02:00"
  },
  "topics": [
    { "slug": "stability", "title": "Stability", "ord": 1 },
    { "slug": "accuracy", "title": "Accuracy", "ord": 2 }
  ],
  "questions": [
    { "uid": "q-tf-1", "topic": "stability",
      "prompt": "A BIBO-stable system has all poles in the LHP.",
      "kind": "true_false", "answer": true },
    { "uid": "q-tf-2", "topic": "stability",
      "prompt": "An integrator is BIBO-stable.",
      "kind": "true_false", "answer": false },
    { "uid": "q-mc-1", "topic": "accuracy",
      "prompt": "Zero steady-state error to a step needs...",
      "kind": "multiple_choice",
      "options": [
        { "text": "at least one integrator", "correct": true },
        { "text": "no integrator", "correct": false }
      ] },
    { "uid": "q-mc-2", "topic": "accuracy",
      "prompt": "Which help transient shaping?",
      "kind": "multiple_choice", "multi": true,
      "options": [
        { "text": "D-term", "correct": true },
        { "text": "P-term", "correct": true },
        { "text": "constant disturbance", "correct": false }
      ] }
  ]
}"#;

fn imported_store() -> (Rc<Store>, i64) {
    let mut store = Store::open_in_memory().unwrap();
    let pack: Pack = serde_json::from_str(PACK).unwrap();
    let deck = content::import_pack(&mut store, &pack).unwrap().deck_id;
    (Rc::new(store), deck)
}

#[test]
fn a_practice_pass_answers_the_fresh_cards_then_spaces_them_out() {
    let (store, deck) = imported_store();

    // Drive purely off the scheduler like the UI does. In a single sitting a
    // correct card is pushed minutes into the future, so once every card has
    // been seen once, nothing is due and the pass ends. Learning the deck
    // takes repeated exposure across that spacing — see the Cram test below.
    let mut session = Session::start(store.clone(), deck, Mode::Practice).unwrap();
    let mut recent = Vec::new();
    let mut answered = 0;

    for _ in 0..500 {
        let counts = scheduler::counts(&store, deck).unwrap();
        if counts.fresh == 0 && counts.due == 0 {
            break;
        }
        let Some(q) = scheduler::next_card(&store, deck, Mode::Practice, &recent, None).unwrap()
        else {
            break;
        };
        session.show(q.id);

        let response = correct_response(&q);
        let outcome = session.answer(&q, &response, Input::Swipe).unwrap();
        assert!(
            outcome.grade.correct,
            "correct answer graded wrong: {}",
            q.uid
        );

        recent.push(q.id);
        if recent.len() > 8 {
            recent.remove(0);
        }
        answered += 1;
    }
    session.end().unwrap();

    // Exactly the four fresh cards, no runaway loop replaying spaced ones.
    assert_eq!(answered, 4, "a first pass should see each fresh card once");

    let st = stats::deck_stats(&store, deck).unwrap();
    assert_eq!(st.questions, 4);
    assert_eq!(st.attempted, 4, "every card should have been seen");
    assert_eq!(st.correct, st.attempts, "all answers were correct");

    // The attempt table and the event log must tell the same story.
    let attempts: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM attempt", params![], |r| r.get(0))
        .unwrap();
    let committed: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM event WHERE kind = 'answer_committed'",
            params![],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, committed);
    assert_eq!(attempts, answered);
}

#[test]
fn cram_keeps_cards_in_rotation_until_the_deck_is_learned() {
    let (store, deck) = imported_store();

    // Cram ignores spacing, so it can carry a card from box 0 to "learned"
    // (box >= 3) inside one sitting. Answering everything right must reach a
    // fully-learned deck in a bounded number of reviews.
    let mut session = Session::start(store.clone(), deck, Mode::Cram).unwrap();
    let mut recent = Vec::new();
    let mut answered = 0;

    for _ in 0..500 {
        if stats::deck_stats(&store, deck).unwrap().readiness >= 1.0 {
            break;
        }
        let q = scheduler::next_card(&store, deck, Mode::Cram, &recent, None)
            .unwrap()
            .expect("cram always offers a card while any remain unlearned");
        session.show(q.id);
        session
            .answer(&q, &correct_response(&q), Input::Swipe)
            .unwrap();
        recent.push(q.id);
        if recent.len() > 8 {
            recent.remove(0);
        }
        answered += 1;
    }
    session.end().unwrap();

    let st = stats::deck_stats(&store, deck).unwrap();
    assert!(
        (st.readiness - 1.0).abs() < 1e-9,
        "cram should learn the whole deck"
    );
    // 4 cards, each needs at least 3 correct reviews to reach box 3.
    assert!(answered >= 12, "unexpectedly few reviews: {answered}");
    assert!(answered < 60, "cram took too long to converge: {answered}");

    // The log and the attempt table still agree after a cram run.
    let attempts: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM attempt", params![], |r| r.get(0))
        .unwrap();
    let committed: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM event WHERE kind = 'answer_committed'",
            params![],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, committed);
    assert_eq!(attempts, answered);
}

#[test]
fn wrong_answers_surface_as_the_weakest_cards() {
    let (store, deck) = imported_store();
    let mut session = Session::start(store.clone(), deck, Mode::Practice).unwrap();

    // Deliberately fail q-tf-2 a few times, pass the others once.
    let by_uid = |uid: &str| {
        store
            .questions(deck)
            .unwrap()
            .into_iter()
            .find(|q| q.uid == uid)
            .unwrap()
    };

    let wrong = by_uid("q-tf-2");
    for _ in 0..3 {
        session.show(wrong.id);
        // q-tf-2's answer is false; saying true is wrong.
        session
            .answer(&wrong, &Response::TrueFalse { value: true }, Input::Key)
            .unwrap();
    }
    for uid in ["q-tf-1", "q-mc-1"] {
        let q = by_uid(uid);
        session.show(q.id);
        session
            .answer(&q, &correct_response(&q), Input::Key)
            .unwrap();
    }
    session.end().unwrap();

    let weak = stats::weakest(&store, deck, 5).unwrap();
    assert_eq!(
        weak[0].question_id, wrong.id,
        "the failed card must rank first"
    );
    assert!(weak[0].lapses >= 3);
    assert!(weak[0].ema < 0.5);
}

#[test]
fn nothing_is_scheduled_past_the_exam() {
    let (store, deck) = imported_store();
    let exam = store.deck(deck).unwrap().unwrap().exam_at.unwrap();

    let mut session = Session::start(store.clone(), deck, Mode::Practice).unwrap();
    let q = store.questions(deck).unwrap().into_iter().next().unwrap();

    // Graduate it as far as it will go.
    for _ in 0..10 {
        session.show(q.id);
        session
            .answer(&q, &correct_response(&q), Input::Key)
            .unwrap();
    }

    let due: i64 = store
        .conn()
        .query_row(
            "SELECT due_at FROM review_state WHERE question_id = ?1",
            params![q.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(due < exam, "a card is due after the exam has started");
}

fn correct_response(q: &idiosepius_core::Question) -> Response {
    use idiosepius_core::Body::*;
    match &q.body {
        TrueFalse { answer } => Response::TrueFalse { value: *answer },
        MultipleChoice { options, .. } => Response::MultipleChoice {
            selected: options
                .iter()
                .enumerate()
                .filter(|(_, c)| c.correct)
                .map(|(i, _)| i)
                .collect(),
        },
    }
}
