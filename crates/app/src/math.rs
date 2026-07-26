//! A LaTeX math renderer: parser, box layout, and painting.
//!
//! Formulas are the content of this deck. `ω₀²/(s² + 2ζω₀s + ω₀²)` written in
//! Unicode is legible but flat — a fraction is not a slash, an exponent is not
//! a lookalike codepoint, and half of what an exam asks about has no Unicode
//! spelling at all. So `$…$` spans are real LaTeX, laid out here.
//!
//! This is a TeX-shaped box model rather than a full TeX: source is parsed
//! into a [`Node`] tree, each node becomes a box that knows its own width,
//! ascent and descent, and boxes are glued together on a shared baseline. It
//! covers what the course actually writes — fractions, radicals, scripts,
//! sized fences, sums and integrals, matrices, accents and the symbol tables.
//!
//! Why not a real TeX engine (ReX and friends): they need an OpenType *math*
//! font vendored into the repository, and they emit glyph outlines, which
//! epaint cannot fill for concave shapes. Everything here draws with the fonts
//! the UI already loaded, plus a handful of shapes (radical, sum, integral)
//! drawn by hand so a missing glyph can never leave a hole in a formula.
//!
//! Unknown input never panics and never disappears: an unrecognised command
//! renders as its own source text, where the author can see it.

use eframe::egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use eframe::epaint::{CubicBezierShape, Galley, PathStroke, Shape, TextShape};
use std::sync::Arc;

use crate::card::rotate;

// ------------------------------------------------------------------ parse --

/// How an atom spaces against its neighbours, TeX's atom classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Ordinary: variables, digits, most symbols.
    Ord,
    /// Binary operator: `+`, `-`, `\cdot`.
    Bin,
    /// Relation: `=`, `<`, `\approx`.
    Rel,
    /// Punctuation: `,`, `;`.
    Punct,
    /// A named function: `\sin`, `\lim`.
    Func,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Dot,
    DDot,
    Hat,
    Bar,
    Vec,
    Tilde,
}

/// A big operator drawn by hand rather than taken from the font, because a
/// missing `∑` would otherwise leave a hole in the middle of a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Big {
    Sum,
    Prod,
    /// `\int`, `\iint`, `\iiint`: one, two or three integral signs, kerned
    /// tighter than writing the command that many times would give.
    Int(u8),
    Oint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Row(Vec<Node>),
    /// A run of characters set as they are.
    Sym(String, Class),
    /// `\mathbb{…}`: double-struck letters, drawn rather than looked up.
    Bb(String),
    Frac(Box<Node>, Box<Node>),
    Sqrt {
        index: Option<Box<Node>>,
        body: Box<Node>,
    },
    Script {
        base: Box<Node>,
        sup: Option<Box<Node>>,
        sub: Option<Box<Node>>,
    },
    /// `\left( … \right)`, with the fences grown to fit.
    Fenced {
        left: Option<char>,
        right: Option<char>,
        body: Box<Node>,
    },
    Accented {
        accent: Accent,
        base: Box<Node>,
    },
    /// A big operator with its limits, which sit above and below when the
    /// operator takes them that way.
    BigOp {
        op: Big,
        limits: bool,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Matrix {
        rows: Vec<Vec<Node>>,
        left: Option<char>,
        right: Option<char>,
        /// `cases` aligns left; every other environment centres.
        align_left: bool,
        /// Explicit `array` column spec; empty for the matrix environments.
        column_align: Vec<ColumnAlign>,
        /// Column-boundary indices carrying a vertical rule.
        vertical_rules: Vec<usize>,
        /// Row-boundary indices carrying a horizontal `\hline`.
        horizontal_rules: Vec<usize>,
    },
    /// Explicit space, in multiples of the current font size.
    Space(f32),
    Empty,
}

