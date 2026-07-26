//! Explanations: the short reading, the deep one, and the facts they cite.
//!
//! A question's explanation is a list of raw text and references into the
//! fact table (see `core::model::Seg`), so ten variants of one idea share a
//! single wording of it. This module turns that list into something on
//! screen, and it is the only place that knows what a fact looks like.
//!
//! The deep reading also gets a glossary: every symbol fact whose glyph
//! actually occurs in the question is appended, so `ζ` on the card is never
//! left as a shape you are expected to already recognise. A lesson gets the
//! same glossary under its body, for the same reason — a reading uses more
//! symbols than any one card does, not fewer.

use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use idiosepius_core::{
    Fact, FactKind, Id, Lesson, LessonBlock, Question, Seg, Store, content_text, content_transcript,
};

use crate::blocks;
use crate::richtext;
use crate::theme::{Palette, text, tracked};

/// Every fact available while studying a deck.
///
/// Held for the whole session rather than queried per card: it is a few
/// hundred short rows, any explanation may cite any of them, and a card
/// flipping over is not a good moment to go to the database.
#[derive(Default)]
pub struct Facts {
    by_uid: HashMap<String, Fact>,
    /// Symbol facts, longest glyph first, so `ω₀` is matched before `ω`.
    symbols: Vec<Fact>,
}

impl Facts {
    pub fn load(store: &Store, deck_id: Id) -> Self {
        let all = store.facts(deck_id).unwrap_or_default();
        let mut symbols: Vec<Fact> = all
            .iter()
            .filter(|f| f.kind == FactKind::Symbol)
            .cloned()
            .collect();
        symbols
            .sort_by_key(|f| std::cmp::Reverse(f.label.as_ref().map_or(0, |l| l.chars().count())));

        Facts {
            by_uid: all.into_iter().map(|f| (f.uid.clone(), f)).collect(),
            symbols,
        }
    }

    pub fn get(&self, uid: &str) -> Option<&Fact> {
        self.by_uid.get(uid)
    }

    /// The symbols that actually appear in `text`, in the order they were
    /// authored — longest glyph first, so a match on `ω₀` does not also drag
    /// in a redundant entry for `ω`.
    pub fn symbols_in(&self, text: &str) -> Vec<&Fact> {
        let mut out: Vec<&Fact> = Vec::new();
        let mut claimed = String::new();
        for f in &self.symbols {
            if !f.appears_in(text) {
                continue;
            }
            // Skip a symbol already contained in a longer one we matched.
            if let Some(l) = &f.label
                && claimed.contains(l.as_str())
            {
                continue;
            }
            if let Some(l) = &f.label {
                claimed.push_str(l);
            }
            out.push(f);
        }
        out
    }
}

/// Which of a question's authored option notes may be shown.
///
/// A note is addressed to whoever picked that option — "That is the settling
/// time", "Inverted — check the units" — so who is reading decides which ones
/// belong on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteView {
    /// Nothing has been answered. A note names a wrong option, so showing one
    /// leaks the answer exactly as surely as marking the option would.
    Hidden,
    /// Only the options actually selected. Answering is a diagnosis of *your*
    /// mistake; every note at once turns that into a wall.
    Picked,
    /// Every note on the card. Right on the review screen, where the card is
    /// being studied rather than answered: the set of notes is a map of the
    /// mistakes the question was built to catch.
    All,
}

/// The note to draw under each option, in option order.
///
/// `None` where there is nothing to say — either the option carries no note,
/// or this reader is not entitled to it.
pub fn option_notes<'a>(
    options: &'a [idiosepius_core::Choice],
    picked: &[usize],
    view: NoteView,
) -> Vec<Option<&'a str>> {
    options
        .iter()
        .enumerate()
        .map(|(i, option)| {
            let visible = match view {
                NoteView::Hidden => false,
                NoteView::Picked => picked.contains(&i),
                NoteView::All => true,
            };
            if !visible {
                return None;
            }
            option
                .note
                .as_deref()
                .map(str::trim)
                .filter(|note| !note.is_empty())
        })
        .collect()
}

/// Which reading is on screen. `l`/`d` toggles between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Short,
    Deep,
}

impl Depth {
    pub fn toggled(self) -> Self {
        match self {
            Depth::Short => Depth::Deep,
            Depth::Deep => Depth::Short,
        }
    }
}

/// Draw a block of prose, wrapped to the current column.
pub fn prose(ui: &mut Ui, s: &str, size: f32, color: Color32) {
    let wrap = ui.available_width().max(40.0);
    let doc = richtext::layout(ui.painter(), s, size, wrap);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(wrap, doc.height()), Sense::hover());
    doc.paint(ui.painter(), rect.min, color, 1.0);
}

