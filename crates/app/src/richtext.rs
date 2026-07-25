//! Prose with formulas in it, wrapped to a width.
//!
//! Every authored string in the app goes through here: prompts, options,
//! explanations, facts. Text between `$` is LaTeX and goes to [`crate::math`];
//! everything else is set as ordinary text. The two are laid out on a shared
//! baseline, so `the pole at $s = -1/\tau$ decays` reads as one sentence
//! rather than as a picture dropped into a line of text.
//!
//! Line breaking is greedy and breaks only at spaces: a formula is one
//! unbreakable object. A formula that cannot fit the column at all is set
//! smaller rather than clipped — a truncated equation is a wrong equation.

use eframe::egui::{Color32, FontId, Painter, Pos2, Vec2};
use eframe::epaint::{Galley, TextShape};
use std::sync::Arc;

use crate::card::rotate;
use crate::math::{self, Formula};

/// Laid-out prose, positioned relative to its own top-left corner.
#[derive(Debug, Clone, Default)]
pub struct Doc {
    pieces: Vec<Piece>,
    pub size: Vec2,
}

#[derive(Debug, Clone)]
enum Piece {
    Text {
        pos: Pos2,
        galley: Arc<Galley>,
        emphasized: bool,
    },
    Math {
        pos: Pos2,
        formula: Formula,
    },
}

impl Doc {
    pub fn height(&self) -> f32 {
        self.size.y
    }

    pub fn paint(&self, painter: &Painter, top_left: Pos2, color: Color32, opacity: f32) {
        self.paint_rotated(painter, top_left, top_left, 0.0, color, opacity);
    }

    /// Paint as part of something rotated, so text on a tilted card tilts
    /// with it rigidly.
    pub fn paint_rotated(
        &self,
        painter: &Painter,
        top_left: Pos2,
        pivot: Pos2,
        angle: f32,
        color: Color32,
        opacity: f32,
    ) {
        for piece in &self.pieces {
            match piece {
                Piece::Text {
                    pos,
                    galley,
                    emphasized,
                } => {
                    let at = rotate(*pos + top_left.to_vec2(), pivot, angle);
                    let ink = if *emphasized {
                        color.gamma_multiply(1.18)
                    } else {
                        color
                    };
                    painter.add(
                        TextShape::new(at, galley.clone(), ink)
                            .with_override_text_color(ink)
                            .with_angle(angle)
                            .with_opacity_factor(opacity),
                    );
                }
                Piece::Math { pos, formula } => {
                    formula.paint_rotated(
                        painter,
                        *pos + top_left.to_vec2(),
                        pivot,
                        angle,
                        color,
                        opacity,
                    );
                }
            }
        }
    }
}

/// One thing that can sit on a line.
enum Token {
    Word {
        text: String,
        emphasized: bool,
    },
    Math(String),
    /// A hard line break; two in a row start a new paragraph.
    Break,
}

/// Split source into words, formulas and line breaks.
///
/// `$` opens and closes a formula; `\$` is a literal dollar sign. An unclosed
/// `$` takes the rest of the string, which is what an author who forgot the
/// closing one meant anyway.
fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut chars = src.chars().peekable();

    let flush = |word: &mut String, out: &mut Vec<Token>| {
        if !word.is_empty() {
            let mut text = std::mem::take(word);
            let emphasized = match (text.find('*'), text.rfind('*')) {
                (Some(start), Some(end)) if end > start + 1 => {
                    text.remove(end);
                    text.remove(start);
                    true
                }
                _ => false,
            };
            out.push(Token::Word { text, emphasized });
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'$') => {
                chars.next();
                word.push('$');
            }
            '$' => {
                flush(&mut word, &mut out);
                let mut formula = String::new();
                for c in chars.by_ref() {
                    if c == '$' {
                        break;
                    }
                    formula.push(c);
                }
                out.push(Token::Math(formula));
            }
            '\n' => {
                flush(&mut word, &mut out);
                out.push(Token::Break);
            }
            c if c.is_whitespace() => flush(&mut word, &mut out),
            c => word.push(c),
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Lay out a single text run and report where its baseline sits.
fn text_run(painter: &Painter, s: &str, font: FontId) -> (Arc<Galley>, f32, f32) {
    let galley = painter.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE);
    let baseline = galley
        .rows
        .first()
        .map(|r| r.pos.y + r.row.glyphs.first().map_or(font.size * 0.8, |g| g.pos.y))
        .unwrap_or(font.size * 0.8);
    let descent = (galley.rect.height() - baseline).max(0.0);
    (galley, baseline, descent)
}

/// One piece of a line, before the line's baseline is known.
struct Placed {
    piece: Piece,
    ascent: f32,
    descent: f32,
}