pub fn parse(src: &str) -> Node {
    let chars: Vec<char> = src.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    p.row(Stop::End)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    End,
    Brace,
    Right,
    EndEnv,
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.i += 1;
        }
    }

    /// A command name: letters after a backslash, or a single punctuation
    /// character (`\,`, `\{`, `\\`).
    fn command(&mut self) -> String {
        if matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            let start = self.i;
            while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
                self.i += 1;
            }
            self.s[start..self.i].iter().collect()
        } else {
            self.bump().map(String::from).unwrap_or_default()
        }
    }

    /// The word inside braces after `\begin` / `\end`.
    fn braced_word(&mut self) -> String {
        self.skip_space();
        if !self.eat('{') {
            return String::new();
        }
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == '}' {
                break;
            }
            self.i += 1;
        }
        let word: String = self.s[start..self.i].iter().collect();
        self.eat('}');
        word
    }

    fn at_stop(&self, stop: Stop) -> bool {
        match self.peek() {
            None => true,
            Some('}') if stop == Stop::Brace => true,
            Some('&') => stop == Stop::EndEnv,
            Some('\\') => {
                let rest: String = self.s[self.i..].iter().take(7).collect();
                match stop {
                    Stop::Right => rest.starts_with("\\right"),
                    Stop::EndEnv => {
                        rest.starts_with("\\end")
                            || rest.starts_with("\\\\")
                            || rest.starts_with("\\hline")
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn row(&mut self, stop: Stop) -> Node {
        let mut out: Vec<Node> = Vec::new();
        loop {
            self.skip_space();
            if self.at_stop(stop) {
                break;
            }
            let Some(atom) = self.atom(stop) else { break };
            let atom = self.scripts(atom);
            out.push(atom);
        }
        match out.len() {
            0 => Node::Empty,
            1 => out.pop().unwrap(),
            _ => Node::Row(out),
        }
    }

    /// Attach any `^`/`_` that follow an atom, in either order.
    fn scripts(&mut self, base: Node) -> Node {
        let mut sup = None;
        let mut sub = None;
        loop {
            self.skip_space();
            match self.peek() {
                Some('^') if sup.is_none() => {
                    self.i += 1;
                    sup = Some(Box::new(self.arg()));
                }
                Some('_') if sub.is_none() => {
                    self.i += 1;
                    sub = Some(Box::new(self.arg()));
                }
                _ => break,
            }
        }
        if sup.is_none() && sub.is_none() {
            return base;
        }
        // A big operator takes its scripts as limits rather than as corners.
        if let Node::BigOp { op, limits, .. } = base {
            return Node::BigOp {
                op,
                limits,
                sub,
                sup,
            };
        }
        Node::Script {
            base: Box::new(base),
            sup,
            sub,
        }
    }

    /// One argument: a braced group, or the single next atom.
    ///
    /// "Single" is literal, as in TeX: `\frac12` is one half, not one twelfth.
    /// Digits only clump into a number when nothing is claiming them.
    fn arg(&mut self) -> Node {
        self.skip_space();
        if self.eat('{') {
            let n = self.row(Stop::Brace);
            self.eat('}');
            return n;
        }
        if matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
            let d = self.bump().expect("just peeked");
            return Node::Sym(d.to_string(), Class::Ord);
        }
        self.atom(Stop::End).unwrap_or(Node::Empty)
    }

    fn atom(&mut self, stop: Stop) -> Option<Node> {
        self.skip_space();
        let c = self.peek()?;

        if c == '{' {
            self.i += 1;
            let n = self.row(Stop::Brace);
            self.eat('}');
            return Some(n);
        }
        if c == '}' || c == '&' {
            return None;
        }
        if c == '\\' {
            self.i += 1;
            return self.command_atom(stop);
        }
        self.i += 1;

        // Digits clump: `12.5` is one atom, so a following script attaches to
        // the whole number rather than to the last digit.
        if c.is_ascii_digit() {
            let start = self.i - 1;
            while matches!(self.peek(), Some(d) if d.is_ascii_digit() || d == '.') {
                self.i += 1;
            }
            let s: String = self.s[start..self.i].iter().collect();
            return Some(Node::Sym(s, Class::Ord));
        }

        Some(Node::Sym(c.to_string(), class_of(c)))
    }

    fn command_atom(&mut self, stop: Stop) -> Option<Node> {
        let name = self.command();
        let node = match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.arg();
                let den = self.arg();
                Node::Frac(Box::new(num), Box::new(den))
            }

            "sqrt" => {
                self.skip_space();
                let index = if self.eat('[') {
                    let n = self.row(Stop::End);
                    // `row` stops at `]` only by falling through to atom, so
                    // consume up to it explicitly.
                    self.eat(']');
                    Some(Box::new(n))
                } else {
                    None
                };
                Node::Sqrt {
                    index,
                    body: Box::new(self.arg()),
                }
            }

            "left" => {
                self.skip_space();
                let left = self.fence();
                let body = self.row(Stop::Right);
                // `\right` and its delimiter.
                if self.eat('\\') {
                    let _ = self.command();
                    self.skip_space();
                }
                let right = self.fence();
                Node::Fenced {
                    left,
                    right,
                    body: Box::new(body),
                }
            }

            "right" => {
                // Unbalanced `\right`: swallow the delimiter and carry on.
                self.skip_space();
                self.fence();
                Node::Empty
            }

            "begin" => {
                let env = self.braced_word();
                self.matrix(&env)
            }

            "end" => {
                let _ = self.braced_word();
                Node::Empty
            }

            "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathcal" | "operatorname" => {
                Node::Sym(self.literal_arg(), Class::Ord)
            }

            // Blackboard bold has to say "a set of numbers", which a
            // passed-through `R` does not.
            "mathbb" => Node::Bb(self.literal_arg()),

            "dot" => self.accent(Accent::Dot),
            "ddot" => self.accent(Accent::DDot),
            "hat" | "widehat" => self.accent(Accent::Hat),
            "bar" | "overline" => self.accent(Accent::Bar),
            "vec" => self.accent(Accent::Vec),
            "tilde" | "widetilde" => self.accent(Accent::Tilde),

            "sum" => big(Big::Sum, true),
            "prod" => big(Big::Prod, true),
            "int" => big(Big::Int(1), false),
            "iint" => big(Big::Int(2), false),
            "iiint" => big(Big::Int(3), false),
            "oint" => big(Big::Oint, false),

            "," => Node::Space(0.17),
            ":" | ";" => Node::Space(0.24),
            "!" => Node::Space(-0.17),
            " " => Node::Space(0.30),
            "quad" => Node::Space(1.0),
            "qquad" => Node::Space(2.0),
            "\\" => Node::Empty, // a row break outside a matrix

            _ => {
                if let Some(sym) = symbol(&name) {
                    sym
                } else if name.chars().all(|c| !c.is_ascii_alphabetic()) && !name.is_empty() {
                    // `\{`, `\%`, `\_`: an escaped literal.
                    Node::Sym(name, Class::Ord)
                } else {
                    // Show the author their own typo rather than swallowing it.
                    Node::Sym(format!("\\{name}"), Class::Ord)
                }
            }
        };
        let _ = stop;
        Some(node)
    }

    fn accent(&mut self, accent: Accent) -> Node {
        Node::Accented {
            accent,
            base: Box::new(self.arg()),
        }
    }

    /// The raw text of a braced argument, for `\text{…}`.
    fn literal_arg(&mut self) -> String {
        self.skip_space();
        if !self.eat('{') {
            return self.atom(Stop::End).map(flatten).unwrap_or_default();
        }
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        let s: String = self.s[start..self.i].iter().collect();
        self.eat('}');
        s
    }

    /// The delimiter after `\left` or `\right`. `.` means "no fence".
    fn fence(&mut self) -> Option<char> {
        self.skip_space();
        match self.peek() {
            Some('.') => {
                self.i += 1;
                None
            }
            Some('\\') => {
                self.i += 1;
                let name = self.command();
                match name.as_str() {
                    "{" | "lbrace" => Some('{'),
                    "}" | "rbrace" => Some('}'),
                    "|" | "Vert" => Some('|'),
                    "langle" => Some('⟨'),
                    "rangle" => Some('⟩'),
                    "lceil" => Some('⌈'),
                    "rceil" => Some('⌉'),
                    "lfloor" => Some('⌊'),
                    "rfloor" => Some('⌋'),
                    _ => None,
                }
            }
            Some(c) => {
                self.i += 1;
                Some(c)
            }
            None => None,
        }
    }

    fn matrix(&mut self, env: &str) -> Node {
        let (left, right, align_left) = match env {
            "pmatrix" => (Some('('), Some(')'), false),
            "bmatrix" => (Some('['), Some(']'), false),
            "vmatrix" => (Some('|'), Some('|'), false),
            "Bmatrix" => (Some('{'), Some('}'), false),
            "cases" => (Some('{'), None, true),
            _ => (None, None, false),
        };
        let (column_align, vertical_rules) = if env == "array" {
            self.array_spec()
        } else {
            (Vec::new(), Vec::new())
        };

        let mut rows: Vec<Vec<Node>> = vec![Vec::new()];
        let mut horizontal_rules = Vec::new();
        loop {
            self.skip_space();
            if self.peek().is_none() {
                break;
            }
            // `\end{…}` closes the environment.
            let rest: String = self.s[self.i..].iter().take(4).collect();
            if rest.starts_with("\\end") {
                self.i += 4;
                let _ = self.braced_word();
                break;
            }
            if rest.starts_with("\\\\") {
                self.i += 2;
                rows.push(Vec::new());
                continue;
            }
            let rest: String = self.s[self.i..].iter().take(6).collect();
            if rest.starts_with("\\hline") {
                self.i += 6;
                let boundary = rows.len().saturating_sub(1);
                if !horizontal_rules.contains(&boundary) {
                    horizontal_rules.push(boundary);
                }
                continue;
            }
            if self.eat('&') {
                continue;
            }
            let cell = self.row(Stop::EndEnv);
            rows.last_mut().expect("always one row").push(cell);
            // `row` stopped on `&`, `\\`, `\end` or the end of input; if it
            // made no progress, drop a character so this cannot spin.
            if matches!(self.peek(), Some(c) if c != '&' && c != '\\') {
                self.i += 1;
            }
        }

        if rows.last().is_some_and(|r| r.is_empty()) {
            rows.pop();
        }
        Node::Matrix {
            rows,
            left,
            right,
            align_left,
            column_align,
            vertical_rules,
            horizontal_rules,
        }
    }

    /// Parse the `{l|cr}` immediately following `\begin{array}`. Unsupported
    /// TeX column modifiers stay visible as empty columns are avoided; the
    /// course only needs alignment letters and vertical rules for Routh
    /// tables.
    fn array_spec(&mut self) -> (Vec<ColumnAlign>, Vec<usize>) {
        let spec = self.braced_word();
        let mut columns = Vec::new();
        let mut rules = Vec::new();
        for c in spec.chars().filter(|c| !c.is_whitespace()) {
            match c {
                'l' => columns.push(ColumnAlign::Left),
                'c' => columns.push(ColumnAlign::Center),
                'r' => columns.push(ColumnAlign::Right),
                '|' => {
                    let boundary = columns.len();
                    if !rules.contains(&boundary) {
                        rules.push(boundary);
                    }
                }
                _ => {}
            }
        }
        (columns, rules)
    }
}

fn big(op: Big, limits: bool) -> Node {
    Node::BigOp {
        op,
        limits,
        sub: None,
        sup: None,
    }
}

fn flatten(n: Node) -> String {
    match n {
        Node::Sym(s, _) | Node::Bb(s) => s,
        Node::Row(items) => items.into_iter().map(flatten).collect(),
        _ => String::new(),
    }
}

fn class_of(c: char) -> Class {
    match c {
        '+' | '-' | '−' | '*' | '/' | '±' | '∓' | '·' | '×' | '÷' => Class::Bin,
        '=' | '<' | '>' | '≤' | '≥' | '≈' | '≠' | '≡' | '∼' | '∝' | '→' | '←' | '⇒' | '∈' => {
            Class::Rel
        }
        ',' | ';' | ':' => Class::Punct,
        _ => Class::Ord,
    }
}

/// The symbol tables. Greek, relations, arrows, and the named functions.
fn symbol(name: &str) -> Option<Node> {
    let greek = [
        ("alpha", "α"),
        ("beta", "β"),
        ("gamma", "γ"),
        ("delta", "δ"),
        ("epsilon", "ε"),
        ("varepsilon", "ε"),
        ("zeta", "ζ"),
        ("eta", "η"),
        ("theta", "θ"),
        ("vartheta", "ϑ"),
        ("iota", "ι"),
        ("kappa", "κ"),
        ("lambda", "λ"),
        ("mu", "μ"),
        ("nu", "ν"),
        ("xi", "ξ"),
        ("pi", "π"),
        ("rho", "ρ"),
        ("sigma", "σ"),
        ("tau", "τ"),
        ("upsilon", "υ"),
        ("phi", "φ"),
        ("varphi", "φ"),
        ("chi", "χ"),
        ("psi", "ψ"),
        ("omega", "ω"),
        ("Gamma", "Γ"),
        ("Delta", "Δ"),
        ("Theta", "Θ"),
        ("Lambda", "Λ"),
        ("Xi", "Ξ"),
        ("Pi", "Π"),
        ("Sigma", "Σ"),
        ("Upsilon", "Υ"),
        ("Phi", "Φ"),
        ("Psi", "Ψ"),
        ("Omega", "Ω"),
    ];
    if let Some((_, g)) = greek.iter().find(|(n, _)| *n == name) {
        return Some(Node::Sym((*g).to_string(), Class::Ord));
    }

    let rel = [
        ("le", "≤"),
        ("leq", "≤"),
        ("ge", "≥"),
        ("geq", "≥"),
        ("ne", "≠"),
        ("neq", "≠"),
        ("approx", "≈"),
        ("equiv", "≡"),
        ("sim", "∼"),
        ("propto", "∝"),
        ("ll", "≪"),
        ("gg", "≫"),
        ("in", "∈"),
        ("notin", "∉"),
        ("subset", "⊂"),
        ("to", "→"),
        ("rightarrow", "→"),
        ("longrightarrow", "⟶"),
        ("leftarrow", "←"),
        ("Rightarrow", "⇒"),
        ("Leftrightarrow", "⇔"),
        ("mapsto", "↦"),
    ];
    if let Some((_, g)) = rel.iter().find(|(n, _)| *n == name) {
        return Some(Node::Sym((*g).to_string(), Class::Rel));
    }

    let bin = [
        ("cdot", "·"),
        ("times", "×"),
        ("div", "÷"),
        ("pm", "±"),
        ("mp", "∓"),
        ("ast", "∗"),
        ("circ", "∘"),
        ("cup", "∪"),
        ("cap", "∩"),
    ];
    if let Some((_, g)) = bin.iter().find(|(n, _)| *n == name) {
        return Some(Node::Sym((*g).to_string(), Class::Bin));
    }

    let ord = [
        ("infty", "∞"),
        ("partial", "∂"),
        ("nabla", "∇"),
        ("angle", "∠"),
        ("degree", "°"),
        ("deg", "°"),
        ("ldots", "…"),
        ("dots", "…"),
        ("cdots", "⋯"),
        ("vdots", "⋮"),
        ("prime", "′"),
        ("forall", "∀"),
        ("exists", "∃"),
        ("emptyset", "∅"),
        ("Re", "Re"),
        ("Im", "Im"),
        ("jmath", "j"),
        ("imath", "i"),
    ];
    if let Some((_, g)) = ord.iter().find(|(n, _)| *n == name) {
        return Some(Node::Sym((*g).to_string(), Class::Ord));
    }

    const FUNCS: &[&str] = &[
        "sin", "cos", "tan", "cot", "sec", "csc", "arctan", "arcsin", "arccos", "sinh", "cosh",
        "tanh", "log", "ln", "lg", "exp", "lim", "max", "min", "arg", "det", "dim", "gcd", "sup",
        "inf",
    ];
    if FUNCS.contains(&name) {
        return Some(Node::Sym(name.to_string(), Class::Func));
    }

    None
}

/// Rewrite LaTeX into the closest plain-text spelling: `\omega_0` becomes
/// `ω₀`, `s^2` becomes `s²`.
///
/// Used to decide which symbols a question actually contains. The glossary
/// matches on glyphs, and it must not matter whether the author wrote the
/// letter or its command.
pub fn unicodify(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let name: String = chars[start..j].iter().collect();
            match symbol(&name) {
                Some(Node::Sym(g, _)) => out.push_str(&g),
                _ => {
                    out.push('\\');
                    out.push_str(&name);
                }
            }
            i = j;
            continue;
        }
        // `_0` and `^2`, with or without braces, become the small digits the
        // rest of the content is written with.
        if (c == '_' || c == '^') && i + 1 < chars.len() {
            let (digits, next) = if chars[i + 1] == '{' {
                let mut j = i + 2;
                while j < chars.len() && chars[j] != '}' {
                    j += 1;
                }
                (
                    chars[i + 2..j.min(chars.len())].to_vec(),
                    (j + 1).min(chars.len()),
                )
            } else {
                (vec![chars[i + 1]], i + 2)
            };
            let small: Option<String> = digits
                .iter()
                .map(|d| small_digit(*d, c == '_'))
                .collect::<Option<String>>();
            match small {
                Some(s) => {
                    out.push_str(&s);
                    i = next;
                    continue;
                }
                None => {
                    // Not digits: keep the underscore form, which is how the
                    // packs spell `ω_d` and `K_p` anyway.
                    out.push(c);
                    out.extend(digits.iter());
                    i = next;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn small_digit(d: char, subscript: bool) -> Option<char> {
    const SUB: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
    const SUP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    let n = d.to_digit(10)? as usize;
    Some(if subscript { SUB[n] } else { SUP[n] })
}

// ----------------------------------------------------------------- layout --

/// One drawable piece of a laid-out formula.
#[derive(Debug, Clone)]
enum Item {
    Text {
        /// Top-left of the galley.
        pos: Pos2,
        galley: Arc<Galley>,
    },
    /// A filled rectangle: fraction bars, radical overbars, accent dots.
    Rule(Rect),
    /// An open polyline: radical hooks, hats, hand-drawn operators.
    Poly { points: Vec<Pos2>, width: f32 },
    /// A cubic curve, for the integral sign.
    Curve { points: [Pos2; 4], width: f32 },
}

/// A box with a baseline: everything is positioned relative to the origin at
/// the left end of the baseline, with `y` growing downward as on screen.
#[derive(Debug, Clone, Default)]
struct Bx {
    items: Vec<Item>,
    width: f32,
    /// Ink above the baseline, positive.
    ascent: f32,
    /// Ink below the baseline, positive.
    descent: f32,
}

impl Bx {
    fn height(&self) -> f32 {
        self.ascent + self.descent
    }

    fn shift(mut self, by: Vec2) -> Self {
        for item in &mut self.items {
            match item {
                Item::Text { pos, .. } => *pos += by,
                Item::Rule(r) => *r = r.translate(by),
                Item::Poly { points, .. } => points.iter_mut().for_each(|p| *p += by),
                Item::Curve { points, .. } => points.iter_mut().for_each(|p| *p += by),
            }
        }
        self.ascent -= by.y;
        self.descent += by.y;
        self
    }

    /// Place `other` immediately to the right, sharing the baseline.
    fn append(&mut self, other: Bx, gap: f32) {
        let placed = other.shift(Vec2::new(self.width + gap, 0.0));
        self.ascent = self.ascent.max(placed.ascent);
        self.descent = self.descent.max(placed.descent);
        self.width = placed.width + self.width + gap;
        self.items.extend(placed.items);
    }
}

/// A formula, laid out and ready to paint.
#[derive(Debug, Clone, Default)]
pub struct Formula {
    items: Vec<Item>,
    /// Width, and total height from the top of the ink to the bottom.
    pub size: Vec2,
    /// Distance from the top of the box down to the baseline.
    pub ascent: f32,
}

impl Formula {
    pub fn descent(&self) -> f32 {
        self.size.y - self.ascent
    }

    /// Paint as part of something rotated — the swipe card tilts, and a
    /// formula on it has to tilt rigidly with it.
    pub fn paint_rotated(
        &self,
        painter: &Painter,
        top_left: Pos2,
        pivot: Pos2,
        angle: f32,
        color: Color32,
        opacity: f32,
    ) {
        let at = |p: Pos2| rotate(p + top_left.to_vec2(), pivot, angle);
        let col = color.gamma_multiply(opacity);

        for item in &self.items {
            match item {
                Item::Text { pos, galley } => {
                    painter.add(
                        TextShape::new(at(*pos), galley.clone(), color)
                            .with_override_text_color(color)
                            .with_angle(angle)
                            .with_opacity_factor(opacity),
                    );
                }
                Item::Rule(r) => {
                    let r = r.translate(top_left.to_vec2());
                    painter.add(Shape::convex_polygon(
                        crate::card::corners(r, pivot, angle),
                        col,
                        Stroke::NONE,
                    ));
                }
                Item::Poly { points, width } => {
                    painter.add(Shape::line(
                        points.iter().map(|p| at(*p)).collect(),
                        Stroke::new(*width, col),
                    ));
                }
                Item::Curve { points, width } => {
                    let pts = [at(points[0]), at(points[1]), at(points[2]), at(points[3])];
                    painter.add(CubicBezierShape {
                        points: pts,
                        closed: false,
                        fill: Color32::TRANSPARENT,
                        stroke: PathStroke::new(*width, col),
                    });
                }
            }
        }
    }
}

/// Lay out a LaTeX formula at `size` points.
pub fn layout(painter: &Painter, src: &str, size: f32) -> Formula {
    let node = parse(src);
    let bx = layout_node(painter, &node, size);

    // Move the origin from the baseline to the top-left corner.
    let ascent = bx.ascent;
    let width = bx.width;
    let height = bx.height();
    let placed = bx.shift(Vec2::new(0.0, ascent));
    Formula {
        items: placed.items,
        size: Vec2::new(width, height),
        ascent,
    }
}

/// Thickness of fraction bars and radical strokes.
fn rule_width(size: f32) -> f32 {
    (size * 0.055).max(1.0)
}

/// Height of the fraction bar above the baseline — TeX's "axis", roughly
/// where a minus sign sits.
fn axis(size: f32) -> f32 {
    size * 0.28
}

fn font(size: f32) -> FontId {
    FontId::new(size.max(6.0), FontFamily::Monospace)
}

/// One run of characters, measured by its actual ink so that stacking is
/// tight: font line height would leave a fraction looking hollow.
fn run(painter: &Painter, s: &str, size: f32) -> Bx {
    run_with_ink(painter, s, size).0
}

/// A run, plus the left edge of its ink. Only the double-struck letters need
/// the second half: their extra stroke goes against the stem, and the side
/// bearing between the box and the stem varies with the letter.
fn run_with_ink(painter: &Painter, s: &str, size: f32) -> (Bx, f32) {
    if s.is_empty() {
        return (Bx::default(), 0.0);
    }
    let galley = painter.layout_no_wrap(s.to_owned(), font(size), Color32::WHITE);
    let baseline = galley
        .rows
        .first()
        .map(|r| r.pos.y + r.row.glyphs.first().map_or(size * 0.8, |g| g.pos.y))
        .unwrap_or(size * 0.8);

    let ink = galley.mesh_bounds;
    let (ascent, descent) = if ink.is_positive() {
        (
            (baseline - ink.min.y).max(0.0),
            (ink.max.y - baseline).max(0.0),
        )
    } else {
        // Whitespace has no ink; give it the advance width and no height.
        (0.0, 0.0)
    };

    (
        Bx {
            items: vec![Item::Text {
                pos: Pos2::new(0.0, -baseline),
                galley: galley.clone(),
            }],
            width: galley.rect.width(),
            ascent,
            descent,
        },
        if ink.is_positive() { ink.min.x } else { 0.0 },
    )
}

fn layout_node(painter: &Painter, node: &Node, size: f32) -> Bx {
    match node {
        Node::Empty => Bx::default(),
        Node::Space(k) => Bx {
            width: k * size,
            ..Bx::default()
        },
        Node::Sym(s, class) => {
            let mut bx = run(painter, s, size);
            if *class == Class::Func {
                // A function name is upright text with a thin space after it.
                bx.width += size * 0.17;
            }
            bx
        }
        Node::Bb(s) => layout_bb(painter, s, size),
        Node::Row(items) => layout_row(painter, items, size),
        Node::Frac(num, den) => layout_frac(painter, num, den, size),
        Node::Sqrt { index, body } => layout_sqrt(painter, index.as_deref(), body, size),
        Node::Script { base, sup, sub } => {
            layout_script(painter, base, sup.as_deref(), sub.as_deref(), size)
        }
        Node::Fenced { left, right, body } => {
            let inner = layout_node(painter, body, size);
            layout_fenced(painter, *left, *right, inner, size)
        }
        Node::Accented { accent, base } => layout_accent(painter, *accent, base, size),
        Node::BigOp {
            op,
            limits,
            sub,
            sup,
        } => layout_bigop(painter, *op, *limits, sub.as_deref(), sup.as_deref(), size),
        Node::Matrix {
            rows,
            left,
            right,
            align_left,
            column_align,
            vertical_rules,
            horizontal_rules,
        } => layout_matrix(
            painter,
            rows,
            *left,
            *right,
            *align_left,
            column_align,
            vertical_rules,
            horizontal_rules,
            size,
        ),
    }
}

/// The class of a node, for spacing decisions.
fn class_of_node(n: &Node) -> Class {
    match n {
        Node::Sym(_, c) => *c,
        Node::Row(items) => items.first().map_or(Class::Ord, class_of_node),
        _ => Class::Ord,
    }
}

/// A double-struck letter: the ordinary letter with its stem struck twice.
///
/// Not the Unicode codepoints. `ℝ` (U+211D) is missing from the monospaced
/// face the rest of the maths is set in, and a tofu box in the domain of a
/// function is worse than no `\mathbb` at all — the same reason `∑` and `∫`
/// are drawn here rather than looked up. The extra stroke sits to the left of
/// the letter, which is where it goes when the letter is written by hand.
fn layout_bb(painter: &Painter, s: &str, size: f32) -> Bx {
    let rule = rule_width(size);
    let mut out = Bx::default();

    for c in s.chars() {
        let (letter, ink_left) = run_with_ink(painter, &c.to_string(), size);
        let mut glyph = if c.is_alphanumeric() {
            // The stroke all but touches the stem: any further off and it
            // reads as a pipe character standing next to the letter.
            let hair = rule * 0.4;
            let dx = (rule + hair - ink_left).max(0.0);
            let mut g = letter.clone().shift(Vec2::new(dx, 0.0));
            let height = letter.ascent.max(size * 0.5);
            g.items.push(Item::Rule(Rect::from_min_size(
                Pos2::new(dx + ink_left - hair - rule, -height),
                Vec2::new(rule, height),
            )));
            g.width = letter.width + dx;
            g.ascent = g.ascent.max(height);
            g
        } else {
            letter
        };
        if out.width == 0.0 && out.items.is_empty() {
            out = std::mem::take(&mut glyph);
        } else {
            out.append(glyph, 0.0);
        }
    }
    out
}

fn layout_row(painter: &Painter, items: &[Node], size: f32) -> Bx {
    let mut out = Bx::default();
    let mut prev: Option<Class> = None;

    for node in items {
        let mut class = class_of_node(node);
        // A binary operator with nothing to bind on its left is a sign, not an
        // operator: `-x` must not be spaced like `a - x`.
        if class == Class::Bin
            && matches!(
                prev,
                None | Some(Class::Bin) | Some(Class::Rel) | Some(Class::Punct)
            )
        {
            class = Class::Ord;
        }

        let gap = match (prev, class) {
            (None, _) => 0.0,
            (_, Class::Rel) | (Some(Class::Rel), _) => size * 0.24,
            (_, Class::Bin) | (Some(Class::Bin), _) => size * 0.18,
            (Some(Class::Punct), _) => size * 0.16,
            _ => 0.0,
        };

        let bx = layout_node(painter, node, size);
        if out.items.is_empty() && out.width == 0.0 {
            out = bx;
        } else {
            out.append(bx, gap);
        }
        prev = Some(class);
    }
    out
}

fn layout_frac(painter: &Painter, num: &Node, den: &Node, size: f32) -> Bx {
    // Inline prose already sets math only a touch larger than its text.
    // Shrinking a fraction again made the numerator and denominator needlessly
    // small, especially on dense explanation panels. Keep them at the
    // surrounding math size; global UI zoom remains the user's larger lever.
    let inner = size.max(9.0);
    let n = layout_node(painter, num, inner);
    let d = layout_node(painter, den, inner);

    let rule = rule_width(size);
    let ax = axis(size);
    let gap = size * 0.20;
    let pad = size * 0.22;

    let width = n.width.max(d.width) + 2.0 * pad;
    let bar_y = -ax - rule / 2.0;

    let n_placed = n
        .clone()
        .shift(Vec2::new((width - n.width) / 2.0, bar_y - gap - n.descent));
    let d_placed = d.clone().shift(Vec2::new(
        (width - d.width) / 2.0,
        -ax + rule / 2.0 + gap + d.ascent,
    ));

    let mut items = vec![Item::Rule(Rect::from_min_size(
        Pos2::new(0.0, bar_y),
        Vec2::new(width, rule),
    ))];
    items.extend(n_placed.items);
    items.extend(d_placed.items);

    Bx {
        items,
        width,
        ascent: n_placed.ascent.max(ax + rule),
        descent: d_placed.descent.max(0.0),
    }
}

fn layout_sqrt(painter: &Painter, index: Option<&Node>, body: &Node, size: f32) -> Bx {
    let inner = layout_node(painter, body, size);
    let rule = rule_width(size);
    let pad = size * 0.14;
    let hook_w = size * 0.55;

    // The radical spans the content plus a gap above it for the overbar.
    let top = -(inner.ascent + pad + rule);
    let bottom = inner.descent.max(size * 0.1);
    let h = bottom - top;
    let content_x = hook_w + pad;
    let width = content_x + inner.width + pad;

    let mut items = vec![
        Item::Poly {
            points: vec![
                Pos2::new(0.0, top + h * 0.58),
                Pos2::new(hook_w * 0.30, top + h * 0.50),
                Pos2::new(hook_w * 0.62, bottom),
                Pos2::new(hook_w, top + rule / 2.0),
            ],
            width: rule,
        },
        Item::Rule(Rect::from_min_size(
            Pos2::new(hook_w - rule / 2.0, top),
            Vec2::new(width - hook_w + rule / 2.0, rule),
        )),
    ];

    let mut ascent = -top;
    if let Some(idx) = index {
        let ix = layout_node(painter, idx, (size * 0.55).max(7.0));
        let placed = ix
            .clone()
            .shift(Vec2::new(0.0, top + h * 0.45 - ix.descent));
        ascent = ascent.max(-(top + h * 0.45 - ix.height()));
        items.extend(placed.items);
    }

    items.extend(inner.clone().shift(Vec2::new(content_x, 0.0)).items);

    Bx {
        items,
        width,
        ascent,
        descent: bottom,
    }
}

fn layout_script(
    painter: &Painter,
    base: &Node,
    sup: Option<&Node>,
    sub: Option<&Node>,
    size: f32,
) -> Bx {
    let mut out = layout_node(painter, base, size);
    let small = (size * 0.70).max(7.5);
    let x = out.width + size * 0.04;
    let mut width = out.width;

    if let Some(sup) = sup {
        let s = layout_node(painter, sup, small);
        // Raised so its foot clears the base's x-height, never dipping into it.
        let shift = -(out.ascent.max(size * 0.5) * 0.62 + s.descent);
        let placed = s.shift(Vec2::new(x, shift));
        width = width.max(x + placed.width);
        out.ascent = out.ascent.max(placed.ascent);
        out.items.extend(placed.items);
    }
    if let Some(sub) = sub {
        let s = layout_node(painter, sub, small);
        let shift = size * 0.22 + s.ascent * 0.30;
        let placed = s.shift(Vec2::new(x, shift));
        width = width.max(x + placed.width);
        out.descent = out.descent.max(placed.descent);
        out.items.extend(placed.items);
    }

    out.width = width + size * 0.04;
    out
}

/// Grow a delimiter to the height it has to span.
///
/// Scaling the glyph rather than assembling one from pieces: in a monospaced
/// face the fences are simple shapes, and a stretched `(` reads correctly
/// where a stack of box-drawing characters would not.
fn fence_box(painter: &Painter, delim: char, needed: f32, size: f32) -> Bx {
    if delim == '|' {
        let rule = rule_width(size);
        let half = needed / 2.0;
        return Bx {
            items: vec![Item::Rule(Rect::from_min_size(
                Pos2::new(
                    size * 0.12,
                    -half - axis(size) + needed / 2.0 - needed / 2.0,
                ),
                Vec2::new(rule, needed),
            ))],
            width: size * 0.30,
            ascent: half + axis(size),
            descent: (needed - half - axis(size)).max(0.0),
        };
    }

    let natural = run(painter, &delim.to_string(), size);
    let natural_h = natural.height().max(size * 0.5);
    let scale = (needed / natural_h).clamp(1.0, 4.0);
    let mut bx = run(painter, &delim.to_string(), size * scale);

    // Centre the fence on the axis, which is where a fraction bar sits and
    // where the eye expects a bracket to be centred.
    let centre = -axis(size);
    let current = (bx.descent - bx.ascent) / 2.0;
    bx = bx.shift(Vec2::new(0.0, centre - current));
    bx
}

fn layout_fenced(
    painter: &Painter,
    left: Option<char>,
    right: Option<char>,
    inner: Bx,
    size: f32,
) -> Bx {
    let needed = (inner.height() + size * 0.18).max(size);
    let mut out = Bx::default();

    if let Some(l) = left {
        out = fence_box(painter, l, needed, size);
    }
    if out.width == 0.0 {
        out = inner;
    } else {
        out.append(inner, size * 0.06);
    }
    if let Some(r) = right {
        let rb = fence_box(painter, r, needed, size);
        out.append(rb, size * 0.06);
    }
    out
}

fn layout_accent(painter: &Painter, accent: Accent, base: &Node, size: f32) -> Bx {
    let mut out = layout_node(painter, base, size);
    let w = out.width.max(size * 0.4);
    let rule = rule_width(size);
    let gap = size * 0.10;
    // Accents ride at a fixed height, so `ẋ` and `Ẋ` in one formula line up.
    let top = -(out.ascent.max(size * 0.52) + gap);
    let cx = w / 2.0;

    let dot = |x: f32, y: f32| {
        Item::Rule(Rect::from_center_size(
            Pos2::new(x, y),
            Vec2::splat(rule * 1.8),
        ))
    };

    let (items, extra): (Vec<Item>, f32) = match accent {
        Accent::Dot => (vec![dot(cx, top - rule)], rule * 3.0),
        Accent::DDot => (
            vec![
                dot(cx - size * 0.13, top - rule),
                dot(cx + size * 0.13, top - rule),
            ],
            rule * 3.0,
        ),
        Accent::Bar => (
            vec![Item::Rule(Rect::from_min_size(
                Pos2::new(0.0, top - rule),
                Vec2::new(w, rule),
            ))],
            rule * 2.0,
        ),
        Accent::Hat => (
            vec![Item::Poly {
                points: vec![
                    Pos2::new(cx - size * 0.18, top),
                    Pos2::new(cx, top - size * 0.16),
                    Pos2::new(cx + size * 0.18, top),
                ],
                width: rule,
            }],
            size * 0.16,
        ),
        Accent::Tilde => (
            vec![Item::Poly {
                points: vec![
                    Pos2::new(cx - size * 0.20, top - size * 0.02),
                    Pos2::new(cx - size * 0.07, top - size * 0.12),
                    Pos2::new(cx + size * 0.07, top - size * 0.02),
                    Pos2::new(cx + size * 0.20, top - size * 0.12),
                ],
                width: rule,
            }],
            size * 0.14,
        ),
        Accent::Vec => (
            vec![
                Item::Poly {
                    points: vec![
                        Pos2::new(cx - size * 0.20, top - size * 0.06),
                        Pos2::new(cx + size * 0.20, top - size * 0.06),
                    ],
                    width: rule,
                },
                Item::Poly {
                    points: vec![
                        Pos2::new(cx + size * 0.10, top - size * 0.14),
                        Pos2::new(cx + size * 0.20, top - size * 0.06),
                        Pos2::new(cx + size * 0.10, top + size * 0.02),
                    ],
                    width: rule,
                },
            ],
            size * 0.14,
        ),
    };

    out.ascent = (-top) + extra;
    out.items.extend(items);
    out
}

/// The big operators, drawn rather than set: `∑` and `∫` are missing from
/// plenty of monospaced faces, and a hole in a formula is worse than a shape
/// that is a pixel off.
fn bigop_shape(op: Big, size: f32) -> Bx {
    let h = size * 1.45;
    let w = size * 0.95;
    let rule = (size * 0.075).max(1.2);
    let top = -h * 0.72;
    let bottom = h * 0.28;
    // `∬` is two signs sharing a shoulder, not two integrals side by side.
    let signs = match op {
        Big::Int(n) => n.clamp(1, 3) as usize,
        _ => 1,
    };
    let kern = w * 0.46;

    let items = match op {
        Big::Sum => vec![Item::Poly {
            points: vec![
                Pos2::new(w, top + h * 0.16),
                Pos2::new(w, top),
                Pos2::new(0.0, top),
                Pos2::new(w * 0.62, (top + bottom) / 2.0),
                Pos2::new(0.0, bottom),
                Pos2::new(w, bottom),
                Pos2::new(w, bottom - h * 0.16),
            ],
            width: rule,
        }],
        Big::Prod => vec![
            Item::Poly {
                points: vec![Pos2::new(0.0, top), Pos2::new(w, top)],
                width: rule,
            },
            Item::Poly {
                points: vec![Pos2::new(w * 0.22, top), Pos2::new(w * 0.22, bottom)],
                width: rule,
            },
            Item::Poly {
                points: vec![Pos2::new(w * 0.78, top), Pos2::new(w * 0.78, bottom)],
                width: rule,
            },
        ],
        Big::Int(_) | Big::Oint => {
            let mut v: Vec<Item> = (0..signs)
                .map(|i| {
                    let dx = i as f32 * kern;
                    Item::Curve {
                        points: [
                            Pos2::new(dx + w * 0.05, bottom + h * 0.06),
                            Pos2::new(dx + w * 0.72, bottom - h * 0.10),
                            Pos2::new(dx + w * 0.08, top + h * 0.10),
                            Pos2::new(dx + w * 0.75, top - h * 0.06),
                        ],
                        width: rule,
                    }
                })
                .collect();
            if op == Big::Oint {
                v.push(Item::Poly {
                    points: circle_points(Pos2::new(w * 0.40, (top + bottom) / 2.0), size * 0.26),
                    width: rule * 0.8,
                });
            }
            v
        }
    };

    Bx {
        items,
        width: w + kern * (signs - 1) as f32 + size * 0.12,
        ascent: -top,
        descent: bottom,
    }
}

fn circle_points(centre: Pos2, r: f32) -> Vec<Pos2> {
    (0..=16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            centre + Vec2::new(r * a.cos(), r * a.sin())
        })
        .collect()
}

fn layout_bigop(
    painter: &Painter,
    op: Big,
    limits: bool,
    sub: Option<&Node>,
    sup: Option<&Node>,
    size: f32,
) -> Bx {
    let glyph = bigop_shape(op, size);
    let small = (size * 0.62).max(7.5);

    if !limits {
        // An integral carries its bounds at the corners, as it is set inline.
        let mut out = glyph;
        let x = out.width;
        let mut width = out.width;
        if let Some(sup) = sup {
            let s = layout_node(painter, sup, small).shift(Vec2::new(x, -out.ascent * 0.85));
            width = width.max(x + s.width);
            out.ascent = out.ascent.max(s.ascent);
            out.items.extend(s.items);
        }
        if let Some(sub) = sub {
            let s = layout_node(painter, sub, small).shift(Vec2::new(x, out.descent * 0.9));
            width = width.max(x + s.width);
            out.descent = out.descent.max(s.descent);
            out.items.extend(s.items);
        }
        out.width = width + size * 0.10;
        return out;
    }

    let up = sup.map(|n| layout_node(painter, n, small));
    let down = sub.map(|n| layout_node(painter, n, small));
    let width = glyph
        .width
        .max(up.as_ref().map_or(0.0, |b| b.width))
        .max(down.as_ref().map_or(0.0, |b| b.width));

    let gap = size * 0.14;
    let mut out = glyph
        .clone()
        .shift(Vec2::new((width - glyph.width) / 2.0, 0.0));
    out.width = width;

    if let Some(u) = up {
        let dy = -glyph.ascent - gap - u.descent;
        let placed = u.clone().shift(Vec2::new((width - u.width) / 2.0, dy));
        out.ascent = out.ascent.max(placed.ascent);
        out.items.extend(placed.items);
    }
    if let Some(d) = down {
        let dy = glyph.descent + gap + d.ascent;
        let placed = d.clone().shift(Vec2::new((width - d.width) / 2.0, dy));
        out.descent = out.descent.max(placed.descent);
        out.items.extend(placed.items);
    }
    out.width += size * 0.12;
    out
}

fn layout_matrix(
    painter: &Painter,
    rows: &[Vec<Node>],
    left: Option<char>,
    right: Option<char>,
    align_left: bool,
    column_align: &[ColumnAlign],
    vertical_rules: &[usize],
    horizontal_rules: &[usize],
    size: f32,
) -> Bx {
    let cells: Vec<Vec<Bx>> = rows
        .iter()
        .map(|r| r.iter().map(|c| layout_node(painter, c, size)).collect())
        .collect();

    let cols = cells.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_w = vec![0.0f32; cols];
    for row in &cells {
        for (i, c) in row.iter().enumerate() {
            col_w[i] = col_w[i].max(c.width);
        }
    }

    let col_gap = size * 0.7;
    let row_gap = size * 0.45;
    let total_w: f32 = col_w.iter().sum::<f32>() + col_gap * (cols.saturating_sub(1)) as f32;

    // Stack the rows, then centre the whole grid on the axis.
    let mut items = Vec::new();
    let mut y = 0.0f32;
    let mut row_boundaries = vec![0.0];
    for row in &cells {
        let asc = row.iter().map(|c| c.ascent).fold(size * 0.5, f32::max);
        let des = row.iter().map(|c| c.descent).fold(size * 0.2, f32::max);
        let mut x = 0.0f32;
        for (i, c) in row.iter().enumerate() {
            let alignment = column_align.get(i).copied().unwrap_or(if align_left {
                ColumnAlign::Left
            } else {
                ColumnAlign::Center
            });
            let dx = match alignment {
                ColumnAlign::Left => 0.0,
                ColumnAlign::Center => (col_w[i] - c.width) / 2.0,
                ColumnAlign::Right => col_w[i] - c.width,
            };
            items.extend(c.clone().shift(Vec2::new(x + dx, y + asc)).items);
            x += col_w[i] + col_gap;
        }
        y += asc + des + row_gap;
        row_boundaries.push(y - row_gap / 2.0);
    }
    let total_h = (y - row_gap).max(0.0);
    if let Some(last) = row_boundaries.last_mut() {
        *last = total_h;
    }

    let rule = rule_width(size);
    for &boundary in vertical_rules {
        if boundary > cols {
            continue;
        }
        let x = if boundary == 0 {
            0.0
        } else if boundary == cols {
            total_w
        } else {
            col_w[..boundary].iter().sum::<f32>() + col_gap * (boundary as f32 - 0.5)
        };
        items.push(Item::Rule(Rect::from_min_size(
            Pos2::new(x - rule / 2.0, 0.0),
            Vec2::new(rule, total_h),
        )));
    }
    for &boundary in horizontal_rules {
        let Some(&y) = row_boundaries.get(boundary) else {
            continue;
        };
        items.push(Item::Rule(Rect::from_min_size(
            Pos2::new(0.0, y - rule / 2.0),
            Vec2::new(total_w, rule),
        )));
    }

    let mut grid = Bx {
        items,
        width: total_w,
        ascent: 0.0,
        descent: total_h,
    };
    // Shift so the grid straddles the axis.
    grid = grid.shift(Vec2::new(0.0, -total_h / 2.0 - axis(size)));
    grid.ascent = total_h / 2.0 + axis(size);
    grid.descent = total_h / 2.0 - axis(size);

    layout_fenced(painter, left, right, grid, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> Node {
        Node::Sym(s.into(), Class::Ord)
    }

    #[test]
    fn plain_text_is_a_row_of_atoms() {
        assert_eq!(parse("ab"), Node::Row(vec![sym("a"), sym("b")]));
    }

    #[test]
    fn digits_clump_so_a_script_lands_on_the_whole_number() {
        assert_eq!(
            parse("10^3"),
            Node::Script {
                base: Box::new(sym("10")),
                sup: Some(Box::new(sym("3"))),
                sub: None,
            }
        );
    }

    #[test]
    fn scripts_attach_to_the_last_atom_only() {
        let Node::Row(items) = parse("xy^2") else {
            panic!("expected a row")
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], sym("x"));
        assert!(matches!(items[1], Node::Script { .. }));
    }

    #[test]
    fn both_scripts_attach_in_either_order() {
        let a = parse("x^2_i");
        let b = parse("x_i^2");
        assert_eq!(a, b, "TeX does not care which came first");
        let Node::Script { sup, sub, .. } = a else {
            panic!("expected scripts")
        };
        assert_eq!(sup, Some(Box::new(sym("2"))));
        assert_eq!(sub, Some(Box::new(sym("i"))));
    }

    #[test]
    fn a_fraction_takes_two_arguments_braced_or_not() {
        assert_eq!(
            parse(r"\frac12"),
            Node::Frac(Box::new(sym("1")), Box::new(sym("2")))
        );
        let braced = parse(r"\frac{1+s}{2}");
        let Node::Frac(num, _) = braced else {
            panic!("expected a fraction")
        };
        assert!(matches!(*num, Node::Row(_)));
    }

    #[test]
    fn greek_and_relations_come_out_as_glyphs() {
        assert_eq!(parse(r"\zeta"), Node::Sym("ζ".into(), Class::Ord));
        assert_eq!(parse(r"\approx"), Node::Sym("≈".into(), Class::Rel));
        assert_eq!(parse(r"\cdot"), Node::Sym("·".into(), Class::Bin));
    }

    #[test]
    fn an_unknown_command_shows_itself_rather_than_vanishing() {
        assert_eq!(parse(r"\nosuchthing"), sym("\\nosuchthing"));
    }

    #[test]
    fn fences_carry_their_delimiters() {
        let n = parse(r"\left( \frac{a}{b} \right)");
        let Node::Fenced { left, right, .. } = n else {
            panic!("expected a fence")
        };
        assert_eq!((left, right), (Some('('), Some(')')));
    }

    #[test]
    fn a_missing_closing_fence_does_not_hang() {
        // Malformed content must render *something*, not spin or panic.
        let n = parse(r"\left( x");
        assert!(matches!(n, Node::Fenced { .. }));
        assert!(matches!(parse(r"\right)"), Node::Empty));
        assert!(matches!(parse(r"\frac{"), Node::Frac(..)));
    }

    #[test]
    fn big_operators_swallow_their_limits() {
        let n = parse(r"\sum_{k=0}^{n} k");
        let Node::Row(items) = n else {
            panic!("expected a row")
        };
        let Node::BigOp {
            op,
            sub,
            sup,
            limits,
        } = &items[0]
        else {
            panic!("expected a big operator")
        };
        assert_eq!(*op, Big::Sum);
        assert!(limits);
        assert!(sub.is_some() && sup.is_some());
    }

    #[test]
    fn an_integral_keeps_its_bounds_at_the_corners() {
        let Node::BigOp { op, limits, .. } = parse(r"\int_0^\infty") else {
            panic!("expected an integral")
        };
        assert_eq!(op, Big::Int(1));
        assert!(!limits, "an integral is set inline, not with limits");
    }

    #[test]
    fn multiple_integrals_are_one_operator_with_several_signs() {
        for (src, signs) in [(r"\int", 1), (r"\iint", 2), (r"\iiint", 3)] {
            let Node::BigOp { op, .. } = parse(src) else {
                panic!("expected an integral for {src}")
            };
            assert_eq!(op, Big::Int(signs));
        }
    }

    #[test]
    fn blackboard_bold_is_its_own_node_rather_than_a_plain_letter() {
        assert_eq!(parse(r"\mathbb{R}"), Node::Bb("R".into()));
        assert_eq!(
            parse(r"\mathbb{R}^n"),
            Node::Script {
                base: Box::new(Node::Bb("R".into())),
                sup: Some(Box::new(sym("n"))),
                sub: None,
            }
        );
        assert_eq!(
            parse(r"\mathbb{R}^2 \to \mathbb{R}"),
            Node::Row(vec![
                Node::Script {
                    base: Box::new(Node::Bb("R".into())),
                    sup: Some(Box::new(sym("2"))),
                    sub: None,
                },
                Node::Sym("→".into(), Class::Rel),
                Node::Bb("R".into()),
            ])
        );
    }

    #[test]
    fn a_matrix_splits_on_ampersands_and_row_breaks() {
        let n = parse(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
        let Node::Matrix { rows, left, .. } = n else {
            panic!("expected a matrix")
        };
        assert_eq!(left, Some('('));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 2);
        assert_eq!(rows[1][1], sym("d"));
    }

    #[test]
    fn an_array_keeps_column_alignment_and_rules() {
        let n = parse(r"\begin{array}{r|cc} s^3 & 1 & 3 \\ \hline s^2 & 2 & 4 \end{array}");
        let Node::Matrix {
            rows,
            column_align,
            vertical_rules,
            horizontal_rules,
            ..
        } = n
        else {
            panic!("expected an array")
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(
            column_align,
            vec![ColumnAlign::Right, ColumnAlign::Center, ColumnAlign::Center]
        );
        assert_eq!(vertical_rules, vec![1]);
        assert_eq!(horizontal_rules, vec![1]);
    }

    #[test]
    fn text_keeps_its_spaces() {
        assert_eq!(
            parse(r"\text{rise time}"),
            Node::Sym("rise time".into(), Class::Ord)
        );
        assert_eq!(
            parse(r"\mathcal{L}"),
            Node::Sym("L".into(), Class::Ord),
            "unsupported font variants should still render their contents"
        );
    }

    #[test]
    fn accents_wrap_their_base() {
        assert_eq!(
            parse(r"\dot{x}"),
            Node::Accented {
                accent: Accent::Dot,
                base: Box::new(sym("x")),
            }
        );
    }

    #[test]
    fn latex_flattens_to_the_glyphs_the_packs_are_written_with() {
        assert_eq!(unicodify(r"\zeta"), "ζ");
        assert_eq!(unicodify(r"\omega_0"), "ω₀");
        assert_eq!(unicodify(r"2\zeta\omega_0 s + s^{2}"), "2ζω₀ s + s²");
        assert_eq!(unicodify(r"\omega_d"), "ω_d", "non-digits keep their form");
        assert_eq!(unicodify(r"\notasymbol"), r"\notasymbol");
        assert_eq!(unicodify("plain text"), "plain text");
    }

    #[test]
    fn an_empty_formula_is_not_an_error() {
        assert_eq!(parse(""), Node::Empty);
        assert_eq!(parse("   "), Node::Empty);
    }

    #[test]
    fn a_unary_minus_is_not_spaced_like_an_operator() {
        // Not observable from the tree, but the classes that drive spacing are.
        assert_eq!(class_of('-'), Class::Bin);
        let Node::Row(items) = parse("-x+y") else {
            panic!("expected a row")
        };
        assert_eq!(class_of_node(&items[0]), Class::Bin);
    }
}