/// One fact, as an inset block with a rail down its left edge.
///
/// Facts are visibly *quoted* rather than woven into the sentence: the point
/// of a shared fact is that it is the same wherever it appears, and it should
/// be recognisable as the same thing on the fifth card that cites it.
pub fn fact_block(ui: &mut Ui, fact: &Fact) {
    let indent = 14.0;
    let rail = if fact.kind == FactKind::Symbol {
        Palette::VIOLET
    } else {
        Palette::ACCENT
    };

    let top = ui.cursor().top();
    ui.horizontal_top(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            match fact.kind {
                FactKind::Symbol => {
                    // Glyph, then its name: "ζ  ZETA".
                    let head = match (&fact.label, &fact.name) {
                        (Some(l), Some(n)) => format!("{l}   {}", tracked(n)),
                        (Some(l), None) => l.clone(),
                        _ => String::new(),
                    };
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
                    ui.painter().text(
                        rect.left_top(),
                        Align2::LEFT_TOP,
                        head,
                        text::body(),
                        Palette::VIOLET,
                    );
                }
                FactKind::Note | FactKind::Formula => {
                    if let Some(title) = &fact.title {
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 18.0),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            rect.left_top(),
                            Align2::LEFT_TOP,
                            tracked(title),
                            text::label(),
                            Palette::ACCENT,
                        );
                    }
                    // The equation gets a display line of its own. A formula
                    // cited mid-derivation has to be readable at a glance,
                    // which it is not when it is buried in a sentence.
                    if fact.kind == FactKind::Formula
                        && let Some(f) = &fact.label
                    {
                        ui.add_space(2.0);
                        prose(ui, &format!("${f}$"), 16.5, Palette::TEXT);
                        ui.add_space(2.0);
                    }
                }
            }
            blocks::show(ui, &fact.body, 14.5, Palette::TEXT);
            if let Some(src) = &fact.source {
                prose(ui, src, 11.5, Palette::TEXT_FAINT);
            }
        });
    });
    let bottom = ui.cursor().top();

    let x = ui.max_rect().left() + 2.0;
    ui.painter().rect_filled(
        Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + 2.0, bottom - 6.0)),
        0,
        rail,
    );
}

/// The whole explanation for one question, at the requested depth.
///
/// Returns false when there was nothing to say, so a caller can decide
/// whether the panel is worth showing at all.
pub fn body(ui: &mut Ui, q: &Question, facts: &Facts, depth: Depth) -> bool {
    let segments = segments(q, depth);

    let mut said_something = false;
    for seg in &segments {
        match seg {
            Seg::Text(s) if s.trim().is_empty() => {}
            Seg::Text(s) => {
                prose(ui, s, 15.0, Palette::TEXT_DIM);
                said_something = true;
            }
            Seg::Fact { fact } => {
                if let Some(f) = facts.get(fact) {
                    fact_block(ui, f);
                    said_something = true;
                }
            }
        }
    }

    if depth == Depth::Deep {
        said_something |= symbols(ui, q, facts, &segments);
    }
    said_something
}

/// The glossary under a deep explanation: every symbol the question uses that
/// its own text did not already stop to define.
fn symbols(ui: &mut Ui, q: &Question, facts: &Facts, shown: &[Seg]) -> bool {
    glossary(ui, &used_symbols(q, facts, shown))
}

/// A "symbols" section: the rule, then one symbol fact per line.
///
/// Shared by the deep reading and the lesson reader so the two cannot drift
/// into two different-looking glossaries.
pub fn glossary(ui: &mut Ui, found: &[&Fact]) -> bool {
    if found.is_empty() {
        return false;
    }

    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
    ui.painter().text(
        rect.left_top(),
        Align2::LEFT_TOP,
        tracked("symbols"),
        text::label(),
        Palette::TEXT_FAINT,
    );
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + 70.0, rect.top() + 8.0),
            Pos2::new(rect.right(), rect.top() + 8.0),
        ],
        Stroke::new(1.0, Palette::LINE),
    );

    for f in found {
        fact_block(ui, f);
    }
    true
}

