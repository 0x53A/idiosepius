//! Which card to show next, and when to show it again.
//!
//! A Leitner box system with sub-day intervals. Classic spaced repetition is
//! tuned for retention over months; this is tuned for an exam on Monday, so
//! the early boxes are minutes apart and no interval is ever allowed to park a
//! card past the exam date.

use anyhow::Result;
use rand::RngExt;

use crate::db::Store;
use crate::model::*;
use crate::params;
use crate::session::Mode;
use crate::sql::Row;

/// Seconds until a card in box *n* comes back.
const INTERVALS: [i64; 7] = [
    45,      // 45 s   — just got it wrong, comes back this session
    180,     // 3 min
    600,     // 10 min
    3_600,   // 1 h
    21_600,  // 6 h
    86_400,  // 1 day
    259_200, // 3 days
];

const MAX_BOX: i32 = INTERVALS.len() as i32 - 1;

/// A card may never be scheduled further out than this fraction of the time
/// remaining until the exam, so there is always room for another repetition
/// or two before it counts.
const EXAM_HORIZON: f64 = 0.4;

/// Weight of the newest attempt in the correctness moving average.
const EMA_ALPHA: f64 = 0.4;

/// Multiplicative noise applied to a card's score before ranking. Wide enough
/// that cards within about a fifth of each other trade places from draw to
/// draw — which covers the whole spread of `difficulty` and of a mildly stale
/// `ema` — and narrow enough that a genuinely weak card still outranks a fresh
/// one every time.
const JITTER_LOW: f64 = 0.8;
const JITTER_HIGH: f64 = 1.25;

#[derive(Debug, Clone, Copy)]
pub struct Next {
    pub box_after: i32,
    pub due_at: Millis,
}

pub fn current_box(store: &Store, question_id: Id) -> Result<i32> {
    Ok(store
        .conn()
        .query_row_opt(
            "SELECT box FROM review_state WHERE question_id = ?1",
            params![question_id],
            |r| r.get(0),
        )?
        .unwrap_or(0))
}

/// Apply the result of an attempt to a card's scheduler state.
pub fn advance(
    store: &Store,
    question_id: Id,
    grade: Grade,
    exam_at: Option<Millis>,
) -> Result<Next> {
    let now = now_ms();
    let prev = current_box(store, question_id)?;

    // Right answer moves one box up; a miss goes back to the bottom, where
    // the card returns inside the same session.
    let box_after = if grade.correct {
        (prev + 1).min(MAX_BOX)
    } else {
        0
    };

    let interval_s = interval_for(box_after, now, exam_at);
    let due_at = now + interval_s * 1000;

    store.conn().execute(
        "INSERT INTO review_state
             (question_id, box, due_at, last_seen_at, streak, lapses,
              seen_count, correct_count, ema)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
         ON CONFLICT(question_id) DO UPDATE SET
             box = ?2,
             due_at = ?3,
             last_seen_at = ?4,
             streak = CASE WHEN ?9 THEN review_state.streak + 1 ELSE 0 END,
             lapses = review_state.lapses + CASE WHEN ?9 THEN 0 ELSE 1 END,
             seen_count = review_state.seen_count + 1,
             correct_count = review_state.correct_count + ?7,
             ema = review_state.ema * (1.0 - ?10) + ?11 * ?10",
        params![
            question_id,
            box_after,
            due_at,
            now,
            grade.correct as i32,  // streak seed for a first-ever attempt
            !grade.correct as i32, // lapses seed
            grade.correct as i32,  // correct_count increment
            grade.score as f64,    // ema seed
            grade.correct,
            EMA_ALPHA,
            grade.score as f64,
        ],
    )?;

    Ok(Next { box_after, due_at })
}

