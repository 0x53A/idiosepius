//! Content types: what a question *is*, and what answering one looks like.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Id = i64;
/// Milliseconds since the Unix epoch, UTC.
pub type Millis = i64;

pub fn now_ms() -> Millis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as Millis)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ decks --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deck {
    pub id: Id,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub exam_at: Option<Millis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub id: Id,
    pub deck_id: Id,
    pub slug: String,
    pub title: String,
    pub ord: i64,
}

// -------------------------------------------------------------- questions --

/// The kind-specific half of a question. Serialised into `question.payload`,
/// so adding a variant needs no migration — only a UI that can render it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    /// Swipe left / right. `answer` is what a "true" swipe should mean.
    TrueFalse { answer: bool },
    /// One or more correct options out of several.
    MultipleChoice {
        options: Vec<Choice>,
        /// More than one option may be correct, and the user must find all.
        #[serde(default)]
        multi: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Choice {
    pub text: String,
    pub correct: bool,
    /// Shown after answering, when this specific option is why they got it
    /// wrong. Optional; the question-level explanation is the fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Choice {
    pub fn new(text: impl Into<String>, correct: bool) -> Self {
        Self {
            text: text.into(),
            correct,
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

impl Body {
    /// Stored in `question.kind` so we can filter without parsing every payload.
    pub fn kind(&self) -> Kind {
        match self {
            Body::TrueFalse { .. } => Kind::TrueFalse,
            Body::MultipleChoice { .. } => Kind::MultipleChoice,
        }
    }

    /// Indices of the options that are correct. Empty for non-choice kinds.
    pub fn correct_indices(&self) -> Vec<usize> {
        match self {
            Body::MultipleChoice { options, .. } => options
                .iter()
                .enumerate()
                .filter(|(_, c)| c.correct)
                .map(|(i, _)| i)
                .collect(),
            Body::TrueFalse { .. } => Vec::new(),
        }
    }

    /// Reject content that would render as an unanswerable card. Worth doing
    /// at import time: a multiple-choice question with no correct option is a
    /// typo that would otherwise silently mark every answer wrong.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Body::TrueFalse { .. } => Ok(()),
            Body::MultipleChoice { options, multi } => {
                if options.len() < 2 {
                    return Err(format!("needs at least 2 options, has {}", options.len()));
                }
                let n_correct = options.iter().filter(|c| c.correct).count();
                if n_correct == 0 {
                    return Err("no option is marked correct".into());
                }
                if !multi && n_correct > 1 {
                    return Err(format!(
                        "single-answer question has {n_correct} correct options; set multi = true"
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TrueFalse,
    MultipleChoice,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::TrueFalse => "true_false",
            Kind::MultipleChoice => "multiple_choice",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "true_false" => Some(Kind::TrueFalse),
            "multiple_choice" => Some(Kind::MultipleChoice),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: Id,
    pub deck_id: Id,
    pub topic_id: Option<Id>,
    pub uid: String,
    /// Markdown-ish; `$...$` spans are LaTeX and rendered as such by the UI.
    pub prompt: String,
    pub body: Body,
    pub explanation: Option<String>,
    /// 1 (recall) .. 5 (multi-step derivation).
    pub difficulty: u8,
    /// Where this came from, e.g. "Lecture Slides - Stability Part 2, p. 14".
    pub source: Option<String>,
    pub tags: Vec<String>,
}

// --------------------------------------------------------------- answering --

/// What the user actually did with the card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    TrueFalse {
        value: bool,
    },
    MultipleChoice {
        selected: Vec<usize>,
    },
    /// Card was pushed away without an answer. Recorded, but never graded.
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grade {
    pub correct: bool,
    /// 0.0 ..= 1.0. Partial credit exists so a multi-select miss is not
    /// scored the same as picking every wrong option.
    pub score: f32,
}

impl Grade {
    pub const WRONG: Grade = Grade {
        correct: false,
        score: 0.0,
    };
    pub const RIGHT: Grade = Grade {
        correct: true,
        score: 1.0,
    };
}

impl Body {
    /// Grade a response. A response of the wrong shape for this body (which
    /// means a UI bug) grades as wrong rather than panicking mid-session.
    pub fn grade(&self, response: &Response) -> Grade {
        match (self, response) {
            (Body::TrueFalse { answer }, Response::TrueFalse { value }) => {
                if answer == value {
                    Grade::RIGHT
                } else {
                    Grade::WRONG
                }
            }

            (Body::MultipleChoice { options, .. }, Response::MultipleChoice { selected }) => {
                let n_correct = options.iter().filter(|c| c.correct).count();
                if n_correct == 0 {
                    return Grade::WRONG; // Malformed content; validate() catches it at import.
                }

                let mut hits = 0usize;
                let mut misses = 0usize;
                // Deduplicate: a UI that sends the same index twice must not
                // be able to score above 1.0.
                let mut seen = selected.clone();
                seen.sort_unstable();
                seen.dedup();

                for &i in &seen {
                    match options.get(i) {
                        Some(c) if c.correct => hits += 1,
                        Some(_) => misses += 1,
                        None => misses += 1, // Index out of range counts against them.
                    }
                }

                let correct = hits == n_correct && misses == 0;
                let score = ((hits as f32 - misses as f32) / n_correct as f32).clamp(0.0, 1.0);
                Grade { correct, score }
            }

            (_, Response::Skipped) => Grade::WRONG,
            _ => Grade::WRONG,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc(correct: &[bool], multi: bool) -> Body {
        Body::MultipleChoice {
            options: correct.iter().map(|&c| Choice::new("opt", c)).collect(),
            multi,
        }
    }

    #[test]
    fn true_false_grading() {
        let b = Body::TrueFalse { answer: true };
        assert!(b.grade(&Response::TrueFalse { value: true }).correct);
        assert!(!b.grade(&Response::TrueFalse { value: false }).correct);
        assert!(!b.grade(&Response::Skipped).correct);
    }

    #[test]
    fn single_choice_grading() {
        let b = mc(&[false, true, false], false);
        assert!(
            b.grade(&Response::MultipleChoice { selected: vec![1] })
                .correct
        );
        assert!(
            !b.grade(&Response::MultipleChoice { selected: vec![0] })
                .correct
        );
        // Selecting everything is not a way to be right.
        assert!(
            !b.grade(&Response::MultipleChoice {
                selected: vec![0, 1, 2]
            })
            .correct
        );
    }

    #[test]
    fn multi_choice_partial_credit() {
        let b = mc(&[true, true, false, false], true);
        let all = b.grade(&Response::MultipleChoice {
            selected: vec![0, 1],
        });
        assert!(all.correct && all.score == 1.0);

        let half = b.grade(&Response::MultipleChoice { selected: vec![0] });
        assert!(!half.correct && (half.score - 0.5).abs() < 1e-6);

        // One hit, one wrong option: nets out to zero, not half.
        let mixed = b.grade(&Response::MultipleChoice {
            selected: vec![0, 2],
        });
        assert!(!mixed.correct && mixed.score == 0.0);

        // Score is clamped, never negative.
        let bad = b.grade(&Response::MultipleChoice {
            selected: vec![2, 3],
        });
        assert_eq!(bad.score, 0.0);
    }

    #[test]
    fn duplicate_selection_cannot_inflate_score() {
        let b = mc(&[true, true, false], true);
        let g = b.grade(&Response::MultipleChoice {
            selected: vec![0, 0, 0],
        });
        assert!(!g.correct);
        assert!((g.score - 0.5).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_index_counts_against() {
        let b = mc(&[true, false], false);
        let g = b.grade(&Response::MultipleChoice { selected: vec![99] });
        assert!(!g.correct && g.score == 0.0);
    }

    #[test]
    fn validation_catches_broken_content() {
        assert!(mc(&[true, false], false).validate().is_ok());
        assert!(mc(&[false, false], false).validate().is_err());
        assert!(mc(&[true, true], false).validate().is_err());
        assert!(mc(&[true, true], true).validate().is_ok());
        assert!(mc(&[true], false).validate().is_err());
    }
}