/// The symbols a lesson uses and does not itself stop to define.
///
/// The context is everything the reader can see: the prose, the display maths,
/// and the facts the lesson quotes — most of a lesson's notation arrives
/// inside a quoted formula rather than in its own sentences, so a glossary
/// built from the prose alone would miss exactly the symbols worth naming.
pub fn lesson_symbols<'a>(lesson: &Lesson, facts: &'a Facts) -> Vec<&'a Fact> {
    let mut context = lesson.summary.clone();
    let mut push = |s: &str| {
        context.push(' ');
        context.push_str(s);
    };
    for block in &lesson.body {
        match block {
            LessonBlock::Text(text) => push(text),
            LessonBlock::Heading { heading } => push(heading),
            LessonBlock::Math { math } => push(math),
            LessonBlock::Figure { .. } => {}
            LessonBlock::Fact { fact } => {
                if let Some(f) = facts.get(fact) {
                    if let Some(label) = &f.label {
                        push(label);
                    }
                    push(&content_transcript(&f.body));
                }
            }
        }
    }

    // Match on glyphs, whether the author wrote `ζ` or `\zeta`.
    let context = crate::math::unicodify(&context);

    let already: Vec<&str> = lesson
        .body
        .iter()
        .filter_map(LessonBlock::fact_uid)
        .collect();
    facts
        .symbols_in(&context)
        .into_iter()
        .filter(|f| !already.contains(&f.uid.as_str()))
        .collect()
}

fn segments(q: &Question, depth: Depth) -> Vec<Seg> {
    match depth {
        Depth::Short => q.short(),
        // Falling back to the short reading rather than showing an empty
        // panel: content is authored over time, and a card whose deep reading
        // has not been written yet still has something to say.
        Depth::Deep if !q.deep().is_empty() => q.deep(),
        Depth::Deep => q.short(),
    }
}

fn used_symbols<'a>(q: &Question, facts: &'a Facts, shown: &[Seg]) -> Vec<&'a Fact> {
    let mut context = content_text(&q.prompt);
    if let idiosepius_core::Body::MultipleChoice { options, .. } = &q.body {
        for o in options {
            context.push(' ');
            context.push_str(&o.text);
        }
    }
    for seg in shown {
        if let Seg::Text(s) = seg {
            context.push(' ');
            context.push_str(s);
        }
    }

    // Match on glyphs, whether the author wrote `ζ` or `\zeta`.
    let context = crate::math::unicodify(&context);

    let already: Vec<&str> = shown.iter().filter_map(|s| s.fact_uid()).collect();
    let found: Vec<&Fact> = facts
        .symbols_in(&context)
        .into_iter()
        .filter(|f| !already.contains(&f.uid.as_str()))
        .collect();
    found
}

/// The explanation currently shown, formatted for the clipboard.
///
/// Authored math stays as `$...$` LaTeX instead of being flattened to the
/// screen glyphs, so the result can be pasted into notes or a chatbot without
/// losing the formula structure.
pub fn plain_text(q: &Question, facts: &Facts, depth: Depth) -> String {
    let segments = segments(q, depth);
    let mut blocks = Vec::new();

    for seg in &segments {
        match seg {
            Seg::Text(s) if !s.trim().is_empty() => blocks.push(s.trim().to_owned()),
            Seg::Fact { fact } => {
                if let Some(f) = facts.get(fact) {
                    blocks.push(fact_text(f));
                }
            }
            Seg::Text(_) => {}
        }
    }

    if depth == Depth::Deep {
        let symbols = used_symbols(q, facts, &segments);
        if !symbols.is_empty() {
            let mut glossary = String::from("Symbols");
            for fact in symbols {
                glossary.push_str("\n\n");
                glossary.push_str(&fact_text(fact));
            }
            blocks.push(glossary);
        }
    }

    blocks.join("\n\n")
}

fn fact_text(fact: &Fact) -> String {
    let heading = match fact.kind {
        FactKind::Symbol => match (&fact.label, &fact.name) {
            (Some(label), Some(name)) => format!("{label} ({name})"),
            (Some(label), None) => label.clone(),
            (None, Some(name)) => name.clone(),
            (None, None) => String::new(),
        },
        FactKind::Note => fact.title.clone().unwrap_or_default(),
        // The transcript keeps authored LaTeX, so the formula travels as
        // `$...$` rather than as whatever the screen happened to draw.
        FactKind::Formula => match (&fact.title, &fact.label) {
            (Some(title), Some(f)) => format!("{title}:  ${f}$"),
            (None, Some(f)) => format!("${f}$"),
            (Some(title), None) => title.clone(),
            (None, None) => String::new(),
        },
    };

    let mut parts = Vec::new();
    if !heading.trim().is_empty() {
        parts.push(heading);
    }
    let body = content_transcript(&fact.body);
    if !body.trim().is_empty() {
        parts.push(body);
    }
    if let Some(source) = &fact.source
        && !source.trim().is_empty()
    {
        parts.push(format!("Source: {}", source.trim()));
    }
    parts.join("\n")
}

