//! Screens and interaction.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use idiosepius_core::model::{Deck, Topic};
use idiosepius_core::session::{Event, Outcome};
use idiosepius_core::{
    Body, Grade, Input, Mode, Question, Response, Session, Store, now_ms, scheduler, stats,
};

use crate::card::{self, Motion};
use crate::coin::CoinAnimation;
use crate::theme::{Palette, text, tracked};

/// How long a correct answer's feedback stays up before dealing the next card.
const AUTO_ADVANCE: f32 = 0.75;

pub struct App {
    store: Rc<Store>,
    screen: Screen,
    decks: Vec<Deck>,
    error: Option<String>,
    coin: CoinAnimation,
    /// Development aid: render a few frames, save a PNG, quit. Lets the UI be
    /// checked headlessly (under Xvfb, in CI) instead of by eye.
    shot: Option<Shot>,
}

/// A pending `--shot` capture.
pub struct Shot {
    pub path: std::path::PathBuf,
    /// Which screen to capture; `None` means whatever is on screen.
    pub screen: Option<String>,
    /// Force a particular question, by `uid`, so a capture is reproducible.
    pub card: Option<String>,
    /// Pre-drag the card by this many points, to capture a swipe in progress.
    pub drag: f32,
    frames: u32,
    requested: bool,
}

impl Shot {
    pub fn new(path: std::path::PathBuf, screen: Option<String>) -> Self {
        Shot {
            path,
            screen,
            card: None,
            drag: 0.0,
            frames: 0,
            requested: false,
        }
    }

    pub fn with_card(mut self, card: Option<String>) -> Self {
        self.card = card;
        self
    }

    pub fn with_drag(mut self, drag: f32) -> Self {
        self.drag = drag;
        self
    }
}

enum Screen {
    Decks,
    Study(Box<Study>),
    Summary(Summary),
}

struct Study {
    session: Session,
    deck: Deck,
    topics: HashMap<i64, String>,
    current: Option<Question>,
    motion: Motion,
    /// Tail of question ids already shown, for interleaving.
    recent: Vec<i64>,
    /// Multiple-choice selection for the current card.
    selected: Vec<usize>,
    feedback: Option<Feedback>,
    answered: u32,
    correct: u32,
    counts: scheduler::Counts,
    /// Where the pointer grabbed the card, in card-local coordinates.
    grab: Option<Vec2>,
}

struct Feedback {
    question: Question,
    grade: Grade,
    response: Response,
    since: Instant,
    outcome: Outcome,
}

struct Summary {
    session_id: i64,
    deck_id: i64,
    stats: stats::SessionStats,
    weakest: Vec<stats::WeakQuestion>,
}

impl App {
    pub fn new(ctx: &egui::Context, store: Store, shot: Option<Shot>) -> Self {
        crate::theme::install(ctx);
        let store = Rc::new(store);
        let decks = store.decks().unwrap_or_default();
        let animate_coin = shot.is_none();
        let mut app = App {
            store,
            screen: Screen::Decks,
            decks,
            error: None,
            coin: CoinAnimation::new(animate_coin),
            shot,
        };

        // Jump straight to the screen being captured.
        if let Some(target) = app.shot.as_ref().and_then(|s| s.screen.clone())
            && target != "decks"
            && let Some(deck) = app.decks.first().cloned()
            && let Some(mut screen) = app.begin(deck, Mode::Practice)
        {
            app.stage_shot(&mut screen);
            app.screen = screen;
        }
        app
    }

    /// Put the captured screen into a fixed, reproducible state.
    fn stage_shot(&mut self, screen: &mut Screen) {
        let Some(shot) = &self.shot else { return };
        let Screen::Study(study) = screen else { return };

        if let Some(uid) = &shot.card {
            let found = self
                .store
                .questions(study.deck.id)
                .unwrap_or_default()
                .into_iter()
                .find(|q| &q.uid == uid);
            match found {
                Some(q) => {
                    study.session.show(q.id);
                    study.current = Some(q);
                }
                None => eprintln!("no question with uid {uid:?}"),
            }
        }

        if shot.drag != 0.0 {
            study.motion.entry = 1.0;
            study.motion.dragging = true; // freeze it: no spring-back mid-capture
            study.motion.offset = Vec2::new(shot.drag, shot.drag * 0.08);
        }
    }

    /// Drive the capture: settle for a few frames, ask for the framebuffer,
    /// write it out, then close the window.
    fn drive_shot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        ctx.request_repaint();
        shot.frames += 1;