/// Undo support: put a card back in a given box and make it due now.
///
/// The counters (`seen_count`, `lapses`, …) are deliberately *not* rolled
/// back — the user did see the card, and pretending otherwise would let a
/// misswipe be laundered out of the statistics.
pub fn rewind(store: &Store, question_id: Id, box_before: i32) -> Result<()> {
    store.conn().execute(
        "UPDATE review_state SET box = ?2, due_at = ?3 WHERE question_id = ?1",
        params![question_id, box_before, now_ms()],
    )?;
    Ok(())
}

fn interval_for(box_n: i32, now: Millis, exam_at: Option<Millis>) -> i64 {
    let base = INTERVALS[box_n.clamp(0, MAX_BOX) as usize];

    match exam_at {
        Some(exam) if exam > now => {
            let remaining_s = (exam - now) / 1000;
            let horizon = (remaining_s as f64 * EXAM_HORIZON) as i64;
            // Never below the bottom interval: a card must still come back
            // promptly even in the last minutes before the exam.
            base.min(horizon.max(INTERVALS[0]))
        }
        // Exam already passed, or none set: plain Leitner.
        _ => base,
    }
}

// ------------------------------------------------------------- selection --

#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    /// Never answered.
    pub fresh: i64,
    /// Answered before and due now.
    pub due: i64,
    /// Answered and not yet due.
    pub later: i64,
    pub total: i64,
}

pub fn counts(store: &Store, deck_id: Id) -> Result<Counts> {
    let now = now_ms();
    let row = store.conn().query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN r.seen_count = 0 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN r.seen_count > 0 AND r.due_at <= ?2 THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN r.seen_count > 0 AND r.due_at >  ?2 THEN 1 ELSE 0 END), 0),
             COUNT(*)
         FROM question q
         JOIN review_state r ON r.question_id = q.id
         WHERE q.deck_id = ?1 AND q.active = 1",
        params![deck_id, now],
        |r| {
            Ok(Counts {
                fresh: r.get(0)?,
                due: r.get(1)?,
                later: r.get(2)?,
                total: r.get(3)?,
            })
        },
    )?;
    Ok(row)
}

struct Candidate {
    id: Id,
    topic_id: Option<Id>,
    seen_count: i64,
    due_at: Millis,
    ema: f64,
    lapses: i64,
    difficulty: i64,
}