/// Run `add` inside a scrolling column clipped to `rect`.
///
/// A deep explanation with a glossary is routinely taller than the panel it
/// sits in, and a explanation you cannot reach the bottom of is worse than
/// none.
pub fn scroll_column<R>(
    ui: &mut Ui,
    rect: Rect,
    id: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if rect.height() <= 1.0 || rect.width() <= 1.0 {
        return None;
    }
    let out = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.spacing_mut().scroll.bar_width = 5.0;
        ui.spacing_mut().scroll.floating = false;
        ui.spacing_mut().scroll.bar_inner_margin = 4.0;
        egui::ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false])
            .show(ui, add)
            .inner
    });
    Some(out.inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use idiosepius_core::{Body, Explain};

    #[test]
    fn clipboard_text_expands_fact_references_without_flattening_latex() {
        let fact = Fact {
            id: 1,
            deck_id: Some(1),
            uid: "dc-gain".into(),
            kind: FactKind::Note,
            label: None,
            name: None,
            title: Some("DC gain".into()),
            body: vec![idiosepius_core::ContentBlock::text(
                r"Evaluate $G(s)$ at $s=0$.",
            )],
            source: Some("Lecture 2".into()),
        };
        let facts = Facts {
            by_uid: HashMap::from([(fact.uid.clone(), fact)]),
            symbols: Vec::new(),
        };
        let question = Question {
            id: 1,
            deck_id: 1,
            topic_id: None,
            uid: "q".into(),
            prompt: vec![idiosepius_core::ContentBlock::text(r"Find $G(0)$.")],
            body: Body::TrueFalse { answer: true },
            explanation: None,
            explain: Explain {
                short: vec![Seg::fact("dc-gain")],
                deep: Vec::new(),
            },
            difficulty: 1,
            source: None,
            tags: Vec::new(),
        };

        assert_eq!(
            plain_text(&question, &facts, Depth::Short),
            "DC gain\nEvaluate $G(s)$ at $s=0$.\nSource: Lecture 2"
        );
    }

    fn symbol(uid: &str, label: &str, name: &str) -> Fact {
        Fact {
            id: 1,
            deck_id: Some(1),
            uid: uid.into(),
            kind: FactKind::Symbol,
            label: Some(label.into()),
            name: Some(name.into()),
            title: None,
            body: Vec::new(),
            source: None,
        }
    }

    fn lesson(body: Vec<LessonBlock>) -> Lesson {
        Lesson {
            id: 1,
            deck_id: 1,
            topic_id: 1,
            uid: "les".into(),
            ord: 1,
            title: "A reading".into(),
            summary: String::new(),
            body,
            practice: Vec::new(),
            source: None,
        }
    }

    #[test]
    fn a_lesson_glossary_reaches_into_quoted_formulas_but_not_past_its_own_definitions() {
        let zeta = symbol("sym-zeta", "ζ", "zeta");
        let omega = symbol("sym-omega-0", "ω₀", "omega zero");
        let formula = Fact {
            id: 3,
            deck_id: Some(1),
            uid: "f-settling".into(),
            kind: FactKind::Formula,
            // The notation a reading uses mostly arrives inside a quoted
            // formula, never in a sentence of its own.
            label: Some(r"t_{se} \approx \frac{3}{\zeta\omega_0}".into()),
            name: None,
            title: Some("Settling time".into()),
            body: Vec::new(),
            source: None,
        };
        let facts = Facts {
            by_uid: HashMap::from([
                (zeta.uid.clone(), zeta.clone()),
                (omega.uid.clone(), omega.clone()),
                (formula.uid.clone(), formula.clone()),
            ]),
            symbols: vec![omega, zeta],
        };

        let found = lesson_symbols(
            &lesson(vec![
                LessonBlock::Text("Settling depends on the product.".into()),
                LessonBlock::Fact {
                    fact: "f-settling".into(),
                },
            ]),
            &facts,
        );
        assert_eq!(
            found.iter().map(|f| f.uid.as_str()).collect::<Vec<_>>(),
            ["sym-omega-0", "sym-zeta"]
        );

        // A lesson that stops to define ζ itself must not define it twice.
        let found = lesson_symbols(
            &lesson(vec![
                LessonBlock::Fact {
                    fact: "f-settling".into(),
                },
                LessonBlock::Fact {
                    fact: "sym-zeta".into(),
                },
            ]),
            &facts,
        );
        assert_eq!(
            found.iter().map(|f| f.uid.as_str()).collect::<Vec<_>>(),
            ["sym-omega-0"]
        );
    }
}
