//! Screens and interaction.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use idiosepius_core::model::{Deck, Topic};
use idiosepius_core::session::{Event, Outcome};
use idiosepius_core::{
    Body, Grade, Input, Mode, Question, Response, Session, Store, now_ms, scheduler, stats,
};

use crate::card::{self, Motion};
use crate::coin::CoinAnimation;
use crate::explain::{self, Depth, Facts};
use crate::richtext;
use crate::theme::{Palette, text, tracked};

/// How many answered cards stay reachable with `r`.
///
/// Not the whole session: this is for "wait, what was that one", not for
/// browsing history, which is what the summary screen and `idiodb` are for.
const REVIEW_HISTORY: usize = 40;

pub struct App {
    store: Rc<Store>,
    screen: Screen,
    decks: Vec<Deck>,
    error: Option<String>,
    coin: CoinAnimation,
    /// A brief confirmation in the corner: a copy, an import, an export.
    notice: Option<Notice>,
    /// Something the shell has to do, asked for by a screen this frame.
    request: Option<Request>,
    /// A native file dialog, running on a thread of its own so the window
    /// keeps painting while it is open.
    #[cfg(not(target_arch = "wasm32"))]
    dialog: Option<std::sync::mpsc::Receiver<Picked>>,
    /// A review asked for this frame, waiting for the screen it came from.
    pending_review: Option<Box<Review>>,
    /// Development aid: render a few frames, save a PNG, quit. Lets the UI be
    /// checked headlessly (under Xvfb, in CI) instead of by eye.
    shot: Option<Shot>,
}

/// Something only the shell around the app can do: put a file picker on
/// screen, hand a file back to the user.
///
/// The deck screen records the ask and someone else carries it out — a native
/// dialog on the desktop, an `<input type=file>` and a download in the
/// browser — because those two have nothing in common but the intent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Request {
    ImportDeck,
    ExportDatabase,
}

/// What a native file dialog came back with.
#[cfg(not(target_arch = "wasm32"))]
enum Picked {
    Decks(Vec<std::path::PathBuf>),
    ExportTo(std::path::PathBuf),
    /// The dialog was cancelled, or could not be shown at all.
    Nothing,
}

/// A short-lived confirmation in the top-right corner.
struct Notice {
    text: String,
    since: Instant,
    ttl: Duration,
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
    /// A sheet of formulas, for checking the renderer. Reachable only through
    /// `--shot --screen math`: a missing glyph or a fraction sitting a pixel
    /// off is not something to discover on a card the night before an exam.
    MathCheck,
    Study(Box<Study>),
    Summary(Summary),
    /// Looking back at a card that has already been answered. Holds the screen
    /// it was opened from, so closing it returns exactly where you were —
    /// including a study session that is still running.
    Review(Box<Review>),
}

struct Study {
    session: Session,
    deck: Deck,
    topics: HashMap<i64, String>,
    facts: Rc<Facts>,
    current: Option<Question>,
    motion: Motion,
    /// Tail of question ids already shown, for interleaving.
    recent: Vec<i64>,
    /// Cards answered this session, oldest first, for looking back at them.
    history: Vec<Answered>,
    /// Multiple-choice selection for the current card.
    selected: Vec<usize>,
    feedback: Option<Feedback>,
    answered: u32,
    correct: u32,
    counts: scheduler::Counts,
    /// Where the pointer grabbed the card, in card-local coordinates.
    grab: Option<Vec2>,
}

/// A card that has been answered, kept so it can be looked at again.
#[derive(Clone)]
struct Answered {
    question: Question,
    /// What was answered, when that is known. A card opened from the summary
    /// screen is a card to re-read, not a record of one attempt.
    response: Option<Response>,
    grade: Option<Grade>,
}

struct Feedback {
    question: Question,
    /// Both are absent when `e` revealed the answer using skip semantics.
    grade: Option<Grade>,
    response: Option<Response>,
    since: Instant,
    outcome: Option<Outcome>,
    depth: Depth,
    /// Pointer position when the press started, so releasing a swipe does not
    /// also dismiss the explanation it just produced.
    press_origin: Option<Pos2>,
}

struct Summary {
    session_id: i64,
    deck_id: i64,
    stats: stats::SessionStats,
    weakest: Vec<stats::WeakQuestion>,
    facts: Rc<Facts>,
}

/// Looking back over answered cards.
struct Review {
    items: Vec<Answered>,
    idx: usize,
    depth: Depth,
    facts: Rc<Facts>,
    topics: HashMap<i64, String>,
    back: Screen,
}

impl App {
    pub fn new(ctx: &egui::Context, store: Store, shot: Option<Shot>) -> Self {
        crate::theme::install(ctx);
        // Keep egui's browser-style whole-interface zoom available:
        // Ctrl/Cmd + or =, Ctrl/Cmd -, and Ctrl/Cmd 0 to reset. This scales
        // cards, explanations, formulas and chrome together rather than
        // special-casing whichever piece of text happens to be hard to read.
        ctx.options_mut(|o| o.zoom_with_keyboard = true);
        let store = Rc::new(store);
        let decks = store.decks().unwrap_or_default();
        let animate_coin = shot.is_none();
        let mut app = App {
            store,
            screen: Screen::Decks,
            decks,
            error: None,
            coin: CoinAnimation::new(animate_coin),
            notice: None,
            request: None,
            #[cfg(not(target_arch = "wasm32"))]
            dialog: None,
            pending_review: None,
            shot,
        };

        // Jump straight to the screen being captured.
        match app.shot.as_ref().and_then(|s| s.screen.clone()).as_deref() {
            None | Some("decks") => {}
            Some("math") => app.screen = Screen::MathCheck,
            Some(name) => {
                if let Some(deck) = app.decks.first().cloned()
                    && let Some(mut screen) = app.begin(deck, Mode::Practice)
                {
                    app.stage_shot(&mut screen);
                    // The review overlay wraps the screen it was opened from,
                    // which the first frame will do for us.
                    if name == "review"
                        && let Screen::Study(study) = &screen
                    {
                        let items = study.history.clone();
                        let last = items.len().saturating_sub(1);
                        app.open_review(items, last, study.facts.clone(), study.topics.clone());
                    }
                    app.screen = screen;
                }
            }
        }
        app
    }

    pub(crate) fn import_pack(
        &mut self,
        pack: &idiosepius_core::content::Pack,
    ) -> anyhow::Result<idiosepius_core::content::ImportReport> {
        let report = idiosepius_core::content::import_pack(&self.store, pack)?;
        self.decks = self.store.decks()?;
        self.screen = Screen::Decks;
        Ok(report)
    }

    pub(crate) fn export_database(&self) -> anyhow::Result<Vec<u8>> {
        self.store.export_database()
    }