/// Pick the next card for a deck.
///
/// `recent` is the tail of question ids already shown, most recent last. It is
/// used to avoid showing the same card twice in a row and to interleave
/// topics, which is what makes the difference between recognising a card and
/// knowing the answer.
pub fn next_card(
    store: &Store,
    deck_id: Id,
    mode: Mode,
    recent: &[Id],
    topic_filter: Option<Id>,
) -> Result<Option<Question>> {
    let now = now_ms();

    let mut sql = String::from(
        "SELECT q.id, q.topic_id, r.seen_count, r.due_at, r.ema, r.lapses, q.difficulty
         FROM question q
         JOIN review_state r ON r.question_id = q.id
         WHERE q.deck_id = ?1 AND q.active = 1",
    );
    if topic_filter.is_some() {
        sql.push_str(" AND q.topic_id = ?2");
    }

    let binds = match topic_filter {
        Some(t) => params![deck_id, t],
        None => params![deck_id],
    };
    let candidates: Vec<Candidate> = store.conn().query_all(&sql, binds, |r: &Row| {
        Ok(Candidate {
            id: r.get(0)?,
            topic_id: r.get(1)?,
            seen_count: r.get(2)?,
            due_at: r.get(3)?,
            ema: r.get(4)?,
            lapses: r.get(5)?,
            difficulty: r.get(6)?,
        })
    })?;

    if candidates.is_empty() {
        return Ok(None);
    }

    let last_topic = recent
        .last()
        .and_then(|id| candidates.iter().find(|c| c.id == *id))
        .and_then(|c| c.topic_id);

    // How far back to suppress. A fixed four-card memory is a loop on a deck
    // of a hundred and thirty, so the window grows with the deck.
    let cooldown = recent.len().min((candidates.len() / 8).max(4));

    let mut rng = rand::rng();
    let mut scored: Vec<(f64, &Candidate)> = candidates
        .iter()
        .map(|c| (score(c, now, mode, recent, cooldown, last_topic), c))
        .filter(|(s, _)| *s > 0.0)
        // Jitter before ranking. Without it the sort is stable over exactly
        // equal scores, so a deck of otherwise identical fresh cards is served
        // in row-id order and the pool below never reaches past the first few.
        // It also lets neighbouring scores interleave, which is what keeps a
        // coarse term like `difficulty` a preference instead of a gate.
        .map(|(s, c)| (s * rng.random_range(JITTER_LOW..JITTER_HIGH), c))
        .collect();

    if scored.is_empty() {
        // Everything is either suppressed as too recent or not due yet. Rather
        // than end the session, fall back to the card that has been waiting
        // longest — practising early beats stopping.
        let fallback = candidates
            .iter()
            .filter(|c| !recent.last().is_some_and(|last| *last == c.id))
            .min_by_key(|c| c.due_at)
            .or_else(|| candidates.first());
        return match fallback {
            Some(c) => store.question(c.id),
            None => Ok(None),
        };
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Sample from the strongest few instead of always taking the maximum, so
    // the deck does not replay in a memorised order.
    let pool = scored.len().min(5);
    let total: f64 = scored[..pool].iter().map(|(s, _)| s).sum();
    let mut pick = rand::rng().random_range(0.0..total.max(f64::MIN_POSITIVE));
    let mut chosen = scored[0].1;
    for (s, c) in &scored[..pool] {
        if pick < *s {
            chosen = c;
            break;
        }
        pick -= s;
    }

    store.question(chosen.id)
}

fn score(
    c: &Candidate,
    now: Millis,
    mode: Mode,
    recent: &[Id],
    cooldown: usize,
    last_topic: Option<Id>,
) -> f64 {
    // Never show the same card twice in a row.
    if recent.last().is_some_and(|last| *last == c.id) {
        return 0.0;
    }
    // Soft cooldown over a recent window, so a small deck still cycles but
    // does not feel like a loop of three cards.
    if recent[recent.len().saturating_sub(cooldown)..].contains(&c.id) {
        return 0.0;
    }

    let overdue_s = (now - c.due_at) as f64 / 1000.0;
    let fresh = c.seen_count == 0;

    // Urgency: is it *time* for this card?
    let urgency = if fresh {
        600.0
    } else if overdue_s >= 0.0 {
        // Waiting longer is more urgent, logarithmically and capped: a card
        // left overdue for a week must not outrank everything forever.
        450.0 + (40.0 * (1.0 + overdue_s / 60.0).ln()).min(150.0)
    } else if matches!(mode, Mode::Cram) {
        // Cram ignores spacing; weak cards stay in rotation regardless.
        150.0
    } else {
        // Not due. Eligible only as a last resort, handled by the caller.
        return 0.0;
    };

    // Weakness: how badly is it *needed*? Multiplicative, so a shaky card
    // beats a solid one that happens to be more overdue.
    let weakness = if fresh {
        // Unseen is not the same as always wrong — an `ema` of 0 here would
        // conflate the two and make new cards crowd out real weak spots.
        1.6
    } else {
        1.0 + 1.5 * (1.0 - c.ema).clamp(0.0, 1.0) + 0.15 * (c.lapses as f64).min(6.0)
    };

    let mut s = urgency * (weakness + 0.05 * c.difficulty as f64);

    // Interleave: penalise staying in the same topic back to back.
    if c.topic_id.is_some() && c.topic_id == last_topic {
        s *= 0.65;
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewQuestion;

    const HOUR: Millis = 3_600_000;

    fn setup(n: usize) -> (Store, Id, Vec<Id>) {
        let store = Store::open_in_memory().unwrap();
        let deck = store.upsert_deck("d", "D", None, None).unwrap();
        let ids = (0..n)
            .map(|i| {
                store
                    .upsert_question(&NewQuestion {
                        deck_id: deck,
                        topic_id: None,
                        uid: format!("q{i}"),
                        prompt: vec![ContentBlock::text("?")],
                        body: Body::TrueFalse { answer: true },
                        explanation: None,
                        explain: Default::default(),
                        difficulty: 1,
                        source: None,
                        tags: vec![],
                    })
                    .unwrap()
            })
            .collect();
        (store, deck, ids)
    }

    #[test]
    fn correct_answers_climb_boxes_and_misses_reset() {
        let (store, _, ids) = setup(1);
        let q = ids[0];

        let mut b = 0;
        for _ in 0..3 {
            b = advance(&store, q, Grade::RIGHT, None).unwrap().box_after;
        }
        assert_eq!(b, 3);

        let after_miss = advance(&store, q, Grade::WRONG, None).unwrap();
        assert_eq!(after_miss.box_after, 0);
        // Back within the session, not tomorrow.
        assert!(after_miss.due_at - now_ms() <= INTERVALS[0] * 1000 + 500);
    }

    #[test]
    fn box_is_capped_at_the_top() {
        let (store, _, ids) = setup(1);
        let mut b = 0;
        for _ in 0..20 {
            b = advance(&store, ids[0], Grade::RIGHT, None)
                .unwrap()
                .box_after;
        }
        assert_eq!(b, MAX_BOX);
    }

    #[test]
    fn nothing_is_scheduled_past_the_exam() {
        let (store, _, ids) = setup(1);
        let exam = now_ms() + 2 * HOUR;

        // Climb to the top box, which would normally be three days out.
        let mut last = None;
        for _ in 0..10 {
            last = Some(advance(&store, ids[0], Grade::RIGHT, Some(exam)).unwrap());
        }
        let next = last.unwrap();
        assert_eq!(next.box_after, MAX_BOX);
        assert!(
            next.due_at < exam,
            "card due {} ms after the exam started",
            next.due_at - exam
        );
    }

    #[test]
    fn the_horizon_still_leaves_room_for_another_repetition() {
        let now = now_ms();
        let exam = now + 10 * HOUR;
        // Top box wants 3 days; the horizon should cut it to 40% of 10 h.
        let i = interval_for(MAX_BOX, now, Some(exam));
        assert_eq!(i, (10 * 3600) * 4 / 10);
    }

    #[test]
    fn a_minute_before_the_exam_cards_still_come_back() {
        let now = now_ms();
        let exam = now + 60_000;
        assert_eq!(interval_for(MAX_BOX, now, Some(exam)), INTERVALS[0]);
    }

    #[test]
    fn counts_track_progress() {
        let (store, deck, ids) = setup(3);
        let c = counts(&store, deck).unwrap();
        assert_eq!((c.fresh, c.due, c.later, c.total), (3, 0, 0, 3));

        advance(&store, ids[0], Grade::RIGHT, None).unwrap();
        let c = counts(&store, deck).unwrap();
        assert_eq!((c.fresh, c.later, c.total), (2, 1, 3));
    }

    #[test]
    fn never_repeats_the_same_card_back_to_back() {
        let (store, deck, ids) = setup(4);
        let mut recent = vec![ids[0]];
        for _ in 0..30 {
            let q = next_card(&store, deck, Mode::Practice, &recent, None)
                .unwrap()
                .expect("deck is not empty");
            assert_ne!(q.id, *recent.last().unwrap());
            recent.push(q.id);
        }
    }

    /// Draw `n` cards the way the app does, keeping the same recent tail.
    fn draw(store: &Store, deck: Id, n: usize) -> Vec<Id> {
        let mut recent: Vec<Id> = Vec::new();
        let mut drawn = Vec::with_capacity(n);
        for _ in 0..n {
            let q = next_card(store, deck, Mode::Practice, &recent, None)
                .unwrap()
                .expect("deck is not empty");
            drawn.push(q.id);
            recent.push(q.id);
            if recent.len() > 32 {
                recent.remove(0);
            }
        }
        drawn
    }

    #[test]
    fn a_large_fresh_deck_is_covered_rather_than_looped() {
        let (store, deck, _) = setup(100);
        let distinct: std::collections::HashSet<Id> = draw(&store, deck, 60).into_iter().collect();
        // Identical fresh cards score identically, and a stable sort over a
        // tie serves them in row-id order: this used to loop over the first
        // handful of rows and never reach the rest of the deck.
        assert!(
            distinct.len() > 40,
            "only {} distinct cards in 60 draws",
            distinct.len()
        );
    }

    #[test]
    fn a_hard_minority_does_not_monopolise_the_deck() {
        let (store, deck, ids) = setup(60);
        // Ten harder cards among fifty, the shape the real Control Systems
        // deck had when it served nothing but its ten difficulty-4 questions.
        for id in ids.iter().take(10) {
            store
                .conn()
                .execute(
                    "UPDATE question SET difficulty = 4 WHERE id = ?1",
                    params![*id],
                )
                .unwrap();
        }

        let hard: std::collections::HashSet<Id> = ids[..10].iter().copied().collect();
        let drawn = draw(&store, deck, 100);
        let easy = drawn.iter().filter(|id| !hard.contains(id)).count();
        assert!(
            easy > 50,
            "the fifty easier cards took only {easy} of 100 draws"
        );
    }

    #[test]
    fn a_single_card_deck_still_yields_that_card() {
        let (store, deck, ids) = setup(1);
        // Even though it is the most recent card, there is nothing else.
        let q = next_card(&store, deck, Mode::Practice, &[ids[0]], None)
            .unwrap()
            .expect("must not stall on a one-card deck");
        assert_eq!(q.id, ids[0]);
    }

    #[test]
    fn returns_none_for_an_empty_deck() {
        let store = Store::open_in_memory().unwrap();
        let deck = store.upsert_deck("empty", "Empty", None, None).unwrap();
        assert!(
            next_card(&store, deck, Mode::Practice, &[], None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn weak_cards_are_favoured_over_solid_ones() {
        let (store, deck, ids) = setup(2);
        // Card 0: answered right repeatedly. Card 1: repeatedly wrong.
        for _ in 0..4 {
            advance(&store, ids[0], Grade::RIGHT, None).unwrap();
            advance(&store, ids[1], Grade::WRONG, None).unwrap();
        }
        // Make both due.
        store
            .conn()
            .execute("UPDATE review_state SET due_at = 0", params![])
            .unwrap();

        let mut weak = 0;
        const DRAWS: usize = 200;
        for _ in 0..DRAWS {
            let q = next_card(&store, deck, Mode::Practice, &[], None)
                .unwrap()
                .unwrap();
            if q.id == ids[1] {
                weak += 1;
            }
        }
        // Clearly favoured, but the solid card must still come round.
        assert!(
            weak > 130,
            "the weak card should dominate, got {weak}/{DRAWS}"
        );
        assert!(weak < DRAWS, "the solid card must not be starved entirely");
    }

    #[test]
    fn undo_restores_the_previous_box() {
        let (store, _, ids) = setup(1);
        advance(&store, ids[0], Grade::RIGHT, None).unwrap();
        advance(&store, ids[0], Grade::RIGHT, None).unwrap();
        assert_eq!(current_box(&store, ids[0]).unwrap(), 2);

        rewind(&store, ids[0], 1).unwrap();
        assert_eq!(current_box(&store, ids[0]).unwrap(), 1);

        // Counters are intentionally left alone.
        let seen: i64 = store
            .conn()
            .query_row(
                "SELECT seen_count FROM review_state WHERE question_id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seen, 2);
    }
}