        // Fonts and the entry animation need a moment to settle.
        if shot.frames == 12 && !shot.requested {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }

        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });

        if let Some(image) = image {
            let path = shot.path.clone();
            let px: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|p| [p.r(), p.g(), p.b(), p.a()])
                .collect();
            let header = format!(
                "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n",
                image.width(),
                image.height()
            );
            let mut out = header.into_bytes();
            out.extend_from_slice(&px);
            if let Err(e) = std::fs::write(&path, out) {
                eprintln!("could not write {}: {e}", path.display());
            } else {
                eprintln!(
                    "wrote {} ({}x{})",
                    path.display(),
                    image.width(),
                    image.height()
                );
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The root ui has no background of its own.
        ui.painter().rect_filled(ui.max_rect(), 0, Palette::BG);

        // Move the screen out of `self` for the duration of the frame. The
        // screen owns a `Session`, which needs `&mut`, while the handlers also
        // need `&mut self` for the store and error slot; splitting them is
        // simpler than threading a borrow through every call.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Decks);

        let next = match &mut screen {
            Screen::Decks => self.deck_screen(ui),
            Screen::Study(study) => self.study_screen(ui, study),
            Screen::Summary(sum) => self.summary_screen(ui, sum),
        };
        self.screen = next.unwrap_or(screen);

        self.error_bar(ui);
        self.drive_shot(ui.ctx());
    }
}

impl App {
    /// A dismissible strip along the bottom. Errors here are things the user
    /// can do nothing about mid-session, so they must not interrupt studying.
    fn error_bar(&mut self, ui: &mut egui::Ui) {
        let Some(msg) = self.error.clone() else {
            return;
        };
        let full = ui.max_rect();
        let bar = Rect::from_min_size(
            full.left_bottom() - Vec2::new(0.0, 34.0),
            Vec2::new(full.width(), 34.0),
        );
        let resp = ui.interact(bar, Id::new("error-bar"), Sense::click());

        let p = ui.painter();
        p.rect_filled(bar, 0, Palette::WRONG.gamma_multiply(0.16));
        p.line_segment(
            [bar.left_top(), bar.right_top()],
            Stroke::new(1.0, Palette::WRONG),
        );
        p.text(
            bar.left_center() + Vec2::new(16.0, 0.0),
            Align2::LEFT_CENTER,
            msg,
            text::small(),
            Palette::TEXT,
        );
        p.text(
            bar.right_center() - Vec2::new(16.0, 0.0),
            Align2::RIGHT_CENTER,
            tracked("click to dismiss"),
            text::label(),
            Palette::TEXT_DIM,
        );
        if resp.clicked() {
            self.error = None;
        }
    }
}

// ------------------------------------------------------------ deck screen --