    /// Take the shell action a screen asked for this frame, if any.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// Say something short in the corner: an import landed, a file was written.
    pub(crate) fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            since: Instant::now(),
            ttl: Duration::from_millis(2600),
        });
    }

    pub(crate) fn report_error(&mut self, text: impl Into<String>) {
        self.error = Some(text.into());
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_snapshot(&self) -> idiosepius_core::browser_io::BrowserSnapshot {
        self.store.browser_snapshot()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn browser_checkpoint_snapshot(
        &self,
    ) -> anyhow::Result<idiosepius_core::browser_io::BrowserSnapshot> {
        self.store.browser_checkpoint_snapshot()
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

        // Screens that only exist after an answer or reveal: give ordinary
        // feedback a deliberately wrong answer, and exercise `e` separately.
        let wants_feedback = matches!(
            shot.screen.as_deref(),
            Some("feedback" | "deep" | "review" | "explain")
        );
        let deep = shot.screen.as_deref() == Some("deep");
        if wants_feedback && let Some(q) = study.current.clone() {
            if shot.screen.as_deref() == Some("explain") {
                self.apply(study, Action::Explain);
            } else {
                let wrong = match &q.body {
                    Body::TrueFalse { answer } => Response::TrueFalse { value: !answer },
                    Body::MultipleChoice { options, .. } => Response::MultipleChoice {
                        selected: vec![options.iter().position(|o| !o.correct).unwrap_or(0)],
                    },
                };
                self.apply(study, Action::Answer(wrong, Input::Key));
            }
            if deep && let Some(fb) = &mut study.feedback {
                fb.depth = Depth::Deep;
            }
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
            Screen::MathCheck => {
                math_check(ui);
                None
            }
            Screen::Study(study) => self.study_screen(ui, study),
            Screen::Summary(sum) => self.summary_screen(ui, sum),
            Screen::Review(review) => self.review_screen(ui, review),
        };
        let mut screen = next.unwrap_or(screen);

        // Opening a review needs the screen it was opened from, which only
        // exists here: the handler that asked for it is holding a borrow of it.
        if let Some(mut review) = self.pending_review.take() {
            review.back = screen;
            screen = Screen::Review(review);
        }

        // egui-winit translates Ctrl/Cmd+C into Event::Copy and deliberately
        // does not emit a C key event, so normal shortcut matching cannot see
        // it. Consume the platform copy event directly.
        let copy = ui
            .ctx()
            .input_mut(|input| take_copy_event(&mut input.events));
        if copy {
            let text = self.visible_text(&screen);
            if !text.trim().is_empty() {
                ui.ctx().copy_text(text);
                self.notice = Some(Notice {
                    text: "copied".into(),
                    since: Instant::now(),
                    ttl: Duration::from_millis(900),
                });
            }
        }
        self.screen = screen;

        // On the desktop the app is its own shell, so it answers the file
        // requests itself. In the browser they belong to `BrowserApp`, which
        // takes them after this returns.
        #[cfg(not(target_arch = "wasm32"))]
        self.serve_requests(ui.ctx());

        self.error_bar(ui);
        self.corner_notice(ui);
        self.drive_shot(ui.ctx());
    }
}

// --------------------------------------------------------- native dialogs --

/// Importing a deck and exporting the database, on the desktop.
///
/// The dialog runs on a thread of its own and answers through a channel: a
/// portal dialog can sit open for a minute, and the window behind it has to
/// keep painting. Only one is allowed at a time — a second file picker over
/// the first is a bug, not a feature.
#[cfg(not(target_arch = "wasm32"))]
impl App {
    fn serve_requests(&mut self, ctx: &egui::Context) {
        self.poll_dialog();
        let Some(request) = self.request.take() else {
            return;
        };
        if self.dialog.is_some() {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let suggested = default_export_name();
        let spawned = std::thread::Builder::new()
            .name("idiosepius-file-dialog".into())
            .spawn(move || {
                let picked = match request {
                    Request::ImportDeck => rfd::FileDialog::new()
                        .set_title("Import deck packs")
                        .add_filter("deck packs", &["json", "zip"])
                        .pick_files()
                        .map_or(Picked::Nothing, Picked::Decks),
                    Request::ExportDatabase => rfd::FileDialog::new()
                        .set_title("Export study database")
                        .add_filter("SQLite database", &["db"])
                        .set_file_name(suggested)
                        .save_file()
                        .map_or(Picked::Nothing, Picked::ExportTo),
                };
                let _ = tx.send(picked);
                ctx.request_repaint();
            });

        match spawned {
            Ok(_) => self.dialog = Some(rx),
            Err(e) => self.error = Some(format!("could not open a file dialog: {e}")),
        }
    }

    fn poll_dialog(&mut self) {
        let Some(rx) = &self.dialog else { return };
        let picked = match rx.try_recv() {
            Ok(picked) => picked,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            // The thread died without answering; drop the dialog and move on.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Picked::Nothing,
        };
        self.dialog = None;

        match picked {
            Picked::Nothing => {}
            Picked::Decks(paths) => self.import_paths(&paths),
            Picked::ExportTo(path) => self.export_to(&path),
        }
    }

    /// Import the picked files through exactly the code path the browser uses,
    /// so a ZIP of packs behaves the same on both.
    fn import_paths(&mut self, paths: &[std::path::PathBuf]) {
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            match std::fs::read(path) {
                Ok(bytes) => files.push(crate::import::PickedFile {
                    name: path.to_string_lossy().into_owned(),
                    bytes,
                }),
                Err(e) => {
                    self.report_error(format!("could not read {}: {e}", path.display()));
                    return;
                }
            }
        }

        match crate::import::decode_packs(files).and_then(|pack| self.import_pack(&pack)) {
            Ok(report) => {
                self.notify(format!("imported {} questions", report.questions));
                self.error = None;
            }
            Err(e) => self.report_error(format!("deck import failed: {e:#}")),
        }
    }

    fn export_to(&mut self, path: &std::path::Path) {
        let written = self
            .export_database()
            .and_then(|bytes| Ok(std::fs::write(path, bytes)?));
        match written {
            Ok(()) => {
                self.notify("database exported");
                self.error = None;
            }
            Err(e) => self.report_error(format!("could not export the database: {e:#}")),
        }
    }
}

/// `idiosepius-2026-07-26.db`: dated, because an export is a snapshot and the
/// first thing you want to know about one is when it was taken.
#[cfg(not(target_arch = "wasm32"))]
fn default_export_name() -> String {
    let day = idiosepius_core::content::format_rfc3339_ms(now_ms());
    format!("idiosepius-{}.db", day.get(..10).unwrap_or("export"))
}

fn take_copy_event(events: &mut Vec<egui::Event>) -> bool {
    let had_copy = events
        .iter()
        .any(|event| matches!(event, egui::Event::Copy));
    if had_copy {
        events.retain(|event| !matches!(event, egui::Event::Copy));
    }
    had_copy
}

impl App {
    /// A human-readable transcript of the current screen.
    ///
    /// This deliberately exports authored strings, not painted glyphs or
    /// database JSON. In particular, `$...$` remains LaTeX so a copied card is
    /// immediately useful in notes or in a question to a chatbot.
    fn visible_text(&self, screen: &Screen) -> String {
        match screen {
            Screen::Decks => {
                let mut out = String::from("Idiosepius\n\nDecks");
                if self.decks.is_empty() {
                    out.push_str("\n\nNo decks yet.");
                }
                for deck in &self.decks {
                    let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();
                    let stat = stats::deck_stats(&self.store, deck.id).unwrap_or_default();
                    let _ = write!(
                        out,
                        "\n\n{}\n{} cards · {} new · {} due\nReadiness: {:.0}%",
                        deck.title,
                        counts.total,
                        counts.fresh,
                        counts.due,
                        stat.readiness * 100.0
                    );
                    if let Some(exam) = deck.exam_at {
                        let remaining = exam - now_ms();
                        if remaining > 0 {
                            let _ = write!(out, "\nExam in {}", fmt_span(remaining));
                        } else {
                            out.push_str("\nExam passed");
                        }
                    }
                }
                out
            }
            Screen::MathCheck => {
                let mut out = String::from("Math renderer");
                for (name, formula) in MATH_SAMPLES {
                    let _ = write!(out, "\n\n{name}\n{formula}");
                }
                out
            }
            Screen::Study(study) => {
                let Some(question) = study.current.as_ref() else {
                    return format!("{}\n\nNothing due right now.", study.deck.title);
                };
                let topic = question
                    .topic_id
                    .and_then(|id| study.topics.get(&id))
                    .map(String::as_str);
                match &study.feedback {
                    Some(feedback) => card_text(
                        &feedback.question,
                        topic,
                        &[],
                        feedback.response.as_ref(),
                        feedback.grade,
                        true,
                        explain::NoteView::Picked,
                        Some((&study.facts, feedback.depth)),
                    ),
                    None => card_text(
                        question,
                        topic,
                        &study.selected,
                        None,
                        None,
                        false,
                        explain::NoteView::Hidden,
                        None,
                    ),
                }
            }
            Screen::Review(review) => {
                let Some(item) = review.items.get(review.idx) else {
                    return String::new();
                };
                let topic = item
                    .question
                    .topic_id
                    .and_then(|id| review.topics.get(&id))
                    .map(String::as_str);
                card_text(
                    &item.question,
                    topic,
                    &[],
                    item.response.as_ref(),
                    item.grade,
                    true,
                    explain::NoteView::All,
                    Some((&review.facts, review.depth)),
                )
            }
            Screen::Summary(summary) => {
                let stat = &summary.stats;
                let mut out = format!(
                    "Session complete\n\nAccuracy: {:.0}%\n{} answered · {} skipped · {} studied",
                    stat.accuracy * 100.0,
                    stat.answered,
                    stat.skipped,
                    fmt_ms(stat.duration_ms)
                );
                if !summary.weakest.is_empty() {
                    out.push_str("\n\nWorth another look");
                    for weak in summary.weakest.iter().take(7) {
                        let _ = write!(out, "\n\n{:.0}% · {}", weak.ema * 100.0, weak.prompt);
                    }
                }
                out
            }
        }
    }

    /// Brief, non-modal confirmation of something that just happened — a copy,
    /// an import, an export. It never asks for a click and never blocks.
    fn corner_notice(&mut self, ui: &mut egui::Ui) {
        let Some(note) = &self.notice else {
            return;
        };
        if note.since.elapsed() >= note.ttl {
            self.notice = None;
            return;
        }
        let label = tracked(&note.text);

        let full = ui.max_rect();
        let width = (ui
            .painter()
            .layout_no_wrap(label.clone(), text::label(), Palette::ACCENT)
            .rect
            .width()
            + 26.0)
            .max(102.0);
        let notice = Rect::from_min_size(
            full.right_top() + Vec2::new(-width - 12.0, 12.0),
            Vec2::new(width, 28.0),
        );
        let p = ui.painter();
        p.rect_filled(notice, 0, Palette::SURFACE);
        p.rect_stroke(
            notice,
            0,
            Stroke::new(1.0, Palette::ACCENT),
            egui::StrokeKind::Inside,
        );
        p.text(
            notice.center(),
            Align2::CENTER_CENTER,
            label,
            text::label(),
            Palette::ACCENT,
        );
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

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
                "No decks yet. Import a pack — one or more .json files, or a .zip of them.",
                text::body(),
                Palette::TEXT_DIM,
            );
            y += 42.0;
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

        // Importing sits with the decks because that is what it produces, and
        // it is dashed because it is not one: the row is an opening, not a
        // thing you can study.
        let import =
            Rect::from_min_size(Pos2::new(panel.left(), y), Vec2::new(panel.width(), 54.0));
        if dashed_row(ui, import, "import deck") {
            self.request = Some(Request::ImportDeck);
        }

        // The database is the whole course and the whole history, so taking a
        // copy of it belongs on the screen that lists what is in it — at the
        // bottom, out of the way of the decks themselves.
        let export = Pos2::new(panel.right(), panel.bottom() - 68.0);
        if chrome_button(ui, export, "export database") {
            self.request = Some(Request::ExportDatabase);
        }

        ui.painter().text(
            Pos2::new(panel.left(), panel.bottom() - 18.0),
            Align2::LEFT_BOTTOM,
            tracked("click a deck to study"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        ui.painter().text(
            Pos2::new(panel.right(), panel.bottom() - 18.0),
            Align2::RIGHT_BOTTOM,
            tracked("ctrl ± size  ·  ctrl 0 reset"),
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

        let facts = Rc::new(Facts::load(&self.store, deck.id));
        let mut study = Study {
            session,
            deck,
            topics,
            facts,
            current: None,
            motion: Motion::deal(),
            recent: Vec::new(),
            history: Vec::new(),
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
                // The scheduler decides how much of this tail it actually
                // suppresses; keep enough of it that a large deck can use a
                // wide cooldown window.
                if study.recent.len() > 32 {
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

    /// Put a specific question back on the table, as after an undo.
    fn deal_again(&mut self, study: &mut Study, question_id: i64) {
        match self.store.question(question_id) {
            Ok(Some(q)) => {
                study.session.show(q.id);
                study.current = Some(q);
                study.motion = Motion::deal();
                study.selected.clear();
            }
            Ok(None) => self.deal_next(study),
            Err(e) => {
                self.error = Some(format!("could not reopen that card: {e}"));
                self.deal_next(study);
            }
        }
        study.counts = scheduler::counts(&self.store, study.deck.id).unwrap_or_default();
    }

    /// Open the weak-card list from the summary screen, starting at the one
    /// that was clicked. All of them are loaded, so `← →` walks the list.
    fn open_weakest(&mut self, sum: &Summary, idx: usize) {
        let items: Vec<Answered> = sum
            .weakest
            .iter()
            .filter_map(|w| self.store.question(w.question_id).ok().flatten())
            .map(|question| Answered {
                question,
                response: None,
                grade: None,
            })
            .collect();
        let topics: HashMap<i64, String> = self
            .store
            .topics(sum.deck_id)
            .unwrap_or_default()
            .into_iter()
            .map(|t: Topic| (t.id, t.title))
            .collect();
        self.open_review(items, idx, sum.facts.clone(), topics);
    }

    /// Open the review overlay on the last `items` answered, newest last.
    fn open_review(
        &mut self,
        items: Vec<Answered>,
        idx: usize,
        facts: Rc<Facts>,
        topics: HashMap<i64, String>,
    ) {
        if items.is_empty() {
            return;
        }
        let idx = idx.min(items.len() - 1);
        self.pending_review = Some(Box::new(Review {
            items,
            idx,
            depth: Depth::Short,
            facts,
            topics,
            back: Screen::Decks,
        }));
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

        // The explanation stays up until it is dismissed, right or wrong. A
        // correct answer is exactly when a misconception is cheapest to fix,
        // and a panel that fades on its own is one you learn to ignore.
        if study.feedback.is_some() {
            if let Some(dismissed) = self.feedback_panel(ui, study, stage, full)
                && dismissed
            {
                action = Action::Continue;
            }
            // Only the 120 ms grow-in needs continuous frames. Once settled,
            // an explanation may stay open for minutes without burning CPU.
            if study
                .feedback
                .as_ref()
                .is_some_and(|fb| fb.since.elapsed().as_secs_f32() < 0.12)
            {
                ctx.request_repaint();
            }
        }

        if let Some(key) = self.keys(&ctx, study) {
            action = key;
        }

        self.apply(study, action)
    }

    fn keys(&mut self, ctx: &egui::Context, study: &Study) -> Option<Action> {
        ctx.input(|i| {
            use egui::Key::*;
            if i.key_pressed(Escape) {
                return Some(Action::Quit);
            }
            if i.key_pressed(R) {
                return Some(Action::Look);
            }
            if study.feedback.is_some() {
                if i.key_pressed(D) {
                    return Some(Action::Deeper);
                }
                return (i.key_pressed(Space) || i.key_pressed(Enter)).then_some(Action::Continue);
            }
            if i.key_pressed(U) {
                return Some(Action::Undo);
            }
            if i.key_pressed(S) {
                return Some(Action::Skip);
            }
            if i.key_pressed(E) {
                return Some(Action::Explain);
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
                        study.history.push(Answered {
                            question: q.clone(),
                            response: Some(response.clone()),
                            grade: Some(outcome.grade),
                        });
                        if study.history.len() > REVIEW_HISTORY {
                            study.history.remove(0);
                        }
                        study.feedback = Some(Feedback {
                            question: q,
                            grade: Some(outcome.grade),
                            response: Some(response),
                            since: Instant::now(),
                            outcome: Some(outcome),
                            depth: Depth::Short,
                            press_origin: None,
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

            Action::Explain => {
                if let Some(q) = study.current.clone() {
                    // Revealing is statistically a skip: it is logged, creates
                    // no attempt, and does not pretend the user got it wrong.
                    // Unlike `s`, it leaves the card up long enough to learn
                    // from the answer before dealing the next one.
                    study.session.skip(q.id);
                    study.history.push(Answered {
                        question: q.clone(),
                        response: None,
                        grade: None,
                    });
                    if study.history.len() > REVIEW_HISTORY {
                        study.history.remove(0);
                    }
                    study.feedback = Some(Feedback {
                        question: q,
                        grade: None,
                        response: None,
                        since: Instant::now(),
                        outcome: None,
                        depth: Depth::Short,
                        press_origin: None,
                    });
                }
                None
            }

            // Undo means "let me answer that one again", so the card comes
            // back rather than being replaced by the next one in the queue.
            Action::Undo => {
                match study.session.undo_last() {
                    Ok(Some(question_id)) => {
                        study.answered = study.answered.saturating_sub(1);
                        if let Some(last) = study.history.pop()
                            && last.grade.is_some_and(|g| g.correct)
                        {
                            study.correct = study.correct.saturating_sub(1);
                        }
                        study.feedback = None;
                        self.deal_again(study, question_id);
                    }
                    Ok(None) => {}
                    Err(e) => self.error = Some(format!("undo failed: {e}")),
                }
                None
            }

            Action::Deeper => {
                if let Some(fb) = &mut study.feedback {
                    fb.depth = fb.depth.toggled();
                }
                None
            }

            // Look back over what has already been answered this session.
            // The card on screen is deliberately not included: it has not
            // been answered, and showing it here would give the answer away.
            Action::Look => {
                let items = study.history.clone();
                let last = items.len().saturating_sub(1);
                self.open_review(items, last, study.facts.clone(), study.topics.clone());
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
                    facts: study.facts.clone(),
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
    /// Switch the explanation between its short and deep readings.
    Deeper,
    /// Look back at cards already answered.
    Look,
    Skip,
    /// Reveal the explanation, recorded exactly like a skip.
    Explain,
    Undo,
    Quit,
}

fn card_text(
    question: &Question,
    topic: Option<&str>,
    selected: &[usize],
    response: Option<&Response>,
    grade: Option<Grade>,
    reveal_answer: bool,
    notes: explain::NoteView,
    explanation: Option<(&Facts, Depth)>,
) -> String {
    let mut out = String::new();
    if let Some(topic) = topic.filter(|topic| !topic.trim().is_empty()) {
        let _ = writeln!(out, "{topic}\n");
    }
    let _ = write!(out, "Question\n{}", question.prompt.trim());

    match &question.body {
        Body::TrueFalse { answer } => {
            out.push_str("\n\nChoices\n- True\n- False");
            if let Some(Response::TrueFalse { value }) = response {
                let _ = write!(out, "\n\nMy answer: {}", truth_word(*value));
            }
            if reveal_answer {
                let _ = write!(out, "\nCorrect answer: {}", truth_word(*answer));
            }
        }
        Body::MultipleChoice { options, multi } => {
            out.push_str(if *multi {
                "\n\nChoices (select all that apply)"
            } else {
                "\n\nChoices"
            });
            // The same rule the screen follows: a note names a wrong option,
            // so an unanswered card must not leak one into the transcript
            // either.
            let picked: Vec<usize> = match response {
                Some(Response::MultipleChoice { selected }) => selected.clone(),
                _ => Vec::new(),
            };
            let option_notes = explain::option_notes(options, &picked, notes);
            for (index, option) in options.iter().enumerate() {
                let marker = if !reveal_answer && selected.contains(&index) {
                    " [selected]"
                } else {
                    ""
                };
                let _ = write!(out, "\n{}. {}{marker}", index + 1, option.text.trim());
                if let Some(note) = option_notes[index] {
                    let _ = write!(out, "\n   Note: {note}");
                }
            }

            if let Some(Response::MultipleChoice { selected }) = response {
                let answer = choice_text(options, selected);
                let _ = write!(out, "\n\nMy answer: {answer}");
            }
            if reveal_answer {
                let correct: Vec<usize> = options
                    .iter()
                    .enumerate()
                    .filter_map(|(index, option)| option.correct.then_some(index))
                    .collect();
                let _ = write!(out, "\nCorrect answer: {}", choice_text(options, &correct));
            }
        }
    }

    if let Some(grade) = grade {
        let result = if grade.correct {
            "Correct"
        } else if grade.score > 0.0 {
            "Partly right"
        } else {
            "Wrong"
        };
        let _ = write!(out, "\nResult: {result}");
    } else if reveal_answer && response.is_none() {
        out.push_str("\n\nNo answer was recorded.");
    }

    if let Some((facts, depth)) = explanation {
        let explanation = explain::plain_text(question, facts, depth);
        if !explanation.is_empty() {
            let label = match depth {
                Depth::Short => "Explanation",
                Depth::Deep => "Deep explanation",
            };
            let _ = write!(out, "\n\n{label}\n{explanation}");
        }
    }

    out
}

fn truth_word(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn choice_text(options: &[idiosepius_core::Choice], indices: &[usize]) -> String {
    if indices.is_empty() {
        return "none".to_owned();
    }
    indices
        .iter()
        .filter_map(|&index| {
            options
                .get(index)
                .map(|option| format!("{}. {}", index + 1, option.text.trim()))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn chrome(ui: &egui::Ui, study: &Study, full: Rect, coin: &mut CoinAnimation) {
    let top = Rect::from_min_size(full.left_top(), Vec2::new(full.width(), 56.0));
    let coin_rect =
        Rect::from_center_size(top.left_center() + Vec2::new(25.0, 0.0), Vec2::splat(36.0));
    // The coin spins wherever it is drawn. It is the one piece of pure
    // ornament in the app and it should always answer when you poke it.
    if ui
        .interact(coin_rect, Id::new("chrome-coin"), Sense::click())
        .clicked()
    {
        coin.spin();
    }
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

    let hint = if study.feedback.is_some() {
        "space/click next  ·  d depth  ·  r review  ·  esc end"
    } else {
        match study.current.as_ref().map(|q| &q.body) {
            Some(Body::TrueFalse { .. }) => {
                "←/→ answer  ·  e explain  ·  s skip  ·  u undo  ·  r review"
            }
            Some(Body::MultipleChoice { multi: true, .. }) => {
                "1-5 select  ·  enter confirm  ·  e explain  ·  s skip  ·  r review"
            }
            Some(Body::MultipleChoice { .. }) => {
                "click/1-5 answer  ·  e explain  ·  s skip  ·  u undo  ·  r review"
            }
            None => "esc end",
        }
    };
    p.text(
        full.center_bottom() - Vec2::new(0.0, 20.0),
        Align2::CENTER_CENTER,
        hint,
        text::label(),
        Palette::TEXT_FAINT,
    );
}

/// The brand coin on screens that do not use the study header.
///
/// It lives outside the centred panel so it never steals reading space, but
/// remains available on every user-facing screen.
fn corner_coin(ui: &egui::Ui, full: Rect, coin: &mut CoinAnimation, id: &'static str) {
    let rect = Rect::from_min_size(full.left_top() + Vec2::new(8.0, 8.0), Vec2::splat(38.0));
    if ui.interact(rect, Id::new(id), Sense::click()).clicked() {
        coin.spin();
    }
    coin.paint(ui, rect);
}

/// A deck-width row with a dashed border: an action that would *produce* a
/// deck, sitting where the deck it produces will appear.
///
/// Dashed rather than solid because the row is an opening, not a thing to
/// study — the same hairline the deck rows use, just interrupted, so it reads
/// as belonging to the list without pretending to be a member of it.
fn dashed_row(ui: &egui::Ui, rect: Rect, label: &str) -> bool {
    let response = ui.interact(rect, Id::new(("dashed-row", label)), Sense::click());
    let colour = if response.hovered() {
        Palette::ACCENT
    } else {
        Palette::LINE_BRIGHT
    };
    let ink = if response.hovered() {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };

    let p = ui.painter();
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    p.extend(Shape::dashed_line(
        &corners,
        Stroke::new(1.0, colour),
        6.0,
        5.0,
    ));
    p.text(
        rect.left_center() + Vec2::new(18.0, 0.0),
        Align2::LEFT_CENTER,
        format!("+   {}", tracked(label)),
        text::label(),
        ink,
    );
    response.clicked()
}

/// A small bordered button for chrome actions, laid out from its right edge
/// and sized to its own label — tracked capitals are much wider than the text
/// they are made of, and a guessed width clips them.
fn chrome_button(ui: &egui::Ui, right_top: Pos2, label: &str) -> bool {
    let caps = tracked(label);
    let width = ui
        .painter()
        .layout_no_wrap(caps.clone(), text::label(), Palette::TEXT)
        .rect
        .width()
        + 28.0;
    let rect = Rect::from_min_size(right_top - Vec2::new(width, 0.0), Vec2::new(width, 30.0));
    let response = ui.interact(rect, Id::new(("chrome-button", label)), Sense::click());
    let colour = if response.hovered() {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    let p = ui.painter();
    p.rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        caps,
        text::label(),
        colour,
    );
    response.clicked()
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

/// Left inset of an option note, chosen to line up with the option text
/// rather than with the row edge or the index box.
const NOTE_INDENT: f32 = 48.0;
/// Air between an option row and the note hanging under it.
const NOTE_GAP: f32 = 10.0;

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
        let mut hovered = false;

        if interactive {
            let resp = ui.interact(
                rect.translate(study.motion.offset),
                Id::new(("tf", q.id)),
                Sense::click_and_drag(),
            );
            hovered = resp.hovered();

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

        // A reproducible screenshot of the active state is more useful than
        // trying to race a real pointer against the headless capture.
        hovered |= self.shot.as_ref().and_then(|shot| shot.screen.as_deref()) == Some("hover");

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
        let hover = ui
            .ctx()
            .animate_bool(Id::new(("tf-hover", q.id)), interactive && hovered);
        let edge = if progress > 0.05 {
            Palette::ACCENT.gamma_multiply(0.3 + 0.7 * progress.abs())
        } else if progress < -0.05 {
            Palette::VIOLET.gamma_multiply(0.3 + 0.7 * progress.abs())
        } else {
            Palette::LINE_BRIGHT
        };

        card::hover_glow(
            p,
            drawn,
            angle,
            Palette::TEXT_DIM.gamma_multiply(opacity),
            hover,
        );
        card::face(
            p,
            drawn,
            angle,
            Palette::CARD.gamma_multiply(opacity),
            Stroke::new(1.0 + 1.6 * hover, edge.gamma_multiply(opacity)),
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

        // Prompt, wrapped and vertically centred. Formulas in it are laid out
        // by `math` and tilt with the card like any other ink on its face.
        let wrap = drawn.width() - 56.0;
        let size = if q.prompt.chars().count() > 180 {
            16.5
        } else {
            19.0
        };
        let doc = richtext::layout(p, &q.prompt, size, wrap);
        let local = Pos2::new(
            drawn.left() + 28.0,
            drawn.center().y - doc.height() / 2.0 - 6.0,
        );
        doc.paint_rotated(p, local, pivot, angle, Palette::TEXT, opacity);

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
        let prompt_doc = richtext::layout(ui.painter(), &q.prompt, 17.5, wrap);
        let option_docs: Vec<_> = options
            .iter()
            .map(|o| richtext::layout(ui.painter(), &o.text, 15.0, wrap - 54.0))
            .collect();

        // A note diagnoses the option it sits under, so it is laid out with
        // that row and grows the card, rather than being appended to the
        // explanation panel — which says what is true, not what went wrong.
        // Before the card is answered there is nothing to show: a note names a
        // wrong option and would give the answer away.
        let picked_options: Vec<usize> = match study
            .feedback
            .as_ref()
            .and_then(|feedback| feedback.response.as_ref())
        {
            Some(Response::MultipleChoice { selected }) => selected.clone(),
            _ => Vec::new(),
        };
        let note_docs: Vec<Option<_>> = explain::option_notes(
            options,
            &picked_options,
            if revealed {
                explain::NoteView::Picked
            } else {
                explain::NoteView::Hidden
            },
        )
        .into_iter()
        .map(|note| {
            note.map(|note| richtext::layout(ui.painter(), note, 13.5, wrap - NOTE_INDENT - 12.0))
        })
        .collect();

        let options_h: f32 = option_docs
            .iter()
            .zip(&note_docs)
            .map(|(d, note)| {
                (d.height() + 22.0).max(40.0)
                    + 8.0
                    + note.as_ref().map_or(0.0, |n| n.height() + NOTE_GAP)
            })
            .sum();
        let content_h =
            48.0 + prompt_doc.height() + 22.0 + options_h + if multi { 50.0 } else { 0.0 } + 20.0;

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

        let prompt_h = prompt_doc.height();
        prompt_doc.paint(
            p,
            card_rect.left_top() + Vec2::new(24.0, 48.0),
            Palette::TEXT,
            opacity,
        );

        let mut y = card_rect.top() + 48.0 + prompt_h + 22.0;

        for (i, ((opt, og), note)) in options.iter().zip(option_docs).zip(&note_docs).enumerate() {
            let h = (og.height() + 22.0).max(40.0);
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
            let hover = ui.ctx().animate_bool(Id::new(("opt-hover", q.id, i)), hot);

            let (border, fill, label_col) = if revealed {
                let chose = match study
                    .feedback
                    .as_ref()
                    .and_then(|feedback| feedback.response.as_ref())
                {
                    Some(Response::MultipleChoice { selected }) => selected.contains(&i),
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
            p.rect_stroke(
                row,
                0,
                Stroke::new(1.0 + 0.7 * hover, border),
                egui::StrokeKind::Inside,
            );

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
            og.paint(p, row.left_top() + Vec2::new(48.0, 11.0), label_col, 1.0);

            if resp.is_some_and(|r| r.clicked()) {
                action = Action::Pick(i, multi);
            }
            y += h + 8.0;

            // Indented under the row it belongs to, in that row's verdict
            // colour, so it reads as an annotation on the choice and not as a
            // second explanation.
            if let Some(note) = note {
                note.paint(
                    ui.painter(),
                    Pos2::new(row.left() + NOTE_INDENT, y + NOTE_GAP - 8.0),
                    label_col,
                    1.0,
                );
                y += note.height() + NOTE_GAP;
            }
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

/// The verdict, the truth, and the explanation — the panel you actually learn
/// from, so it scrolls and it waits for you.
///
/// Returns `Some(true)` when the user dismissed it.
impl App {
    fn feedback_panel(
        &mut self,
        ui: &mut egui::Ui,
        study: &mut Study,
        stage: Rect,
        full: Rect,
    ) -> Option<bool> {
        let fb = study.feedback.as_ref()?;
        let (colour, verdict) = match fb.grade {
            Some(grade) if grade.correct => (Palette::CORRECT, "CORRECT"),
            Some(grade) if grade.score > 0.0 => (Palette::WRONG, "PARTLY RIGHT"),
            Some(_) => (Palette::WRONG, "WRONG"),
            None => (Palette::ACCENT, "EXPLANATION"),
        };
        let truth = match &fb.question.body {
            Body::TrueFalse { answer } => Some(if *answer {
                "the statement is TRUE"
            } else {
                "the statement is FALSE"
            }),
            _ => None,
        };

        // Grow in over ~120 ms so the verdict does not blink into being.
        let t = (fb.since.elapsed().as_secs_f32() / 0.12).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - t).powi(3);

        const HEAD: f32 = 46.0;
        const FOOT: f32 = 34.0;
        let width = stage.width().min(640.0);
        let inner_w = width - 48.0;

        let truth_h = if truth.is_some() { 30.0 } else { 0.0 };
        let body_h = explain::measure(ui, inner_w, &fb.question, &study.facts, fb.depth);
        let wanted = HEAD + truth_h + body_h + FOOT + 16.0;
        let height = wanted.min(full.height() - 120.0).max(110.0);

        // A true/false card has flown off; its explanation takes the middle of
        // the stage. A choice card is still on screen, so the panel sits under
        // it rather than over the options it is talking about.
        let is_tf = matches!(fb.question.body, Body::TrueFalse { .. });
        let panel = if is_tf {
            Rect::from_center_size(stage.center(), Vec2::new(width, height))
        } else {
            Rect::from_min_size(
                Pos2::new(full.center().x - width / 2.0, full.bottom() - height - 44.0),
                Vec2::new(width, height),
            )
        };
        let panel = Rect::from_center_size(
            panel.center(),
            Vec2::new(panel.width(), panel.height() * (0.9 + 0.1 * ease)),
        );
        let hovered = ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|position| panel.contains(position))
        });
        let hover = ui
            .ctx()
            .animate_bool(Id::new(("feedback-hover", fb.question.id)), hovered);

        let p = ui.painter();
        p.rect_filled(panel, 0, Palette::SURFACE);
        p.rect_stroke(
            panel,
            0,
            Stroke::new(1.0 + hover, colour),
            egui::StrokeKind::Inside,
        );
        p.rect_filled(
            Rect::from_min_size(
                panel.left_top(),
                Vec2::new(3.0 + 2.0 * hover, panel.height()),
            ),
            0,
            colour,
        );
        p.text(
            panel.left_top() + Vec2::new(24.0, 18.0),
            Align2::LEFT_TOP,
            tracked(verdict),
            text::label(),
            colour,
        );
        p.text(
            panel.right_top() + Vec2::new(-24.0, 18.0),
            Align2::RIGHT_TOP,
            fb.outcome
                .as_ref()
                .map_or_else(|| "-".to_owned(), |outcome| fmt_ms(outcome.latency_ms)),
            text::label(),
            Palette::TEXT_FAINT,
        );

        let mut y = panel.top() + HEAD;
        if let Some(truth) = truth {
            ui.painter().text(
                Pos2::new(panel.left() + 24.0, y),
                Align2::LEFT_TOP,
                truth,
                text::prompt(17.0),
                Palette::TEXT,
            );
            y += truth_h;
        }

        let body_rect = Rect::from_min_max(
            Pos2::new(panel.left() + 24.0, y),
            Pos2::new(panel.right() - 16.0, panel.bottom() - FOOT),
        );
        let (question, facts, depth) = (&fb.question, study.facts.clone(), fb.depth);
        explain::scroll_column(ui, body_rect, "feedback", |ui| {
            explain::body(ui, question, &facts, depth);
        });

        let footer = Rect::from_min_max(
            Pos2::new(panel.left(), panel.bottom() - FOOT),
            panel.right_bottom(),
        );
        let p = ui.painter();
        p.line_segment(
            [footer.left_top(), footer.right_top()],
            Stroke::new(1.0, Palette::LINE),
        );
        p.text(
            footer.left_center() + Vec2::new(24.0, 0.0),
            Align2::LEFT_CENTER,
            tracked("space or click to continue"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            footer.right_center() - Vec2::new(24.0, 0.0),
            Align2::RIGHT_CENTER,
            tracked(depth.label()),
            text::label(),
            Palette::ACCENT,
        );

        Some(self.feedback_dismissed(ui, study, panel))
    }

    /// A stationary click on the explanation panel dismisses it.
    ///
    /// Only a real click: the press and the release have to land in nearly the
    /// same place. Otherwise the release that ends a swipe — the very gesture
    /// that produced this panel — would dismiss it before it could be read.
    ///
    /// Empty margin is deliberately inert, so clicking a backgrounded window
    /// there is always a safe way to bring it to the foreground.
    fn feedback_dismissed(&mut self, ui: &egui::Ui, study: &mut Study, panel: Rect) -> bool {
        let (pressed, released, at) = ui.ctx().input(|i| {
            (
                i.pointer
                    .any_pressed()
                    .then(|| i.pointer.press_origin())
                    .flatten(),
                i.pointer.any_released(),
                i.pointer.interact_pos(),
            )
        });

        // Remember only presses that *began* while the panel was up. The
        // press that answered the card began before it existed, so the release
        // ending that swipe cannot dismiss the explanation it just produced.
        let Some(fb) = &mut study.feedback else {
            return false;
        };
        if let Some(origin) = pressed {
            fb.press_origin = Some(origin);
        }
        let Some(origin) = fb.press_origin else {
            return false;
        };
        if !released {
            return false;
        }
        fb.press_origin = None;

        let Some(at) = at else { return false };
        // A wheel scroll has no press/release, and a touch or scrollbar drag
        // fails the movement check. Both ends must be on the panel: the
        // surrounding margin is a safe focus target.
        click_hits_panel(origin, at, panel)
    }
}

fn click_hits_panel(origin: Pos2, release: Pos2, panel: Rect) -> bool {
    (release - origin).length() <= 6.0 && panel.contains(origin) && panel.contains(release)
}

// ------------------------------------------------------ math check screen --

/// Every construct the renderer claims to handle, on one sheet.
///
/// Kept in the binary rather than in a test: what can go wrong here — a glyph
/// the font does not have, a numerator overlapping its bar — is visual, and
/// only a picture shows it. `tools/shot.sh` captures this alongside the real
/// screens, so a regression shows up as a diff.
const MATH_SAMPLES: &[(&str, &str)] = &[
    (
        "second-order",
        r"$G(s) = \frac{\omega_0^2}{s^2 + 2\zeta\omega_0 s + \omega_0^2}$",
    ),
    (
        "settling time",
        r"$t_{se} \approx \frac{3}{\zeta\omega_0}$ at the 5 % band",
    ),
    (
        "roots",
        r"$s_{1,2} = -\zeta\omega_0 \pm \omega_0\sqrt{\zeta^2 - 1}$",
    ),
    ("nested", r"$\frac{1}{1 + \frac{K}{s(1 + sT)}}$"),
    (
        "fences",
        r"$\left| \frac{a + b}{c} \right| \le \left( 1 + \sqrt{2} \right)^n$",
    ),
    ("limits", r"$e_{ss} = \lim_{s \to 0} s \cdot E(s)$"),
    ("sum", r"$\sum_{k=0}^{n} a_k s^k = 0$"),
    ("integral", r"$\int_0^\infty e^{-st}f(t)\,dt$"),
    (
        "iterated",
        r"$\iint_D f(x,y)\,dA = \iiint_V \rho(x,y,z)\,dV$",
    ),
    (
        "number sets",
        r"$f: \mathbb{R}^2 \to \mathbb{R}, \quad n \in \mathbb{N}, \ z \in \mathbb{C}$",
    ),
    (
        "accents",
        r"$\dot{x} = Ax + Bu, \quad \ddot{x}, \hat{y}, \bar{u}, \vec{v}$",
    ),
    (
        "matrix",
        r"$\begin{pmatrix} 0 & 1 \\ -\frac{k}{m} & -\frac{d}{m} \end{pmatrix}$",
    ),
    (
        "cases",
        r"$u(t) = \begin{cases} 0 & t < 0 \\ 1 & t \ge 0 \end{cases}$",
    ),
    (
        "greek",
        r"$\alpha\beta\gamma\delta\varepsilon\zeta\eta\theta\lambda\mu\pi\sigma\tau\varphi\psi\omega\ \Delta\Phi\Omega$",
    ),
    (
        "relations",
        r"$a \le b \ne c \approx d \equiv e \propto f \in G \Rightarrow H$",
    ),
    (
        "degrees",
        r"$\varphi_m \approx 100\zeta\ \text{degrees}, \quad \zeta \approx 0.01\varphi_m$",
    ),
    ("unknown", r"$\notacommand{x} + 1$"),
];

fn math_check(ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    let p = ui.painter();
    p.text(
        full.left_top() + Vec2::new(24.0, 16.0),
        Align2::LEFT_TOP,
        tracked("math renderer"),
        text::label(),
        Palette::ACCENT,
    );

    let mut y = full.top() + 44.0;
    let wrap = full.width() - 230.0;
    for (name, src) in MATH_SAMPLES {
        let doc = richtext::layout(p, src, 16.0, wrap);
        p.text(
            Pos2::new(full.left() + 24.0, y + 2.0),
            Align2::LEFT_TOP,
            tracked(name),
            text::label(),
            Palette::TEXT_FAINT,
        );
        doc.paint(p, Pos2::new(full.left() + 200.0, y), Palette::TEXT, 1.0);
        y += doc.height().max(20.0) + 12.0;
        p.line_segment(
            [
                Pos2::new(full.left() + 24.0, y - 6.0),
                Pos2::new(full.right() - 24.0, y - 6.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );
    }
}

// ---------------------------------------------------------- review screen --

impl App {
    /// A card you have already answered, opened again on purpose.
    ///
    /// Everything is here at once — the prompt, what you answered, what was
    /// right, and the explanation — because the reason to come back to a card
    /// is that the two-second version did not land the first time.
    fn review_screen(&mut self, ui: &mut egui::Ui, review: &mut Review) -> Option<Screen> {
        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "review-coin");
        let item = review.items.get(review.idx)?.clone();
        let q = &item.question;

        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(680.0), full.height() - 24.0),
        );

        let colour = match item.grade {
            Some(g) if g.correct => Palette::CORRECT,
            Some(_) => Palette::WRONG,
            None => Palette::LINE_BRIGHT,
        };

        let p = ui.painter();
        p.rect_filled(panel, 0, Palette::SURFACE);
        p.rect_stroke(
            panel,
            0,
            Stroke::new(1.0, Palette::LINE_BRIGHT),
            egui::StrokeKind::Inside,
        );
        p.rect_filled(
            Rect::from_min_size(panel.left_top(), Vec2::new(3.0, panel.height())),
            0,
            colour,
        );

        let topic = q
            .topic_id
            .and_then(|t| review.topics.get(&t))
            .cloned()
            .unwrap_or_default();
        p.text(
            panel.left_top() + Vec2::new(24.0, 18.0),
            Align2::LEFT_TOP,
            tracked(&format!(
                "look back  {}/{}",
                review.idx + 1,
                review.items.len()
            )),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            panel.center_top() + Vec2::new(0.0, 18.0),
            Align2::CENTER_TOP,
            tracked(&topic),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            panel.right_top() + Vec2::new(-24.0, 18.0),
            Align2::RIGHT_TOP,
            tracked(match item.grade {
                Some(g) if g.correct => "you were right",
                Some(_) => "you were wrong",
                None => "not answered yet",
            }),
            text::label(),
            colour,
        );
        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 44.0),
                panel.right_top() + Vec2::new(0.0, 44.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        const FOOT: f32 = 34.0;
        let body_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(24.0, 56.0),
            panel.right_bottom() - Vec2::new(16.0, FOOT),
        );

        let facts = review.facts.clone();
        let depth = review.depth;
        explain::scroll_column(ui, body_rect, "review", |ui| {
            explain::prose(ui, &q.prompt, 18.0, Palette::TEXT);
            ui.add_space(4.0);
            answer_summary(ui, &item);
            ui.add_space(6.0);
            separator(ui);
            explain::body(ui, q, &facts, depth);
        });

        let footer = Rect::from_min_max(
            Pos2::new(panel.left(), panel.bottom() - FOOT),
            panel.right_bottom(),
        );
        let p = ui.painter();
        p.line_segment(
            [footer.left_top(), footer.right_top()],
            Stroke::new(1.0, Palette::LINE),
        );
        p.text(
            footer.left_center() + Vec2::new(24.0, 0.0),
            Align2::LEFT_CENTER,
            tracked("← → other cards  ·  esc back"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            footer.right_center() - Vec2::new(24.0, 0.0),
            Align2::RIGHT_CENTER,
            tracked(depth.label()),
            text::label(),
            Palette::ACCENT,
        );

        let (back, prev, next, deeper) = ui.ctx().input(|i| {
            use egui::Key::*;
            (
                i.key_pressed(Escape) || i.key_pressed(Space) || i.key_pressed(Enter),
                i.key_pressed(ArrowLeft),
                i.key_pressed(ArrowRight),
                i.key_pressed(D),
            )
        });
        if prev {
            review.idx = review.idx.saturating_sub(1);
        }
        if next {
            review.idx = (review.idx + 1).min(review.items.len() - 1);
        }
        if deeper {
            review.depth = review.depth.toggled();
        }
        if back {
            return Some(std::mem::replace(&mut review.back, Screen::Decks));
        }
        None
    }
}

/// What was answered and what was right, in the review view.
fn answer_summary(ui: &mut egui::Ui, item: &Answered) {
    let correct = item.grade.is_some_and(|g| g.correct);
    match (&item.question.body, &item.response) {
        (Body::TrueFalse { answer }, response) => {
            let line = match response {
                Some(Response::TrueFalse { value }) => format!(
                    "you answered {}   ·   the statement is {}",
                    if *value { "TRUE" } else { "FALSE" },
                    if *answer { "TRUE" } else { "FALSE" }
                ),
                _ => format!(
                    "the statement is {}",
                    if *answer { "TRUE" } else { "FALSE" }
                ),
            };
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
            ui.painter().text(
                rect.left_top(),
                Align2::LEFT_TOP,
                line,
                text::small(),
                match item.grade {
                    Some(g) if g.correct => Palette::CORRECT,
                    Some(_) => Palette::WRONG,
                    None => Palette::TEXT_DIM,
                },
            );
        }
        (Body::MultipleChoice { options, .. }, response) => {
            let selected: Vec<usize> = match response {
                Some(Response::MultipleChoice { selected }) => selected.clone(),
                _ => Vec::new(),
            };
            // Here the card is being studied rather than answered, so every
            // note is shown: together they are a map of the mistakes the
            // question was built to catch.
            let notes = explain::option_notes(options, &selected, explain::NoteView::All);
            for (i, opt) in options.iter().enumerate() {
                let chose = selected.contains(&i);
                let colour = match (opt.correct, chose) {
                    (true, _) => Palette::CORRECT,
                    (false, true) => Palette::WRONG,
                    (false, false) => Palette::TEXT_FAINT,
                };
                // A leading mark, so the shape of the answer reads without
                // relying on colour alone.
                let mark = match (opt.correct, chose) {
                    (true, true) => "✓",
                    (true, false) => "·",
                    (false, true) => "✗",
                    (false, false) => " ",
                };
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(18.0, 20.0), Sense::hover());
                    ui.painter().text(
                        rect.left_top(),
                        Align2::LEFT_TOP,
                        mark,
                        text::small(),
                        colour,
                    );
                    explain::prose(ui, &opt.text, 14.5, colour);
                });
                if let Some(note) = notes[i] {
                    ui.horizontal_top(|ui| {
                        ui.add_space(18.0);
                        explain::prose(ui, note, 13.0, colour.gamma_multiply(0.75));
                    });
                }
            }
        }
    }
    let _ = correct;
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, Palette::LINE),
    );
}

// --------------------------------------------------------- summary screen --

impl App {
    fn summary_screen(&mut self, ui: &mut egui::Ui, sum: &mut Summary) -> Option<Screen> {
        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "summary-coin");
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
        p.text(
            panel.right_top() + Vec2::new(0.0, 130.0),
            Align2::RIGHT_TOP,
            tracked("click one to read it"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        // Each row opens the card. A list of things you got wrong that you
        // cannot then look at is a scolding, not a study aid.
        let mut open: Option<usize> = None;
        let mut y = panel.top() + 156.0;
        for (i, w) in sum.weakest.iter().take(7).enumerate() {
            let row = Rect::from_min_size(
                Pos2::new(panel.left(), y - 4.0),
                Vec2::new(panel.width(), 26.0),
            );
            let resp = ui.interact(row, Id::new(("weak", w.question_id)), Sense::click());
            let hot = resp.hovered();
            if resp.clicked() {
                open = Some(i);
            }

            let p = ui.painter();
            if hot {
                p.rect_filled(row, 0, Palette::CARD);
                p.line_segment(
                    [row.left_top(), row.left_bottom()],
                    Stroke::new(2.0, Palette::ACCENT),
                );
            }
            let short: String = w.prompt.chars().take(62).collect();
            p.text(
                Pos2::new(panel.left() + 52.0, y),
                Align2::LEFT_TOP,
                short,
                text::small(),
                if hot {
                    Palette::TEXT
                } else {
                    Palette::TEXT_DIM
                },
            );
            p.text(
                Pos2::new(panel.left() + 8.0, y),
                Align2::LEFT_TOP,
                format!("{:>3.0}%", w.ema * 100.0),
                text::small(),
                if w.ema < 0.4 {
                    Palette::WRONG
                } else {
                    Palette::TEXT_FAINT
                },
            );
            y += 26.0;
        }

        if let Some(i) = open {
            self.open_weakest(sum, i);
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
        if self.pending_review.is_some() {
            return None;
        }

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

    fn clipboard_question() -> Question {
        Question {
            id: 1,
            deck_id: 1,
            topic_id: None,
            uid: "clipboard".into(),
            prompt: r"Which value follows from $G(s)=\frac{1}{s+1}$?".into(),
            body: Body::MultipleChoice {
                options: vec![
                    idiosepius_core::Choice::new(r"$G(0)=1$", true)
                        .with_note("Right — the pole does not contribute at DC."),
                    idiosepius_core::Choice::new(r"$G(0)=0$", false)
                        .with_note("That is $G(\\infty)$, not the DC gain."),
                ],
                multi: false,
            },
            explanation: None,
            explain: idiosepius_core::Explain {
                short: vec![idiosepius_core::Seg::text(
                    r"Set $s=0$ in the transfer function.",
                )],
                deep: Vec::new(),
            },
            difficulty: 1,
            source: None,
            tags: Vec::new(),
        }
    }

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

    #[test]
    fn the_platform_copy_event_is_detected_and_consumed() {
        let mut events = vec![
            egui::Event::Text("unrelated".into()),
            egui::Event::Copy,
            egui::Event::Copy,
        ];

        assert!(take_copy_event(&mut events));
        assert_eq!(events, vec![egui::Event::Text("unrelated".into())]);
        assert!(!take_copy_event(&mut events));
    }

    #[test]
    fn feedback_only_accepts_stationary_clicks_on_its_panel() {
        let panel = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(300.0, 300.0));

        assert!(click_hits_panel(
            Pos2::new(150.0, 150.0),
            Pos2::new(153.0, 151.0),
            panel
        ));
        assert!(!click_hits_panel(
            Pos2::new(50.0, 50.0),
            Pos2::new(50.0, 50.0),
            panel
        ));
        assert!(!click_hits_panel(
            Pos2::new(150.0, 150.0),
            Pos2::new(170.0, 150.0),
            panel
        ));
    }

    #[test]
    fn copying_an_unanswered_card_does_not_leak_the_answer() {
        let text = card_text(
            &clipboard_question(),
            Some("Modeling"),
            &[1],
            None,
            None,
            false,
            explain::NoteView::Hidden,
            None,
        );

        assert!(text.contains(r"$G(s)=\frac{1}{s+1}$"));
        assert!(text.contains("2. $G(0)=0$ [selected]"));
        assert!(!text.contains("Correct answer:"));
        // A note names a wrong option, so it gives the answer away just as
        // surely as the answer line does.
        assert!(!text.contains("Note:"));
    }

    #[test]
    fn feedback_shows_only_the_note_of_the_option_that_was_picked() {
        let question = clipboard_question();
        let facts = Facts::default();
        let text = card_text(
            &question,
            Some("Modeling"),
            &[],
            Some(&Response::MultipleChoice { selected: vec![1] }),
            Some(Grade::WRONG),
            true,
            explain::NoteView::Picked,
            Some((&facts, Depth::Short)),
        );

        assert!(text.contains(r"Note: That is $G(\infty)$, not the DC gain."));
        // The note on the option they did not choose is not their diagnosis.
        assert!(!text.contains("the pole does not contribute at DC"));
    }

    #[test]
    fn review_shows_every_note_because_the_card_is_being_studied() {
        let question = clipboard_question();
        let facts = Facts::default();
        let text = card_text(
            &question,
            Some("Modeling"),
            &[],
            Some(&Response::MultipleChoice { selected: vec![1] }),
            Some(Grade::WRONG),
            true,
            explain::NoteView::All,
            Some((&facts, Depth::Short)),
        );

        assert!(text.contains("the pole does not contribute at DC"));
        assert!(text.contains(r"That is $G(\infty)$, not the DC gain."));
    }

    #[test]
    fn copying_feedback_keeps_latex_and_the_visible_explanation() {
        let question = clipboard_question();
        let facts = Facts::default();
        let text = card_text(
            &question,
            Some("Modeling"),
            &[],
            Some(&Response::MultipleChoice { selected: vec![1] }),
            Some(Grade::WRONG),
            true,
            explain::NoteView::Picked,
            Some((&facts, Depth::Short)),
        );

        assert!(text.contains("My answer: 2. $G(0)=0$"));
        assert!(text.contains("Correct answer: 1. $G(0)=1$"));
        assert!(text.contains("Explanation\nSet $s=0$ in the transfer function."));
    }

    #[test]
    fn explaining_records_a_skip_without_creating_an_attempt() {
        let context = egui::Context::default();
        let store = Store::open_in_memory().unwrap();
        let deck_id = store
            .upsert_deck("clipboard", "Clipboard", None, None)
            .unwrap();
        store
            .upsert_question(&idiosepius_core::NewQuestion {
                deck_id,
                topic_id: None,
                uid: "explain".into(),
                prompt: "The statement is true.".into(),
                body: Body::TrueFalse { answer: true },
                explanation: Some("Because it is.".into()),
                explain: Default::default(),
                difficulty: 1,
                source: None,
                tags: Vec::new(),
            })
            .unwrap();

        let mut app = App::new(&context, store, None);
        let deck = app.decks[0].clone();
        let Some(Screen::Study(mut study)) = app.begin(deck, Mode::Practice) else {
            panic!("study should start");
        };
        let session_id = study.session.id();

        app.apply(&mut study, Action::Explain);

        assert!(
            study.feedback.as_ref().is_some_and(|feedback| {
                feedback.grade.is_none() && feedback.response.is_none()
            })
        );
        let stat = stats::session_stats(&app.store, session_id).unwrap();
        assert_eq!(stat.skipped, 1);
        assert_eq!(stat.answered, 0);
    }
}
