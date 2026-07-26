//! Question packs: the authoring format that gets imported into the database.
//!
//! JSON rather than YAML on purpose — a lot of these questions contain things
//! like `0x1F`, `1:2`, `no`, and LaTeX with colons and braces, all of which
//! YAML is happy to silently reinterpret.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::db::{NewFact, NewLesson, NewQuestion, Store};
use crate::model::*;
use crate::params;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub deck: PackDeck,
    #[serde(default)]
    pub topics: Vec<PackTopic>,
    /// Shared explanation fragments and symbol definitions. Usually authored
    /// in a pack of their own (`content/cs-00-facts.json`) and merged in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<PackFact>,
    #[serde(default)]
    pub questions: Vec<PackQuestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<PackLesson>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackFact {
    pub uid: String,
    #[serde(default)]
    pub kind: FactKind,
    /// The glyph, for a symbol: `"ζ"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// What it is called: `"zeta"`. Also gives the LaTeX spelling `\zeta`,
    /// which is how the UI notices that a prompt uses this symbol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(deserialize_with = "deserialize_content_blocks")]
    pub body: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackQuestion {
    pub uid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(deserialize_with = "deserialize_content_blocks")]
    pub prompt: Vec<ContentBlock>,
    /// `kind` plus its kind-specific fields, flattened into this object.
    #[serde(flatten)]
    pub body: Body,
    /// Plain-text short explanation. Shorthand for `explain.short`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// The structured explanation: `short` and `deep`, each a list of raw
    /// strings and `{"fact": "uid"}` references.
    #[serde(default, skip_serializing_if = "Explain::is_empty")]
    pub explain: Explain,
    #[serde(default = "default_difficulty")]
    pub difficulty: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackLesson {
    pub uid: String,
    pub topic: String,
    #[serde(default)]
    pub ord: i64,
    pub title: String,
    pub summary: String,
    pub body: Vec<LessonBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub practice: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn default_difficulty() -> u8 {
    2
}

/// Existing packs use a string when content is only prose. New content may
/// use the full ordered array; serialisation always emits that canonical form.
fn deserialize_content_blocks<'de, D>(deserializer: D) -> Result<Vec<ContentBlock>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Authored {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    Ok(match Authored::deserialize(deserializer)? {
        Authored::Text(text) => vec![ContentBlock::text(text)],
        Authored::Blocks(blocks) => blocks,
    })
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
        for f in pack.facts {
            // Two files defining the same fact differently is an authoring
            // mistake worth stopping for; the whole point of a fact is that
            // there is one wording of it.
            match merged.facts.iter().find(|e| e.uid == f.uid) {
                Some(existing) if existing != &f => {
                    bail!("fact {:?} is defined twice, with different content", f.uid)
                }
                Some(_) => {}
                None => merged.facts.push(f),
            }
        }
        merged.questions.extend(pack.questions);
        merged.lessons.extend(pack.lessons);
    }

    Ok(merged)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportReport {
    pub deck_id: Id,
    pub topics: usize,
    pub facts: usize,
    pub questions: usize,
    pub lessons: usize,
    pub retired: usize,
    pub retired_lessons: usize,
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
pub fn import_pack(store: &Store, pack: &Pack) -> Result<ImportReport> {
    // Validate everything before touching the database, so a typo in question
    // 90 does not leave a half-imported deck behind.
    let mut seen_facts: HashSet<&str> = HashSet::new();
    for f in &pack.facts {
        if !seen_facts.insert(f.uid.as_str()) {
            bail!("duplicate fact uid {:?} in pack", f.uid);
        }
        if f.kind == FactKind::Symbol && f.label.is_none() {
            bail!("symbol fact {} has no label", f.uid);
        }
        // A formula sheet entry without its formula is the one thing the
        // sheet exists to carry, so refuse the pack rather than print a gap.
        if f.kind == FactKind::Formula && f.label.is_none() {
            bail!("formula fact {} has no label", f.uid);
        }
        for block in &f.body {
            if let ContentBlock::Figure { figure } = block {
                figure
                    .validate()
                    .map_err(|e| anyhow::anyhow!("fact {}: {e}", f.uid))?;
            }
        }
    }

    let mut seen_uids = HashMap::new();
    for q in &pack.questions {
        if seen_uids.insert(&q.uid, ()).is_some() {
            bail!("duplicate uid {:?} in pack", q.uid);
        }
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
        if let Some(t) = &q.topic
            && !pack.topics.iter().any(|pt| &pt.slug == t)
        {
            bail!("question {} references unknown topic {:?}", q.uid, t);
        }
        // A reference to a fact that is in neither this pack nor the database
        // already would render as a silent gap in the explanation.
        for uid in q.explain.referenced_facts() {
            if !seen_facts.contains(uid) && store.fact(uid)?.is_none() {
                bail!("question {} references unknown fact {:?}", q.uid, uid);
            }
        }
    }

    let question_uids: HashSet<&str> = pack.questions.iter().map(|q| q.uid.as_str()).collect();
    let mut seen_lessons = HashSet::new();
    for lesson in &pack.lessons {
        if !seen_lessons.insert(lesson.uid.as_str()) {
            bail!("duplicate lesson uid {:?} in pack", lesson.uid);
        }
        if !pack.topics.iter().any(|topic| topic.slug == lesson.topic) {
            bail!(
                "lesson {} references unknown topic {:?}",
                lesson.uid,
                lesson.topic
            );
        }
        for block in &lesson.body {
            match block {
                LessonBlock::Fact { fact } => {
                    if !seen_facts.contains(fact.as_str()) && store.fact(fact)?.is_none() {
                        bail!("lesson {} references unknown fact {:?}", lesson.uid, fact);
                    }
                }
                LessonBlock::Figure { figure } => figure
                    .validate()
                    .map_err(|e| anyhow::anyhow!("lesson {}: {e}", lesson.uid))?,
                LessonBlock::Text(_) | LessonBlock::Heading { .. } | LessonBlock::Math { .. } => {}
            }
        }
        for uid in &lesson.practice {
            if !question_uids.contains(uid.as_str()) {
                bail!(
                    "lesson {} references unknown practice question {:?}",
                    lesson.uid,
                    uid
                );
            }
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

    // Facts first: the questions below cite them.
    //
    // Unlike questions, facts are never retired by an import. A pack that only
    // carries one topic must not sweep away the shared glossary, and a fact
    // nobody cites any more costs a row.
    for f in &pack.facts {
        store.upsert_fact(&NewFact {
            deck_id: Some(deck_id),
            uid: f.uid.clone(),
            kind: f.kind,
            label: f.label.clone(),
            name: f.name.clone(),
            title: f.title.clone(),
            body: f.body.clone(),
            source: f.source.clone(),
        })?;
    }

    let retired = store.deactivate_deck_questions(deck_id)?;
    let retired_lessons = store.deactivate_deck_lessons(deck_id)?;

    for q in &pack.questions {
        store.upsert_question(&NewQuestion {
            deck_id,
            topic_id: q.topic.as_ref().and_then(|t| topic_ids.get(t).copied()),
            uid: q.uid.clone(),
            prompt: q.prompt.clone(),
            body: q.body.clone(),
            explanation: q.explanation.clone(),
            explain: q.explain.clone(),
            difficulty: q.difficulty.clamp(1, 5),
            source: q.source.clone(),
            tags: q.tags.clone(),
        })?;
    }

    for lesson in &pack.lessons {
        let topic_id = topic_ids
            .get(&lesson.topic)
            .copied()
            .with_context(|| format!("topic {:?} vanished during import", lesson.topic))?;
        store.upsert_lesson(&NewLesson {
            deck_id,
            topic_id,
            uid: lesson.uid.clone(),
            ord: lesson.ord,
            title: lesson.title.clone(),
            summary: lesson.summary.clone(),
            body: lesson.body.clone(),
            practice: lesson.practice.clone(),
            source: lesson.source.clone(),
        })?;
    }

    Ok(ImportReport {
        deck_id,
        topics: pack.topics.len(),
        facts: pack.facts.len(),
        questions: pack.questions.len(),
        lessons: pack.lessons.len(),
        retired: retired.saturating_sub(pack.questions.len()),
        retired_lessons: retired_lessons.saturating_sub(pack.lessons.len()),
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
        assert_eq!(
            p.questions[0].prompt,
            vec![ContentBlock::text(
                "A BIBO-stable LTI system has all poles in the left half plane."
            )],
            "a string remains the convenient one-paragraph shorthand"
        );
        assert_eq!(p.questions[0].body, Body::TrueFalse { answer: true });
        assert_eq!(p.questions[0].difficulty, 2, "difficulty defaults");
        assert!(matches!(p.questions[1].body, Body::MultipleChoice { .. }));
    }

    #[test]
    fn parses_multiple_interspersed_figures_for_questions_and_facts() {
        let source = r#"{
          "deck": { "slug": "cs", "title": "Control Systems" },
          "facts": [{
            "uid": "comparison",
            "body": [
              "Frequency response:",
              { "figure": { "kind": "nyquist", "num": [2], "den": [1, 4, 3] } },
              "Time response:",
              { "figure": { "kind": "step", "num": [4], "den": [1, 0.4, 4], "t": [0, 20] } }
            ]
          }],
          "questions": [{
            "uid": "cs-plots",
            "prompt": [
              "Inspect both views.",
              { "figure": { "kind": "bode", "num": [1], "den": [1, 10, 0], "phase": true } },
              "Then decide.",
              { "figure": { "kind": "svg", "src": "<svg xmlns=\"http://www.w3.org/2000/svg\"/>" } }
            ],
            "kind": "true_false",
            "answer": true
          }]
        }"#;

        let pack: Pack = serde_json::from_str(source).unwrap();
        assert_eq!(pack.questions[0].prompt.len(), 4);
        assert_eq!(pack.facts[0].body.len(), 4);
        assert!(matches!(
            pack.questions[0].prompt[1],
            ContentBlock::Figure {
                figure: crate::Figure::Bode { .. }
            }
        ));

        let canonical = serde_json::to_value(&pack).unwrap();
        assert!(canonical["questions"][0]["prompt"].is_array());
        assert!(canonical["facts"][0]["body"].is_array());
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
            prompt: vec![ContentBlock::text("broken")],
            body: Body::MultipleChoice {
                options: vec![Choice::new("a", false), Choice::new("b", false)],
                multi: false,
            },
            explanation: None,
            explain: Default::default(),
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

    const FACT_PACK: &str = r#"{
      "deck": { "slug": "cs", "title": "Control Systems" },
      "facts": [
        { "uid": "sym-zeta", "kind": "symbol", "label": "ζ", "name": "zeta",
          "body": "The damping ratio." },
        { "uid": "note-2nd-order", "title": "Standard second-order form",
          "body": "$G(s) = \\frac{\\omega_0^2}{s^2 + 2\\zeta\\omega_0 s + \\omega_0^2}$" }
      ],
      "questions": [
        { "uid": "cs-900", "prompt": "ζ = 1 is critically damped.",
          "kind": "true_false", "answer": true,
          "explain": {
            "short": ["Exactly on the boundary."],
            "deep": ["Standard form: ", { "fact": "note-2nd-order" },
                     " where ", { "fact": "sym-zeta" }, " sets the damping."]
          } }
      ]
    }"#;

    const LESSON_PACK: &str = r#"{
      "deck": { "slug": "cs", "title": "Control Systems" },
      "topics": [{ "slug": "modeling", "title": "Modeling", "ord": 1 }],
      "facts": [{
        "uid": "f-loop", "kind": "formula", "title": "Closed loop",
        "label": "H_C = \\frac{H_O}{1 + H_O}", "body": "Unity feedback."
      }],
      "questions": [{
        "uid": "cs-mod-001", "topic": "modeling", "prompt": "Feedback moves poles.",
        "kind": "true_false", "answer": true
      }],
      "lessons": [{
        "uid": "cs-les-001", "topic": "modeling", "ord": 1,
        "title": "The loop", "summary": "Why feedback changes the plant.",
        "body": [
          "Start with the structure.",
          { "heading": "Closing it" },
          { "fact": "f-loop" },
          { "math": "1 + H_O(s) = 0" },
          { "figure": { "kind": "step", "num": [1], "den": [1, 1], "t": [0, 5] } }
        ],
        "practice": ["cs-mod-001"],
        "source": "Lecture 1"
      }]
    }"#;

    #[test]
    fn parses_facts_and_the_two_readings() {
        let p: Pack = serde_json::from_str(FACT_PACK).unwrap();
        assert_eq!(p.facts.len(), 2);
        assert_eq!(p.facts[0].kind, FactKind::Symbol);
        assert_eq!(p.facts[1].kind, FactKind::Note, "kind defaults to note");

        let e = &p.questions[0].explain;
        assert_eq!(e.short, vec![Seg::text("Exactly on the boundary.")]);
        assert_eq!(e.referenced_facts(), vec!["note-2nd-order", "sym-zeta"]);
    }

    #[test]
    fn a_formula_fact_carries_its_equation_in_the_label() {
        let src = r#"{
          "deck": { "slug": "cs", "title": "Control Systems" },
          "facts": [
            { "uid": "f-peak-time", "kind": "formula", "title": "Peak time",
              "label": "t_p = \\frac{\\pi}{\\omega_d}",
              "body": "Half a period of the damped oscillation." }
          ]
        }"#;
        let p: Pack = serde_json::from_str(src).unwrap();
        assert_eq!(p.facts[0].kind, FactKind::Formula);
        assert_eq!(
            p.facts[0].label.as_deref(),
            Some("t_p = \\frac{\\pi}{\\omega_d}")
        );
    }

    #[test]
    fn a_formula_without_its_formula_is_refused() {
        let src = r#"{
          "deck": { "slug": "cs", "title": "Control Systems" },
          "facts": [
            { "uid": "f-nameless", "kind": "formula", "title": "Peak time",
              "body": "Half a period of the damped oscillation." }
          ]
        }"#;
        let store = Store::open_in_memory().unwrap();
        let pack: Pack = serde_json::from_str(src).unwrap();
        let err = import_pack(&store, &pack).unwrap_err().to_string();
        assert!(err.contains("f-nameless"), "{err}");
    }

    #[test]
    fn imports_facts_alongside_questions() {
        let mut store = Store::open_in_memory().unwrap();
        let pack: Pack = serde_json::from_str(FACT_PACK).unwrap();
        let r = import_pack(&mut store, &pack).unwrap();

        assert_eq!(r.facts, 2);
        assert_eq!(store.fact_count().unwrap(), 2);
        assert_eq!(
            store.fact("sym-zeta").unwrap().unwrap().label.as_deref(),
            Some("ζ")
        );

        let q = store.question_by_uid("cs-900").unwrap().unwrap();
        assert_eq!(q.deep().len(), 5);
    }

    #[test]
    fn parses_and_imports_all_lesson_block_kinds() {
        let pack: Pack = serde_json::from_str(LESSON_PACK).unwrap();
        let body = &pack.lessons[0].body;
        assert!(matches!(body[0], LessonBlock::Text(_)));
        assert!(matches!(body[1], LessonBlock::Heading { .. }));
        assert!(matches!(body[2], LessonBlock::Fact { .. }));
        assert!(matches!(body[3], LessonBlock::Math { .. }));
        assert!(matches!(body[4], LessonBlock::Figure { .. }));

        let store = Store::open_in_memory().unwrap();
        let report = import_pack(&store, &pack).unwrap();
        assert_eq!(report.lessons, 1);
        let lesson = store.lessons(report.deck_id).unwrap().pop().unwrap();
        assert_eq!(lesson.uid, "cs-les-001");
        assert_eq!(lesson.practice, vec!["cs-mod-001"]);
        assert_eq!(
            store
                .questions_by_uids(report.deck_id, &lesson.practice)
                .unwrap()[0]
                .uid,
            "cs-mod-001"
        );
    }

    #[test]
    fn lesson_references_are_validated_before_import() {
        let store = Store::open_in_memory().unwrap();

        let mut bad_topic: Pack = serde_json::from_str(LESSON_PACK).unwrap();
        bad_topic.lessons[0].topic = "missing".into();
        assert!(
            import_pack(&store, &bad_topic)
                .unwrap_err()
                .to_string()
                .contains("unknown topic")
        );

        let mut bad_fact: Pack = serde_json::from_str(LESSON_PACK).unwrap();
        bad_fact.lessons[0].body[2] = LessonBlock::Fact {
            fact: "missing".into(),
        };
        assert!(
            import_pack(&store, &bad_fact)
                .unwrap_err()
                .to_string()
                .contains("unknown fact")
        );

        let mut bad_practice: Pack = serde_json::from_str(LESSON_PACK).unwrap();
        bad_practice.lessons[0].practice = vec!["missing".into()];
        assert!(
            import_pack(&store, &bad_practice)
                .unwrap_err()
                .to_string()
                .contains("unknown practice question")
        );
        assert_eq!(store.decks().unwrap().len(), 0);
    }

    #[test]
    fn reimport_updates_lessons_by_uid_and_retires_removed_ones() {
        let store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(LESSON_PACK).unwrap();
        let deck = import_pack(&store, &pack).unwrap().deck_id;
        let id = store.lessons(deck).unwrap()[0].id;

        pack.lessons[0].title = "The feedback loop".into();
        import_pack(&store, &pack).unwrap();
        assert_eq!(store.lessons(deck).unwrap()[0].id, id);
        assert_eq!(store.lessons(deck).unwrap()[0].title, "The feedback loop");

        pack.lessons.clear();
        import_pack(&store, &pack).unwrap();
        assert_eq!(store.lesson_count(deck).unwrap(), 0);
        let active: i64 = store
            .conn()
            .query_row(
                "SELECT active FROM lesson WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
    }

    #[test]
    fn a_question_may_not_cite_a_fact_that_does_not_exist() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(FACT_PACK).unwrap();
        pack.questions[0].explain.deep.push(Seg::fact("sym-nope"));

        let err = import_pack(&mut store, &pack).unwrap_err().to_string();
        assert!(err.contains("unknown fact"), "{err}");
    }

    #[test]
    fn a_pack_may_cite_a_fact_already_in_the_database() {
        let mut store = Store::open_in_memory().unwrap();
        let full: Pack = serde_json::from_str(FACT_PACK).unwrap();
        import_pack(&mut store, &full).unwrap();

        // Re-importing one topic file on its own, without the glossary.
        let mut lone = full.clone();
        lone.facts.clear();
        assert!(import_pack(&mut store, &lone).is_ok());
        assert_eq!(store.fact_count().unwrap(), 2, "facts are never retired");
    }

    #[test]
    fn a_fact_defined_twice_with_different_wording_is_rejected() {
        let a: Pack = serde_json::from_str(FACT_PACK).unwrap();
        let mut b: Pack = serde_json::from_str(FACT_PACK).unwrap();
        b.facts[0].body = vec![ContentBlock::text("Something else entirely.")];
        let err = merge_packs(vec![a.clone(), b]).unwrap_err().to_string();
        assert!(err.contains("defined twice"), "{err}");

        // The same wording twice is just two files agreeing.
        let merged = merge_packs(vec![a.clone(), a]).unwrap();
        assert_eq!(merged.facts.len(), 2);
    }

    #[test]
    fn a_symbol_without_a_glyph_is_rejected() {
        let mut store = Store::open_in_memory().unwrap();
        let mut pack: Pack = serde_json::from_str(FACT_PACK).unwrap();
        pack.facts[0].label = None;
        let err = import_pack(&mut store, &pack).unwrap_err().to_string();
        assert!(err.contains("no label"), "{err}");
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
            facts: vec![],
            questions: vec![PackQuestion {
                uid: "cs-100".into(),
                topic: Some("control".into()),
                prompt: vec![ContentBlock::text(
                    "A PI controller removes steady-state error.",
                )],
                body: Body::TrueFalse { answer: true },
                explanation: None,
                explain: Default::default(),
                difficulty: 2,
                source: None,
                tags: vec![],
            }],
            lessons: vec![],
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