impl App {
    fn deck_screen(&mut self, ui: &mut egui::Ui) -> Option<Screen> {
        let mut next = None;
        let avail = ui.available_rect_before_wrap();

        let panel = Rect::from_center_size(
            avail.center(),
            Vec2::new(avail.width().min(660.0), avail.height().min(560.0)),
        );

        let p = ui.painter();
        p.text(
            panel.left_top() + Vec2::new(0.0, 6.0),
            Align2::LEFT_TOP,
            "IDIOSEPIUS",
            text::title(),
            Palette::ACCENT,
        );
        p.text(
            panel.left_top() + Vec2::new(0.0, 36.0),
            Align2::LEFT_TOP,
            tracked("pygmy squid"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        let coin_rect = Rect::from_center_size(
            panel.right_top() + Vec2::new(-38.0, 34.0),
            Vec2::splat(68.0),
        );
        let coin_response = ui.interact(coin_rect, Id::new("brand-coin"), Sense::click());
        if coin_response.clicked() {
            self.coin.spin();
        }
        self.coin.paint(ui, coin_rect);

        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 62.0),
                panel.right_top() + Vec2::new(0.0, 62.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        let mut y = panel.top() + 84.0;
        let decks = self.decks.clone();

        if decks.is_empty() {
            ui.painter().text(
                Pos2::new(panel.left(), y),
                Align2::LEFT_TOP,
                "No decks yet. Import one:\n\n  idiodb study.db import content/*.json",
                text::body(),
                Palette::TEXT_DIM,
            );
            return None;
        }

        for deck in &decks {
            let row =
                Rect::from_min_size(Pos2::new(panel.left(), y), Vec2::new(panel.width(), 104.0));
            let resp = ui.interact(row, Id::new(("deck", deck.id)), Sense::click());
            let hot = resp.hovered();

            let p = ui.painter();
            p.rect_filled(row, 0, if hot { Palette::CARD } else { Palette::SURFACE });
            p.rect_stroke(
                row,
                0,
                Stroke::new(1.0, if hot { Palette::ACCENT } else { Palette::LINE }),
                egui::StrokeKind::Inside,
            );

            let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();
            let st = stats::deck_stats(&self.store, deck.id).unwrap_or_default();

            p.text(
                row.left_top() + Vec2::new(18.0, 16.0),
                Align2::LEFT_TOP,
                &deck.title,
                text::title(),
                Palette::TEXT,
            );
            p.text(
                row.left_top() + Vec2::new(18.0, 48.0),
                Align2::LEFT_TOP,
                format!(
                    "{} cards   {} new   {} due",
                    counts.total, counts.fresh, counts.due
                ),
                text::small(),
                Palette::TEXT_DIM,
            );

            // Readiness meter along the bottom edge of the row.
            let bar = Rect::from_min_size(
                row.left_bottom() + Vec2::new(18.0, -26.0),
                Vec2::new(row.width() - 36.0, 3.0),
            );
            p.rect_filled(bar, 0, Palette::LINE);
            let mut filled = bar;
            filled.set_width(bar.width() * st.readiness as f32);
            p.rect_filled(filled, 0, Palette::ACCENT);
            p.text(
                row.right_top() + Vec2::new(-18.0, 16.0),
                Align2::RIGHT_TOP,
                format!("{:.0}%", st.readiness * 100.0),
                text::number(),
                if st.readiness > 0.6 {
                    Palette::CORRECT
                } else {
                    Palette::TEXT_DIM
                },
            );

            if let Some(exam) = deck.exam_at {
                let left = exam - now_ms();
                let (txt, col) = if left > 0 {
                    (format!("exam in {}", fmt_span(left)), Palette::ACCENT)
                } else {
                    ("exam passed".to_string(), Palette::TEXT_FAINT)
                };
                p.text(
                    row.right_top() + Vec2::new(-18.0, 52.0),
                    Align2::RIGHT_TOP,
                    tracked(&txt),
                    text::label(),
                    col,
                );
            }

            if resp.clicked() {
                next = self.begin(deck.clone(), Mode::Practice);
            }
            y += 116.0;
        }

        ui.painter().text(
            Pos2::new(panel.left(), panel.bottom() - 18.0),
            Align2::LEFT_BOTTOM,
            tracked("click a deck to study"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        next
    }

    fn begin(&mut self, deck: Deck, mode: Mode) -> Option<Screen> {
        let session = match Session::start(self.store.clone(), deck.id, mode) {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("could not start session: {e}"));
                return None;
            }
        };
        let topics: HashMap<i64, String> = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|t: Topic| (t.id, t.title))
            .collect();
        let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();

        let mut study = Study {
            session,
            deck,
            topics,
            current: None,
            motion: Motion::deal(),
            recent: Vec::new(),
            selected: Vec::new(),
            feedback: None,
            answered: 0,
            correct: 0,
            counts,
            grab: None,
        };
        self.coin.spin();
        self.deal_next(&mut study);
        Some(Screen::Study(Box::new(study)))
    }

    fn deal_next(&mut self, study: &mut Study) {
        let picked = scheduler::next_card(
            &self.store,
            study.deck.id,
            study.session.mode(),
            &study.recent,
            None,
        );
        match picked {
            Ok(Some(q)) => {
                study.session.show(q.id);
                study.recent.push(q.id);
                if study.recent.len() > 12 {
                    study.recent.remove(0);
                }
                study.current = Some(q);
                study.motion = Motion::deal();
                study.selected.clear();
            }
            Ok(None) => study.current = None,
            Err(e) => {
                self.error = Some(format!("could not pick a card: {e}"));
                study.current = None;
            }
        }
        study.counts = scheduler::counts(&self.store, study.deck.id).unwrap_or_default();
    }
}

// ----------------------------------------------------------- study screen --

impl App {
    fn study_screen(&mut self, ui: &mut egui::Ui, study: &mut Study) -> Option<Screen> {
        let ctx = ui.ctx().clone();
        let dt = ui.input(|i| i.stable_dt).min(1.0 / 20.0);

        if study.motion.update(dt) {
            ctx.request_repaint();
        }

        let full = ui.available_rect_before_wrap();
        chrome(ui, study, full, &mut self.coin);

        let stage = Rect::from_min_max(
            full.left_top() + Vec2::new(0.0, 74.0),
            full.right_bottom() - Vec2::new(0.0, 54.0),
        );

        // A card that has been answered and flown off the screen is retired
        // here, once its animation has finished.
        if study.motion.is_gone(full) && study.feedback.is_none() {
            self.deal_next(study);
        }

        let mut action = Action::None;

        match study.current.clone() {
            Some(q) => {
                let card_rect = Rect::from_center_size(
                    stage.center(),
                    Vec2::new(
                        stage.width().min(560.0) - 40.0,
                        stage.height().min(420.0) - 20.0,
                    ),
                );
                match &q.body {
                    Body::TrueFalse { .. } => {
                        action = self.true_false_card(ui, study, &q, card_rect, full);
                    }
                    Body::MultipleChoice { options, multi } => {
                        let opts = options.clone();
                        let multi = *multi;
                        action = self.choice_card(ui, study, &q, &opts, multi, stage);
                    }
                }
            }
            None => {
                ui.painter().text(
                    stage.center(),
                    Align2::CENTER_CENTER,
                    "nothing due right now",
                    text::body(),
                    Palette::TEXT_DIM,
                );
            }
        }

        if let Some(fb) = &study.feedback {
            let auto = fb.grade.correct && fb.since.elapsed().as_secs_f32() > AUTO_ADVANCE;
            feedback_panel(ui, fb, stage, full);
            ctx.request_repaint();
            if auto {
                action = Action::Continue;
            }
        }

        if let Some(key) = self.keys(&ctx, study) {
            action = key;
        }

        self.apply(study, action)
    }