pub fn layout(painter: &Painter, src: &str, size: f32, wrap: f32) -> Doc {
    let font = FontId::new(size, eframe::egui::FontFamily::Proportional);
    let space = painter
        .layout_no_wrap(" ".into(), font.clone(), Color32::WHITE)
        .rect
        .width();
    // A blank line's height, so an empty paragraph still separates.
    let (_, line_ascent, line_descent) = text_run(painter, "Xg", font.clone());

    let mut pieces: Vec<Piece> = Vec::new();
    let mut line: Vec<Placed> = Vec::new();
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut width_used = 0.0f32;

    // Flush the current line: now that its tallest piece is known, put
    // everything on the shared baseline.
    let flush_line = |line: &mut Vec<Placed>,
                      x: &mut f32,
                      y: &mut f32,
                      pieces: &mut Vec<Piece>,
                      width_used: &mut f32| {
        let ascent = line.iter().map(|p| p.ascent).fold(line_ascent, f32::max);
        let descent = line.iter().map(|p| p.descent).fold(line_descent, f32::max);
        let baseline = *y + ascent;

        for placed in line.drain(..) {
            match placed.piece {
                Piece::Text {
                    pos,
                    galley,
                    emphasized,
                } => pieces.push(Piece::Text {
                    pos: Pos2::new(pos.x, baseline - placed.ascent),
                    galley,
                    emphasized,
                }),
                Piece::Math { pos, formula } => pieces.push(Piece::Math {
                    pos: Pos2::new(pos.x, baseline - placed.ascent),
                    formula,
                }),
            }
        }
        *width_used = width_used.max(*x);
        *y = baseline + descent;
        *x = 0.0;
    };

    let tokens = tokenize(src);
    let mut prev_break = false;

    for token in tokens {
        match token {
            Token::Break => {
                // A second break in a row is a paragraph gap, not an empty
                // line: consecutive newlines should not run away vertically.
                if prev_break && line.is_empty() {
                    y += (line_ascent + line_descent) * 0.5;
                } else {
                    flush_line(&mut line, &mut x, &mut y, &mut pieces, &mut width_used);
                }
                prev_break = true;
                continue;
            }
            Token::Word { text, emphasized } => {
                let (galley, ascent, descent) = text_run(painter, &text, font.clone());
                let width = galley.rect.width();
                let lead = if line.is_empty() { 0.0 } else { space };
                if !line.is_empty() && x + lead + width > wrap {
                    flush_line(&mut line, &mut x, &mut y, &mut pieces, &mut width_used);
                }
                let lead = if line.is_empty() { 0.0 } else { space };
                x += lead;
                line.push(Placed {
                    piece: Piece::Text {
                        pos: Pos2::new(x, 0.0),
                        galley,
                        emphasized,
                    },
                    ascent,
                    descent,
                });
                x += width;
            }
            Token::Math(src) => {
                let mut formula = math::layout(painter, &src, size * 1.02);
                // Too wide for the column even on a line of its own: set it
                // smaller rather than let it run off the edge.
                if formula.size.x > wrap && formula.size.x > 0.0 {
                    let shrunk = (size * 1.02 * wrap / formula.size.x).max(9.0);
                    formula = math::layout(painter, &src, shrunk);
                }
                let lead = if line.is_empty() { 0.0 } else { space };
                if !line.is_empty() && x + lead + formula.size.x > wrap {
                    flush_line(&mut line, &mut x, &mut y, &mut pieces, &mut width_used);
                }
                let lead = if line.is_empty() { 0.0 } else { space };
                x += lead;
                let (width, ascent, descent) = (formula.size.x, formula.ascent, formula.descent());
                line.push(Placed {
                    piece: Piece::Math {
                        pos: Pos2::new(x, 0.0),
                        formula,
                    },
                    ascent,
                    descent,
                });
                x += width;
            }
        }
        prev_break = false;
    }

    if !line.is_empty() {
        flush_line(&mut line, &mut x, &mut y, &mut pieces, &mut width_used);
    }

    Doc {
        pieces,
        size: Vec2::new(width_used.min(wrap.max(0.0)).max(0.0), y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<&'static str> {
        tokenize(src)
            .iter()
            .map(|t| match t {
                Token::Word { .. } => "word",
                Token::Math(_) => "math",
                Token::Break => "break",
            })
            .collect()
    }

    fn texts(src: &str) -> Vec<String> {
        tokenize(src)
            .into_iter()
            .filter_map(|t| match t {
                Token::Word { text, .. } => Some(text),
                Token::Math(m) => Some(format!("${m}$")),
                Token::Break => None,
            })
            .collect()
    }

    #[test]
    fn prose_splits_into_words() {
        assert_eq!(kinds("a bc  d"), ["word", "word", "word"]);
    }

    #[test]
    fn asterisks_mark_emphasis_instead_of_becoming_ink() {
        let tokens = tokenize("this is *important*.");
        let Token::Word { text, emphasized } = &tokens[2] else {
            panic!("expected an emphasized word");
        };
        assert_eq!(text, "important.");
        assert!(emphasized);
    }

    #[test]
    fn dollars_delimit_a_formula() {
        assert_eq!(
            kinds(r"the pole $s = -\frac{1}{\tau}$ decays"),
            ["word", "word", "math", "word"]
        );
        assert_eq!(
            texts(r"at $s=-1$ exactly")[1],
            "$s=-1$",
            "the formula source is kept verbatim"
        );
    }

    #[test]
    fn an_escaped_dollar_is_literal() {
        assert_eq!(kinds(r"costs \$5 today"), ["word", "word", "word"]);
        assert_eq!(texts(r"costs \$5")[1], "$5");
    }

    #[test]
    fn an_unclosed_formula_takes_the_rest() {
        // An author who forgot the closing `$` still gets their formula.
        assert_eq!(kinds(r"see $x^2"), ["word", "math"]);
        assert_eq!(texts(r"see $x^2")[1], "$x^2$");
    }

    #[test]
    fn newlines_survive_as_breaks() {
        assert_eq!(kinds("a\n\nb"), ["word", "break", "break", "word"]);
    }
}
