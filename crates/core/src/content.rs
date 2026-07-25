//! Question packs: the authoring format that gets imported into the database.
//!
//! JSON rather than YAML on purpose — a lot of these questions contain things
//! like `0x1F`, `1:2`, `no`, and LaTeX with colons and braces, all of which
//! YAML is happy to silently reinterpret.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::db::{NewQuestion, Store};
use crate::params;
use crate::model::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub deck: PackDeck,
    #[serde(default)]
    pub topics: Vec<PackTopic>,
    pub questions: Vec<PackQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackDeck {
    pub slug: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// RFC 3339, e.g. "2026-07-27T08:00:00+02:00".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exam_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackTopic {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackQuestion {
    pub uid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub prompt: String,
    /// `kind` plus its kind-specific fields, flattened into this object.
    #[serde(flatten)]
    pub body: Body,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    #[serde(default = "default_difficulty")]
    pub difficulty: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn default_difficulty() -> u8 {
    2
}

/// Combine several packs that describe the same deck into one.
///
/// A 150-question deck is unpleasant to edit as a single file, so the pack for
/// a course is authored as one file per topic. They must agree on the deck
/// slug; the first pack supplies the deck metadata.
pub fn merge_packs(packs: Vec<Pack>) -> Result<Pack> {
    let mut iter = packs.into_iter();
    let Some(mut merged) = iter.next() else {
        bail!("no packs to merge");
    };

    for pack in iter {
        if pack.deck.slug != merged.deck.slug {
            bail!(
                "packs describe different decks: {:?} and {:?}",
                merged.deck.slug,
                pack.deck.slug
            );
        }
        // Later files may fill in deck metadata the first one omitted, but
        // never silently override it.
        if merged.deck.description.is_none() {
            merged.deck.description = pack.deck.description;
        }
        if merged.deck.exam_at.is_none() {
            merged.deck.exam_at = pack.deck.exam_at;
        }

        for t in pack.topics {
            if !merged.topics.iter().any(|e| e.slug == t.slug) {
                merged.topics.push(t);
            }
        }
        merged.questions.extend(pack.questions);
    }

    Ok(merged)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportReport {
    pub deck_id: Id,
    pub topics: usize,
    pub questions: usize,
    pub retired: usize,
}

pub fn load_pack(path: impl AsRef<Path>) -> Result<Pack> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading pack {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing pack {}", path.display()))
}

/// Import a pack into the store.
///
/// Idempotent: questions are matched by `uid`, so re-running after an edit
/// updates the text in place and leaves attempt history and scheduling alone.
/// Questions no longer present in the pack are retired (`active = 0`) rather
/// than deleted, which keeps their history readable.
pub fn import_pack(store: &mut Store, pack: &Pack) -> Result<ImportReport> {
    // Validate everything before touching the database, so a typo in question
    // 90 does not leave a half-imported deck behind.
    let mut seen_uids = HashMap::new();
    for q in &pack.questions {
        if seen_uids.insert(&q.uid, ()).is_some() {
            bail!("duplicate uid {:?} in pack", q.uid);
        }
        q.body
            .validate()
            .map_err(|e| anyhow::anyhow!("question {}: {e}", q.uid))?;
        if let Some(t) = &q.topic
            && !pack.topics.iter().any(|pt| &pt.slug == t)
        {
            bail!("question {} references unknown topic {:?}", q.uid, t);
        }
    }

    let exam_at = match &pack.deck.exam_at {
        Some(s) => Some(parse_rfc3339_ms(s)?),
        None => None,
    };

    let tx = store.conn().transaction()?;
    tx.execute(
        "INSERT INTO deck (slug, title, description, exam_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(slug) DO UPDATE SET
             title = excluded.title,
             description = excluded.description,
             exam_at = excluded.exam_at",
        params![
            &pack.deck.slug,
            &pack.deck.title,
            &pack.deck.description,
            exam_at,
            now_ms()
        ],
    )?;
    let deck_id: Id = tx.query_row(
        "SELECT id FROM deck WHERE slug = ?1",
        params![&pack.deck.slug],
        |r| r.get(0),
    )?;
    tx.commit()?;

    let mut topic_ids = HashMap::new();
    for (i, t) in pack.topics.iter().enumerate() {
        let ord = if t.ord != 0 { t.ord } else { i as i64 + 1 };
        topic_ids.insert(
            t.slug.clone(),
            store.upsert_topic(deck_id, &t.slug, &t.title, ord)?,
        );
    }

    let retired = store.deactivate_deck_questions(deck_id)?;

    for q in &pack.questions {
        store.upsert_question(&NewQuestion {
            deck_id,
            topic_id: q.topic.as_ref().and_then(|t| topic_ids.get(t).copied()),
            uid: q.uid.clone(),
            prompt: q.prompt.clone(),
            body: q.body.clone(),
            explanation: q.explanation.clone(),
            difficulty: q.difficulty.clamp(1, 5),
            source: q.source.clone(),
            tags: q.tags.clone(),
        })?;
    }

    Ok(ImportReport {
        deck_id,
        topics: pack.topics.len(),
        questions: pack.questions.len(),
        retired: retired.saturating_sub(pack.questions.len()),
    })
}

fn parse_rfc3339_ms(s: &str) -> Result<Millis> {
    let dt = OffsetDateTime::parse(s, &Rfc3339)
        .with_context(|| format!("exam_at {s:?} is not a valid RFC 3339 timestamp"))?;
    Ok((dt.unix_timestamp_nanos() / 1_000_000) as Millis)
}

pub fn format_rfc3339_ms(ms: Millis) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_json() -> &'static str {
        r#"{
          "deck": {
            "slug": "cs",
            "title": "Control Systems",
            "exam_at": "2026-07-27T08:00:00Z"
          },
          "topics": [{ "slug": "stability", "title": "Stability", "ord": 3 }],
          "questions": [
            {
              "uid": "cs-001",
              "topic": "stability",
              "prompt": "A BIBO-stable LTI system has all poles in the left half plane.",
              "kind": "true_false",
              "answer": true,
              "explanation": "Strictly: Re(p) < 0 for every pole."
            },
            {
              "uid": "cs-002",
              "prompt": "The Routh-Hurwitz criterion needs...",
              "kind": "multiple_choice",
              "options": [
                { "text": "the characteristic polynomial", "correct": true },
                { "text": "a Bode plot", "correct": false }
              ]
            }
          ]
        }"#
    }

    #[test]
    fn parses_the_flattened_authoring_format() {
        let p: Pack = serde_json::from_str(pack_json()).unwrap();
        assert_eq!(p.questions.len(), 2);
        assert_eq!(p.questions[0].body, Body::TrueFalse { answer: true });
        assert_eq!(p.questions[0].difficulty, 2, "difficulty defaults");
        assert!(matches!(p.questions[1].body, Body::MultipleChoice { .. }));
    }

    #[test]
    fn imports_and_reimports_without_duplicating() {
        let mut store = Store::open_in_memory().unwrap();
        let pack: Pack = serde_json::from_str(pack_json()).unwrap();

        let r1 = import_pack(&mut store, &pack).unwrap();
        assert_eq!(r1.questions, 2);
        assert_eq!(store.question_count(r1.deck_id).unwrap(), 2);

        let r2 = import_pack(&mut store, &pack).unwrap();
        assert_eq!(store.question_count(r2.deck_id).unwrap(), 2);
        assert_eq!(r1.deck_id, r2.deck_id);
    }

    #[test]
    fn dropping_a_question_from_the_pack_retires_it() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(pack_json()).unwrap();
        let deck = import_pack(&mut store, &pack).unwrap().deck_id;
        assert_eq!(store.question_count(deck).unwrap(), 2);

        pack.questions.truncate(1);
        import_pack(&mut store, &pack).unwrap();
        assert_eq!(store.question_count(deck).unwrap(), 1);

        // Retired, not deleted.
        let total: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM question", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 2);
    }

    #[test]
    fn exam_date_reaches_the_deck() {
        let mut store = Store::open_in_memory().unwrap();
        let pack: Pack = serde_json::from_str(pack_json()).unwrap();
        let deck = import_pack(&mut store, &pack).unwrap().deck_id;
        let exam = store.deck(deck).unwrap().unwrap().exam_at.unwrap();
        assert_eq!(format_rfc3339_ms(exam), "2026-07-27T08:00:00Z");
    }

    #[test]
    fn a_bad_question_aborts_before_anything_is_written() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(pack_json()).unwrap();
        pack.questions.push(PackQuestion {
            uid: "cs-003".into(),
            topic: None,
            prompt: "broken".into(),
            body: Body::MultipleChoice {
                options: vec![Choice::new("a", false), Choice::new("b", false)],
                multi: false,
            },
            explanation: None,
            difficulty: 2,
            source: None,
            tags: vec![],
        });

        assert!(import_pack(&mut store, &pack).is_err());
        let total: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM question", params![], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 0, "nothing may be written when validation fails");
    }

    #[test]
    fn duplicate_uids_are_rejected() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(pack_json()).unwrap();
        let dup = pack.questions[0].clone();
        pack.questions.push(dup);
        let err = import_pack(&mut store, &pack).unwrap_err().to_string();
        assert!(err.contains("duplicate uid"), "{err}");
    }

    #[test]
    fn unknown_topic_reference_is_rejected() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(pack_json()).unwrap();
        pack.questions[1].topic = Some("nope".into());
        let err = import_pack(&mut store, &pack).unwrap_err().to_string();
        assert!(err.contains("unknown topic"), "{err}");
    }

    #[test]
    fn merges_topic_packs_into_one_deck() {
        let a: Pack = serde_json::from_str(pack_json()).unwrap();
        let b = Pack {
            deck: PackDeck {
                slug: "cs".into(),
                title: "Control Systems".into(),
                description: None,
                exam_at: None,
            },
            topics: vec![PackTopic {
                slug: "control".into(),
                title: "Control".into(),
                ord: 5,
            }],
            questions: vec![PackQuestion {
                uid: "cs-100".into(),
                topic: Some("control".into()),
                prompt: "A PI controller removes steady-state error.".into(),
                body: Body::TrueFalse { answer: true },
                explanation: None,
                difficulty: 2,
                source: None,
                tags: vec![],
            }],
        };

        let merged = merge_packs(vec![a, b]).unwrap();
        assert_eq!(merged.topics.len(), 2);
        assert_eq!(merged.questions.len(), 3);
        // Metadata from the first pack survives.
        assert_eq!(merged.deck.exam_at.as_deref(), Some("2026-07-27T08:00:00Z"));
    }

    #[test]
    fn merging_different_decks_is_rejected() {
        let a: Pack = serde_json::from_str(pack_json()).unwrap();
        let mut b: Pack = serde_json::from_str(pack_json()).unwrap();
        b.deck.slug = "other".into();
        assert!(merge_packs(vec![a, b]).is_err());
    }

    #[test]
    fn round_trips_through_serialisation() {
        let p: Pack = serde_json::from_str(pack_json()).unwrap();
        let s = serde_json::to_string(&p).unwrap();
        let p2: Pack = serde_json::from_str(&s).unwrap();
        assert_eq!(p.questions[0].body, p2.questions[0].body);
        assert_eq!(p.questions[1].uid, p2.questions[1].uid);
    }
}