    /// Top and bottom chrome: deck, counters, exam countdown, key hints.
    fn keys(&mut self, ctx: &egui::Context, study: &Study) -> Option<Action> {
        ctx.input(|i| {
            use egui::Key::*;
            if i.key_pressed(Escape) {
                return Some(Action::Quit);
            }
            if study.feedback.is_some() {
                return (i.key_pressed(Space) || i.key_pressed(Enter)).then_some(Action::Continue);
            }
            if i.key_pressed(U) {
                return Some(Action::Undo);
            }
            if i.key_pressed(S) {
                return Some(Action::Skip);
            }

            match study.current.as_ref().map(|q| &q.body) {
                Some(Body::TrueFalse { .. }) => {
                    if i.key_pressed(ArrowLeft) || i.key_pressed(A) {
                        Some(Action::Answer(
                            Response::TrueFalse { value: false },
                            Input::Key,
                        ))
                    } else if i.key_pressed(ArrowRight) || i.key_pressed(D) {
                        Some(Action::Answer(
                            Response::TrueFalse { value: true },
                            Input::Key,
                        ))
                    } else {
                        None
                    }
                }
                Some(Body::MultipleChoice { options, multi }) => {
                    for (n, key) in [Num1, Num2, Num3, Num4, Num5].into_iter().enumerate() {
                        if i.key_pressed(key) && n < options.len() {
                            return Some(Action::Pick(n, *multi));
                        }
                    }
                    (*multi && (i.key_pressed(Enter) || i.key_pressed(Space)))
                        .then_some(Action::CommitPicks)
                }
                None => None,
            }
        })
    }

    fn apply(&mut self, study: &mut Study, action: Action) -> Option<Screen> {
        match action {
            Action::None => None,

            Action::Answer(response, input) => {
                let q = study.current.clone()?;
                match study.session.answer(&q, &response, input) {
                    Ok(outcome) => {
                        self.coin.spin();
                        study.answered += 1;
                        if outcome.grade.correct {
                            study.correct += 1;
                        }
                        study.feedback = Some(Feedback {
                            question: q,
                            grade: outcome.grade,
                            response,
                            since: Instant::now(),
                            outcome,
                        });
                    }
                    Err(e) => self.error = Some(format!("could not record answer: {e}")),
                }
                None
            }

            Action::Pick(idx, multi) => {
                if multi {
                    if let Some(pos) = study.selected.iter().position(|&i| i == idx) {
                        study.selected.remove(pos);
                    } else {
                        study.selected.push(idx);
                    }
                    None
                } else {
                    let r = Response::MultipleChoice {
                        selected: vec![idx],
                    };
                    self.apply(study, Action::Answer(r, Input::Click))
                }
            }

            Action::CommitPicks => {
                if study.selected.is_empty() {
                    return None;
                }
                let mut selected = study.selected.clone();
                selected.sort_unstable();
                let r = Response::MultipleChoice { selected };
                self.apply(study, Action::Answer(r, Input::Click))
            }

            Action::Continue => {
                study.feedback = None;
                // A true/false card is already off screen; a choice card is
                // still sitting there and needs replacing now.
                self.deal_next(study);
                None
            }

            Action::Skip => {
                if let Some(q) = study.current.clone() {
                    study.session.skip(q.id);
                    self.deal_next(study);
                }
                None
            }

            Action::Undo => {
                match study.session.undo_last() {
                    Ok(Some(_)) => {
                        study.answered = study.answered.saturating_sub(1);
                        study.feedback = None;
                        self.deal_next(study);
                    }
                    Ok(None) => {}
                    Err(e) => self.error = Some(format!("undo failed: {e}")),
                }
                None
            }

            Action::Quit => {
                let session_id = study.session.id();
                let deck_id = study.deck.id;
                if let Err(e) = study.session.end() {
                    self.error = Some(format!("could not close session: {e}"));
                }
                let stats_now = stats::session_stats(&self.store, session_id).ok()?;
                let weakest = stats::weakest(&self.store, deck_id, 8).unwrap_or_default();
                Some(Screen::Summary(Summary {
                    session_id,
                    deck_id,
                    stats: stats_now,
                    weakest,
                }))
            }
        }
    }
}

enum Action {
    None,
    Answer(Response, Input),
    Pick(usize, bool),
    CommitPicks,
    Continue,
    Skip,
    Undo,
    Quit,
}

fn chrome(ui: &egui::Ui, study: &Study, full: Rect, coin: &mut CoinAnimation) {
    let top = Rect::from_min_size(full.left_top(), Vec2::new(full.width(), 56.0));
    let coin_rect =
        Rect::from_center_size(top.left_center() + Vec2::new(25.0, 0.0), Vec2::splat(36.0));
    coin.paint(ui, coin_rect);

    let p = ui.painter();

    p.text(
        top.left_center() + Vec2::new(54.0, 0.0),
        Align2::LEFT_CENTER,
        &study.deck.title,
        text::label(),
        Palette::TEXT_DIM,
    );

    let acc = if study.answered > 0 {
        study.correct as f32 / study.answered as f32
    } else {
        0.0
    };
    p.text(
        top.center(),
        Align2::CENTER_CENTER,
        format!("{} / {}", study.correct, study.answered),
        text::title(),
        if study.answered == 0 {
            Palette::TEXT_FAINT
        } else if acc >= 0.7 {
            Palette::CORRECT
        } else {
            Palette::TEXT
        },
    );

    let right = match study.deck.exam_at {
        Some(exam) if exam > now_ms() => format!("exam in {}", fmt_span(exam - now_ms())),
        _ => format!("{} due", study.counts.due + study.counts.fresh),
    };
    p.text(
        top.right_center() + Vec2::new(-22.0, 0.0),
        Align2::RIGHT_CENTER,
        tracked(&right),
        text::label(),
        Palette::ACCENT,
    );

    p.line_segment(
        [top.left_bottom(), top.right_bottom()],
        Stroke::new(1.0, Palette::LINE),
    );

    // Session accuracy as a hairline under the header.
    if study.answered > 0 {
        let mut bar = Rect::from_min_size(top.left_bottom(), Vec2::new(full.width(), 2.0));
        bar.set_width(full.width() * acc);
        p.rect_filled(bar, 0, Palette::ACCENT.gamma_multiply(0.8));
    }

    let hint = match study.current.as_ref().map(|q| &q.body) {
        Some(Body::TrueFalse { .. }) => "drag or ← →  ·  s skip  ·  u undo  ·  esc end",
        Some(Body::MultipleChoice { multi: true, .. }) => {
            "1-5 toggle  ·  enter confirm  ·  s skip  ·  esc end"
        }
        Some(Body::MultipleChoice { .. }) => "click or 1-5  ·  s skip  ·  u undo  ·  esc end",
        None => "esc end",
    };
    p.text(
        full.center_bottom() - Vec2::new(0.0, 20.0),
        Align2::CENTER_CENTER,
        hint,
        text::label(),
        Palette::TEXT_FAINT,
    );
}

fn fmt_span(ms: i64) -> String {
    let mins = ms / 60_000;
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn fmt_ms(ms: i64) -> String {
    if ms < 0 {
        return "-".into();
    }
    if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

// ------------------------------------------------------------- card views --

impl App {
    fn true_false_card(
        &mut self,
        ui: &mut egui::Ui,
        study: &mut Study,
        q: &Question,
        rect: Rect,
        full: Rect,
    ) -> Action {
        let mut action = Action::None;
        let interactive = study.feedback.is_none() && !study.motion.is_flying();

        if interactive {
            let resp = ui.interact(
                rect.translate(study.motion.offset),
                Id::new(("tf", q.id)),
                Sense::click_and_drag(),
            );

            if resp.drag_started() {
                study.motion.dragging = true;
                study.grab = resp.interact_pointer_pos().map(|p| p - rect.center());
                study
                    .session
                    .log(Event::SwipeStart, Some(q.id), serde_json::Value::Null);
            }
            if study.motion.dragging
                && let (Some(pointer), Some(grab)) = (resp.interact_pointer_pos(), study.grab)
            {
                study.motion.offset = (pointer - grab) - rect.center();
            }
            if resp.drag_stopped() {
                study.motion.dragging = false;
                study.grab = None;
                match study.motion.pending_dir() {
                    Some(dir) => {
                        study.motion.launch(dir);
                        action =
                            Action::Answer(Response::TrueFalse { value: dir > 0.0 }, Input::Swipe);
                    }
                    None => {
                        study.session.log(
                            Event::SwipeCancel,
                            Some(q.id),
                            serde_json::json!({ "offset_x": study.motion.offset.x }),
                        );
                    }
                }
            }

            // Mouse without dragging: left click = false, right click = true.
            if resp.clicked() {
                study.motion.launch(-1.0);
                action = Action::Answer(Response::TrueFalse { value: false }, Input::Click);
            }
            if resp.secondary_clicked() {
                study.motion.launch(1.0);
                action = Action::Answer(Response::TrueFalse { value: true }, Input::Click);
            }
        }

        // The card that was just answered is still animating away; keep
        // drawing it until it clears the screen.
        if study.feedback.is_some() && !study.motion.is_flying() {
            return action;
        }

        let motion = study.motion.clone();
        let opacity = motion.opacity();
        let angle = motion.angle();
        let scale = motion.entry_scale();
        let drawn = Rect::from_center_size(rect.center() + motion.offset, rect.size() * scale);
        let pivot = drawn.center();
        let p = ui.painter();

        card::deck_behind(p, rect, 3, Palette::CARD_DEEP, Palette::LINE);

        let progress = motion.commit_progress();
        let edge = if progress > 0.05 {
            Palette::ACCENT.gamma_multiply(0.3 + 0.7 * progress.abs())
        } else if progress < -0.05 {
            Palette::VIOLET.gamma_multiply(0.3 + 0.7 * progress.abs())
        } else {
            Palette::LINE_BRIGHT
        };

        card::face(
            p,
            drawn,
            angle,
            Palette::CARD.gamma_multiply(opacity),
            Stroke::new(1.0, edge.gamma_multiply(opacity)),
        );

        // Topic label and difficulty pips along the top edge.
        let topic = q
            .topic_id
            .and_then(|t| study.topics.get(&t))
            .cloned()
            .unwrap_or_default();
        let g = p.layout_no_wrap(tracked(&topic), text::label(), Palette::TEXT_FAINT);
        card::text(
            p,
            pivot,
            angle,
            drawn.left_top() + Vec2::new(24.0, 22.0),
            g,
            Palette::TEXT_FAINT,
            opacity,
        );
        for i in 0..5 {
            let filled = i < q.difficulty as usize;
            let c = if filled {
                Palette::TEXT_DIM
            } else {
                Palette::LINE
            };
            let at = drawn.right_top() + Vec2::new(-24.0 - (4 - i) as f32 * 9.0, 26.0);
            let pip = Rect::from_center_size(at, Vec2::splat(4.0));
            p.add(eframe::epaint::Shape::convex_polygon(
                card::corners(pip, pivot, angle),
                c.gamma_multiply(opacity),
                Stroke::NONE,
            ));
        }

        // Prompt, wrapped and vertically centred.
        let wrap = drawn.width() - 56.0;
        let size = if q.prompt.len() > 180 { 16.5 } else { 19.0 };
        let g = p.layout(q.prompt.clone(), text::prompt(size), Palette::TEXT, wrap);
        let local = Pos2::new(
            drawn.left() + 28.0,
            drawn.center().y - g.rect.height() / 2.0 - 6.0,
        );
        card::text(p, pivot, angle, local, g, Palette::TEXT, opacity);

        // Footer rail with the two directions.
        card::text_centered(
            p,
            pivot,
            angle,
            drawn.center_bottom() - Vec2::new(0.0, 30.0),
            "◀  FALSE          TRUE  ▶",
            text::label(),
            Palette::TEXT_FAINT.gamma_multiply(1.0 - progress.abs()),
            opacity,
        );

        card::stamp(
            p,
            drawn,
            angle,
            "FALSE",
            Palette::VIOLET,
            Align2::LEFT_TOP,
            (-progress).max(0.0),
        );
        card::stamp(
            p,
            drawn,
            angle,
            "TRUE",
            Palette::ACCENT,
            Align2::RIGHT_TOP,
            progress.max(0.0),
        );

        let _ = full;
        action
    }

    #[allow(clippy::too_many_arguments)]
    fn choice_card(
        &mut self,
        ui: &mut egui::Ui,
        study: &mut Study,
        q: &Question,
        options: &[idiosepius_core::Choice],
        multi: bool,
        stage: Rect,
    ) -> Action {
        let mut action = Action::None;
        let revealed = study.feedback.is_some();

        let width = stage.width().min(660.0);
        let wrap = width - 48.0;

        // Measure first: the card is sized to its content, so a two-line
        // question does not sit in a half-empty box.
        let prompt_galley =
            ui.painter()
                .layout(q.prompt.clone(), text::prompt(17.5), Palette::TEXT, wrap);
        let option_galleys: Vec<_> = options
            .iter()
            .map(|o| {
                ui.painter()
                    .layout(o.text.clone(), text::body(), Palette::TEXT, wrap - 54.0)
            })
            .collect();

        let options_h: f32 = option_galleys
            .iter()
            .map(|g| (g.rect.height() + 22.0).max(40.0) + 8.0)
            .sum();
        let content_h = 48.0
            + prompt_galley.rect.height()
            + 22.0
            + options_h
            + if multi { 50.0 } else { 0.0 }
            + 20.0;

        let card_rect = Rect::from_center_size(
            stage.center(),
            Vec2::new(width, content_h.min(stage.height())),
        );

        let opacity = study.motion.opacity();
        let p = ui.painter();
        p.rect_filled(card_rect, 0, Palette::CARD.gamma_multiply(opacity));
        p.rect_stroke(
            card_rect,
            0,
            Stroke::new(1.0, Palette::LINE_BRIGHT.gamma_multiply(opacity)),
            egui::StrokeKind::Inside,
        );

        let topic = q
            .topic_id
            .and_then(|t| study.topics.get(&t))
            .cloned()
            .unwrap_or_default();
        p.text(
            card_rect.left_top() + Vec2::new(24.0, 20.0),
            Align2::LEFT_TOP,
            tracked(&topic),
            text::label(),
            Palette::TEXT_FAINT,
        );
        if multi {
            p.text(
                card_rect.right_top() + Vec2::new(-24.0, 20.0),
                Align2::RIGHT_TOP,
                tracked("select all"),
                text::label(),
                Palette::VIOLET,
            );
        }

        let prompt_h = prompt_galley.rect.height();
        p.galley(
            card_rect.left_top() + Vec2::new(24.0, 48.0),
            prompt_galley,
            Palette::TEXT,
        );

        let mut y = card_rect.top() + 48.0 + prompt_h + 22.0;

        for (i, (opt, og)) in options.iter().zip(option_galleys).enumerate() {
            let h = (og.rect.height() + 22.0).max(40.0);
            let row = Rect::from_min_size(
                Pos2::new(card_rect.left() + 24.0, y),
                Vec2::new(card_rect.width() - 48.0, h),
            );

            let picked = study.selected.contains(&i);
            let resp = if revealed {
                None
            } else {
                Some(ui.interact(row, Id::new(("opt", q.id, i)), Sense::click()))
            };
            let hot = resp.as_ref().is_some_and(|r| r.hovered());

            let (border, fill, label_col) = if revealed {
                let chose = match &study.feedback.as_ref().unwrap().response {
                    Response::MultipleChoice { selected } => selected.contains(&i),
                    _ => false,
                };
                if opt.correct {
                    (
                        Palette::CORRECT,
                        Palette::CORRECT.gamma_multiply(0.12),
                        Palette::CORRECT,
                    )
                } else if chose {
                    (
                        Palette::WRONG,
                        Palette::WRONG.gamma_multiply(0.12),
                        Palette::WRONG,
                    )
                } else {
                    (Palette::LINE, Color32::TRANSPARENT, Palette::TEXT_DIM)
                }
            } else if picked {
                (
                    Palette::ACCENT,
                    Palette::ACCENT.gamma_multiply(0.10),
                    Palette::ACCENT,
                )
            } else if hot {
                (Palette::LINE_BRIGHT, Palette::SURFACE, Palette::TEXT)
            } else {
                (Palette::LINE, Color32::TRANSPARENT, Palette::TEXT)
            };

            let p = ui.painter();
            if fill != Color32::TRANSPARENT {
                p.rect_filled(row, 0, fill);
            }
            p.rect_stroke(row, 0, Stroke::new(1.0, border), egui::StrokeKind::Inside);

            // Index box on the left, so 1-5 on the keyboard is discoverable.
            let key_box = Rect::from_min_size(row.left_top(), Vec2::new(34.0, row.height()));
            p.line_segment(
                [key_box.right_top(), key_box.right_bottom()],
                Stroke::new(1.0, border),
            );
            p.text(
                key_box.center(),
                Align2::CENTER_CENTER,
                format!("{}", i + 1),
                text::label(),
                label_col,
            );
            p.galley(row.left_top() + Vec2::new(48.0, 11.0), og, label_col);

            if resp.is_some_and(|r| r.clicked()) {
                action = Action::Pick(i, multi);
            }
            y += h + 8.0;
        }

        if multi && !revealed {
            let btn = Rect::from_min_size(
                Pos2::new(card_rect.left() + 24.0, y + 6.0),
                Vec2::new(card_rect.width() - 48.0, 38.0),
            );
            let resp = ui.interact(btn, Id::new(("commit", q.id)), Sense::click());
            let live = !study.selected.is_empty();
            let col = if !live {
                Palette::TEXT_FAINT
            } else if resp.hovered() {
                Palette::ACCENT
            } else {
                Palette::TEXT
            };
            let p = ui.painter();
            p.rect_stroke(btn, 0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
            p.text(
                btn.center(),
                Align2::CENTER_CENTER,
                tracked("confirm"),
                text::label(),
                col,
            );
            if resp.clicked() && live {
                action = Action::CommitPicks;
            }
        }

        action
    }
}

fn feedback_panel(ui: &egui::Ui, fb: &Feedback, stage: Rect, full: Rect) {
    let is_tf = matches!(fb.question.body, Body::TrueFalse { .. });

    // Grow in over ~120 ms so the verdict does not simply blink into being.
    let t = (fb.since.elapsed().as_secs_f32() / 0.12).clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - t).powi(3);

    let colour = if fb.grade.correct {
        Palette::CORRECT
    } else {
        Palette::WRONG
    };
    let verdict = if fb.grade.correct {
        "CORRECT"
    } else if fb.grade.score > 0.0 {
        "PARTLY RIGHT"
    } else {
        "WRONG"
    };

    let explanation = fb.question.explanation.clone().unwrap_or_default();
    let truth = match &fb.question.body {
        Body::TrueFalse { answer } => Some(if *answer {
            "the statement is TRUE"
        } else {
            "the statement is FALSE"
        }),
        _ => None,
    };

    let wrap = stage.width().min(620.0) - 48.0;
    let g = ui
        .painter()
        .layout(explanation.clone(), text::body(), Palette::TEXT_DIM, wrap);

    let height = g.rect.height() + if is_tf { 132.0 } else { 96.0 };
    let panel = if is_tf {
        Rect::from_center_size(stage.center(), Vec2::new(wrap + 48.0, height))
    } else {
        Rect::from_min_size(
            Pos2::new(
                full.center().x - (wrap + 48.0) / 2.0,
                full.bottom() - height - 44.0,
            ),
            Vec2::new(wrap + 48.0, height),
        )
    };
    let panel = Rect::from_center_size(
        panel.center(),
        Vec2::new(panel.width(), panel.height() * (0.9 + 0.1 * ease)),
    );

    let p = ui.painter();
    p.rect_filled(panel, 0, Palette::SURFACE);
    p.rect_stroke(panel, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    // Accent rail on the left edge.
    p.rect_filled(
        Rect::from_min_size(panel.left_top(), Vec2::new(3.0, panel.height())),
        0,
        colour,
    );

    p.text(
        panel.left_top() + Vec2::new(24.0, 20.0),
        Align2::LEFT_TOP,
        tracked(verdict),
        text::label(),
        colour,
    );
    p.text(
        panel.right_top() + Vec2::new(-24.0, 20.0),
        Align2::RIGHT_TOP,
        fmt_ms(fb.outcome.latency_ms),
        text::label(),
        Palette::TEXT_FAINT,
    );

    let mut y = panel.top() + 46.0;
    if let Some(truth) = truth {
        p.text(
            Pos2::new(panel.left() + 24.0, y),
            Align2::LEFT_TOP,
            truth,
            text::prompt(17.0),
            Palette::TEXT,
        );
        y += 30.0;
    }
    p.galley(Pos2::new(panel.left() + 24.0, y), g, Palette::TEXT_DIM);

    p.text(
        panel.center_bottom() - Vec2::new(0.0, 18.0),
        Align2::CENTER_CENTER,
        tracked("space to continue"),
        text::label(),
        Palette::TEXT_FAINT,
    );
}

// --------------------------------------------------------- summary screen --

impl App {
    fn summary_screen(&mut self, ui: &mut egui::Ui, sum: &mut Summary) -> Option<Screen> {
        let full = ui.available_rect_before_wrap();
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(620.0), full.height().min(560.0)),
        );

        let s = &sum.stats;
        let p = ui.painter();

        p.text(
            panel.left_top(),
            Align2::LEFT_TOP,
            tracked("session"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            panel.left_top() + Vec2::new(0.0, 26.0),
            Align2::LEFT_TOP,
            format!("{} / {}", s.correct, s.answered),
            headline(),
            if s.accuracy >= 0.7 {
                Palette::CORRECT
            } else {
                Palette::TEXT
            },
        );
        p.text(
            panel.right_top() + Vec2::new(0.0, 30.0),
            Align2::RIGHT_TOP,
            format!("{:.0}%", s.accuracy * 100.0),
            headline(),
            Palette::ACCENT,
        );
        p.text(
            panel.left_top() + Vec2::new(0.0, 86.0),
            Align2::LEFT_TOP,
            format!(
                "{} answered   {} skipped   {} studied",
                s.answered,
                s.skipped,
                fmt_ms(s.duration_ms)
            ),
            text::small(),
            Palette::TEXT_DIM,
        );

        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 112.0),
                panel.right_top() + Vec2::new(0.0, 112.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        p.text(
            panel.left_top() + Vec2::new(0.0, 130.0),
            Align2::LEFT_TOP,
            tracked("worth another look"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        let mut y = panel.top() + 158.0;
        for w in sum.weakest.iter().take(7) {
            let short: String = w.prompt.chars().take(64).collect();
            p.text(
                Pos2::new(panel.left() + 52.0, y),
                Align2::LEFT_TOP,
                short,
                text::small(),
                Palette::TEXT_DIM,
            );
            p.text(
                Pos2::new(panel.left(), y),
                Align2::LEFT_TOP,
                format!("{:>3.0}%", w.ema * 100.0),
                text::small(),
                if w.ema < 0.4 {
                    Palette::WRONG
                } else {
                    Palette::TEXT_FAINT
                },
            );
            y += 24.0;
        }

        let btn = Rect::from_min_size(
            Pos2::new(panel.left(), panel.bottom() - 44.0),
            Vec2::new(190.0, 38.0),
        );
        let resp = ui.interact(btn, Id::new("back"), Sense::click());
        let col = if resp.hovered() {
            Palette::ACCENT
        } else {
            Palette::TEXT_DIM
        };
        let p = ui.painter();
        p.rect_stroke(btn, 0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
        p.text(
            btn.center(),
            Align2::CENTER_CENTER,
            tracked("back to decks"),
            text::label(),
            col,
        );

        let again = Rect::from_min_size(
            Pos2::new(panel.left() + 206.0, panel.bottom() - 44.0),
            Vec2::new(190.0, 38.0),
        );
        let resp2 = ui.interact(again, Id::new("again"), Sense::click());
        let col2 = if resp2.hovered() {
            Palette::ACCENT
        } else {
            Palette::TEXT_DIM
        };
        let p = ui.painter();
        p.rect_stroke(again, 0, Stroke::new(1.0, col2), egui::StrokeKind::Inside);
        p.text(
            again.center(),
            Align2::CENTER_CENTER,
            tracked("study again"),
            text::label(),
            col2,
        );

        let _ = sum.session_id;
        let deck_id = sum.deck_id;

        if resp.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.decks = self.store.decks().unwrap_or_default();
            return Some(Screen::Decks);
        }
        if resp2.clicked() {
            let deck = self.store.deck(deck_id).ok().flatten()?;
            return self.begin(deck, Mode::Practice);
        }
        None
    }
}

/// The oversized figure used for headline numbers.
fn headline() -> egui::FontId {
    egui::FontId::new(44.0, egui::FontFamily::Monospace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_read_naturally() {
        assert_eq!(fmt_span(90 * 60_000), "1h 30m");
        assert_eq!(fmt_span(3 * 1440 * 60_000 + 4 * 3_600_000), "3d 4h");
        assert_eq!(fmt_span(45_000), "0m");
    }

    #[test]
    fn unknown_latency_is_not_shown_as_a_time() {
        assert_eq!(fmt_ms(-1), "-");
        assert_eq!(fmt_ms(2500), "2.5s");
        assert_eq!(fmt_ms(65_000), "1m 5s");
    }
}
