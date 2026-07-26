//! Screens and interaction.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use idiosepius_core::model::{Deck, Lesson, LessonBlock, Topic};
use idiosepius_core::session::{Event, Outcome};
use idiosepius_core::{
    Body, ContentBlock, FactKind, Figure, Grade, Input, Mode, Question, Response, Session, Store,
    content_transcript, now_ms, scheduler, stats,
};

use crate::blocks;
use crate::card::{self, Motion};
use crate::coin::CoinAnimation;
use crate::explain::{self, Depth, Facts};
use crate::import_dialog::{self, ImportAction, ImportView};
use crate::plot;
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
    /// The import chooser is shared; only carrying out its request belongs to
    /// the native or browser shell.
    import_view: Option<ImportView>,
    github_url: String,
    import_error: Option<String>,
    repository_return_view: ImportView,
    /// A review asked for this frame, waiting for the screen it came from.
    pending_review: Option<Box<Review>>,
    /// A question-bank row asked to become a one-card study session. The
    /// current screen is attached as its return destination after rendering.
    pending_question: Option<(Deck, Question)>,
    /// A lesson's authored practice sequence, waiting for the current lesson
    /// screen to be attached as its return destination.
    pending_lesson_practice: Option<(Deck, Vec<Question>)>,
    /// A plot opened from any prompt or explanation at a readable size.
    plot_zoom: Option<Figure>,
    /// Back operations asked for by the browser's History API. They are
    /// applied through the same route as the on-screen button and Escape.
    pending_back: usize,
    /// Development aid: render a few frames, save a PNG, quit. Lets the UI be
    /// checked headlessly (under Xvfb, in CI) instead of by eye.
    shot: Option<Shot>,
}

/// Something only the shell around the app can do: put a file picker on
/// screen, fetch a public repository, or hand a file back to the user.
///
/// The shared chooser records the ask and a native or browser shell carries it
/// out, because their I/O mechanisms have nothing in common but the intent.
#[derive(Debug)]
pub(crate) enum Request {
    ImportLocalDeck,
    ImportGithub(String),
    ExportDatabase,
}

/// What a native file dialog came back with.
#[cfg(not(target_arch = "wasm32"))]
enum Picked {
    Decks(Vec<std::path::PathBuf>),
    Repository(Result<Vec<crate::import::PickedFile>, String>, ImportView),
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
    Course(Deck),
    Lessons(Box<Lessons>),
    Questions(Box<QuestionBank>),
    Progress(Box<Progress>),
    /// A sheet of formulas, for checking the renderer. Reachable only through
    /// `--shot --screen math`: a missing glyph or a fraction sitting a pixel
    /// off is not something to discover on a card the night before an exam.
    MathCheck,
    /// Transfer-function plot reference sheet for reproducible visual checks.
    PlotCheck,
    Study(Box<Study>),
    Summary(Summary),
    /// Looking back at a card that has already been answered. Holds the screen
    /// it was opened from, so closing it returns exactly where you were —
    /// including a study session that is still running.
    Review(Box<Review>),
}

struct Lessons {
    deck: Deck,
    lessons: Vec<Lesson>,
    topics: HashMap<i64, String>,
    read: HashSet<i64>,
    selected: Option<usize>,
    facts: Rc<Facts>,
    /// Screenshot-only initial position; normal navigation leaves this empty.
    initial_scroll: Option<f32>,
}

struct QuestionBank {
    deck: Deck,
    questions: Vec<Question>,
    topics: HashMap<i64, String>,
    latest_results: HashMap<i64, bool>,
    filters: QuestionFilters,
    collapsed: HashSet<Option<i64>>,
    /// Screenshot-only initial position; normal navigation leaves this empty.
    initial_scroll: Option<f32>,
}

#[derive(Clone, Copy)]
struct QuestionFilters {
    correct: bool,
    incorrect: bool,
    unattempted: bool,
}

impl Default for QuestionFilters {
    fn default() -> Self {
        Self {
            correct: true,
            incorrect: true,
            unattempted: true,
        }
    }
}

impl QuestionFilters {
    fn includes(self, result: Option<bool>) -> bool {
        match result {
            Some(true) => self.correct,
            Some(false) => self.incorrect,
            None => self.unattempted,
        }
    }
}

struct Progress {
    deck: Deck,
    deck_stats: stats::DeckStats,
    topics: Vec<stats::TopicStat>,
    weakest: Vec<stats::WeakQuestion>,
    topic_names: HashMap<i64, String>,
    facts: Rc<Facts>,
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
    route: StudyRoute,
    /// Where the pointer grabbed the card, in card-local coordinates.
    grab: Option<Vec2>,
}

enum StudyRoute {
    Scheduled,
    Single {
        back: Box<Screen>,
    },
    Lesson {
        back: Box<Screen>,
        remaining: Vec<Question>,
    },
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

fn screen_depth(screen: &Screen) -> usize {
    match screen {
        Screen::Decks | Screen::MathCheck | Screen::PlotCheck => 0,
        Screen::Course(_) | Screen::Summary(_) => 1,
        Screen::Lessons(lessons) => 2 + usize::from(lessons.selected.is_some()),
        Screen::Questions(_) | Screen::Progress(_) => 2,
        Screen::Study(study) => match &study.route {
            StudyRoute::Scheduled => 1,
            StudyRoute::Single { back } | StudyRoute::Lesson { back, .. } => screen_depth(back) + 1,
        },
        Screen::Review(review) => screen_depth(&review.back) + 1,
    }
}

fn screen_back_button(ui: &egui::Ui, full: Rect) -> bool {
    let rect = Rect::from_min_size(
        full.right_top() + Vec2::new(-52.0, 8.0),
        Vec2::new(44.0, 44.0),
    );
    let response = ui
        .interact(rect, Id::new("screen-back"), Sense::click())
        .on_hover_text("Back");
    let hot = response.hovered() || response.has_focus();
    let colour = if hot {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    ui.painter()
        .rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    ui.painter()
        .rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "←",
        text::title(),
        colour,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Back"));
    response.clicked()
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
            import_view: None,
            github_url: String::new(),
            import_error: None,
            repository_return_view: ImportView::Github,
            pending_review: None,
            pending_question: None,
            pending_lesson_practice: None,
            plot_zoom: None,
            pending_back: 0,
            shot,
        };

        // Jump straight to the screen being captured.
        let screen_name = app.shot.as_ref().and_then(|s| s.screen.clone());
        let screen_name = screen_name.as_deref();
        match screen_name {
            None | Some("decks" | "decks-scroll") => {}
            Some("math") => app.screen = Screen::MathCheck,
            Some("plots") => app.screen = Screen::PlotCheck,
            Some("plot-zoom") => {
                app.screen = Screen::PlotCheck;
                app.plot_zoom = Some(nyquist_check_figure());
            }
            Some("course") => {
                if let Some(deck) = app.decks.first().cloned() {
                    app.screen = Screen::Course(deck);
                }
            }
            Some("lessons") => {
                if let Some(deck) = app.decks.first().cloned() {
                    if let Some(screen) = app.lessons(deck) {
                        app.screen = screen;
                    }
                }
            }
            Some("lesson" | "lesson-end") => {
                if let Some(deck) = app.decks.first().cloned()
                    && let Some(mut screen) = app.lessons(deck)
                {
                    if let Screen::Lessons(lessons) = &mut screen
                        && !lessons.lessons.is_empty()
                    {
                        // `--card` pins a lesson capture the same way it pins
                        // a question: by uid, so a screenshot of one reading
                        // stays that reading when the pack grows.
                        let wanted = app.shot.as_ref().and_then(|shot| shot.card.as_deref());
                        lessons.selected = Some(
                            wanted
                                .and_then(|uid| lessons.lessons.iter().position(|l| l.uid == uid))
                                .unwrap_or(0),
                        );
                        // The glossary and the practice row are at the foot of
                        // a reading, so one capture has to start there; on a
                        // reading proper, `--drag` scrolls to a figure the
                        // same way it freezes a swipe — both are pixels.
                        lessons.initial_scroll = match screen_name {
                            Some("lesson-end") => Some(f32::MAX),
                            _ => app
                                .shot
                                .as_ref()
                                .map(|shot| shot.drag)
                                .filter(|drag| *drag > 0.0),
                        };
                    }
                    app.screen = screen;
                }
            }
            Some("questions" | "questions-scroll" | "questions-collapsed") => {
                if let Some(deck) = app.decks.first().cloned()
                    && let Some(mut screen) = app.question_bank(deck)
                {
                    if let Screen::Questions(bank) = &mut screen {
                        match app.shot.as_ref().and_then(|shot| shot.screen.as_deref()) {
                            Some("questions-scroll") => bank.initial_scroll = Some(620.0),
                            Some("questions-collapsed") => {
                                bank.collapsed.insert(
                                    bank.questions
                                        .first()
                                        .and_then(|question| question.topic_id),
                                );
                            }
                            _ => {}
                        }
                    }
                    app.screen = screen;
                }
            }
            Some("progress") => {
                if let Some(deck) = app.decks.first().cloned()
                    && let Some(screen) = app.progress(deck)
                {
                    app.screen = screen;
                }
            }
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

    pub(crate) fn import_picked_files(&mut self, files: Vec<crate::import::PickedFile>) {
        match crate::import::decode_packs(files).and_then(|pack| self.import_pack(&pack)) {
            Ok(report) => {
                self.notify(format!(
                    "imported {} questions · {} lessons",
                    report.questions, report.lessons
                ));
                self.error = None;
                self.import_view = None;
                self.import_error = None;
            }
            Err(error) => {
                if self.import_view == Some(ImportView::Loading) {
                    self.import_view = Some(self.repository_return_view);
                    self.import_error = Some(format!("Deck import failed: {error:#}"));
                } else {
                    self.report_error(format!("deck import failed: {error:#}"));
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn repository_import_failed(&mut self, error: impl Into<String>) {
        self.import_view = Some(self.repository_return_view);
        self.import_error = Some(error.into());
    }

    pub(crate) fn export_database(&self) -> anyhow::Result<Vec<u8>> {
        self.store.export_database()
    }

    /// Take the shell action a screen asked for this frame, if any.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn take_request(&mut self) -> Option<Request> {
        self.request.take()
    }

    /// Number of browser-history steps represented by the current view.
    ///
    /// This is intentionally about navigation, not enum nesting: ending a
    /// scheduled study session replaces it with its summary at the same depth,
    /// while opening a lesson reader adds a step inside `Screen::Lessons`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn navigation_depth(&self) -> usize {
        let overlay = match self.import_view {
            Some(ImportView::Sources) => 1,
            Some(ImportView::Examples | ImportView::Github | ImportView::Loading) => 2,
            None if self.plot_zoom.is_some() => 1,
            None => 0,
        };
        screen_depth(&self.screen) + overlay
    }

    /// Queue browser Back presses for the next egui frame.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn request_back(&mut self, steps: usize) {
        self.pending_back = self.pending_back.saturating_add(steps);
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

    fn show_import_dialog(&mut self, ctx: &egui::Context) {
        let Some(view) = self.import_view else {
            return;
        };
        let action = import_dialog::show(
            ctx,
            view,
            &mut self.github_url,
            self.import_error.as_deref(),
        );
        match action {
            ImportAction::None => {}
            ImportAction::Close => {
                self.import_view = None;
                self.import_error = None;
            }
            ImportAction::LocalFiles => {
                self.import_view = None;
                self.import_error = None;
                self.request = Some(Request::ImportLocalDeck);
            }
            ImportAction::Show(view) => {
                self.import_view = Some(view);
                self.import_error = None;
            }
            ImportAction::LoadRepository(url, return_to) => {
                self.import_view = Some(ImportView::Loading);
                self.import_error = None;
                self.repository_return_view = return_to;
                self.request = Some(Request::ImportGithub(url));
            }
        }
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

        // Let Escape close the plot modal without also navigating the screen
        // visible behind it.
        let close_plot = if self.plot_zoom.is_some() {
            ui.ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        } else {
            false
        };
        let escape_back = if self.plot_zoom.is_none() {
            ui.ctx()
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        } else {
            false
        };

        // Move the screen out of `self` for the duration of the frame. The
        // screen owns a `Session`, which needs `&mut`, while the handlers also
        // need `&mut self` for the store and error slot; splitting them is
        // simpler than threading a borrow through every call.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Decks);

        let next = match &mut screen {
            Screen::Decks => self.deck_screen(ui),
            Screen::Course(deck) => self.course_screen(ui, deck),
            Screen::Lessons(lessons) => self.lessons_screen(ui, lessons),
            Screen::Questions(bank) => self.questions_screen(ui, bank),
            Screen::Progress(progress) => self.progress_screen(ui, progress),
            Screen::MathCheck => {
                math_check(ui);
                None
            }
            Screen::PlotCheck => {
                plot_check(ui);
                None
            }
            Screen::Study(study) => self.study_screen(ui, study),
            Screen::Summary(sum) => self.summary_screen(ui, sum),
            Screen::Review(review) => self.review_screen(ui, review),
        };
        let mut screen = next.unwrap_or(screen);
        let touch_back = screen_depth(&screen) > 0
            && self.import_view.is_none()
            && self.plot_zoom.is_none()
            && screen_back_button(ui, ui.max_rect());

        // Opening a review needs the screen it was opened from, which only
        // exists here: the handler that asked for it is holding a borrow of it.
        if let Some(mut review) = self.pending_review.take() {
            review.back = screen;
            screen = Screen::Review(review);
        }
        if let Some((deck, question)) = self.pending_question.take() {
            screen = self.begin_single(deck, question, screen);
        }
        if let Some((deck, questions)) = self.pending_lesson_practice.take() {
            screen = self.begin_lesson_practice(deck, questions, screen);
        }

        let mut back_steps = std::mem::take(&mut self.pending_back);
        if escape_back || touch_back {
            back_steps = back_steps.saturating_add(1);
        }
        for _ in 0..back_steps {
            if !self.back_once(&mut screen) {
                break;
            }
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

        if let Some(figure) = blocks::take_zoom_request(ui.ctx()) {
            self.plot_zoom = Some(figure);
        }
        self.show_plot_zoom(ui.ctx(), close_plot);

        self.show_import_dialog(ui.ctx());

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

impl App {
    /// Apply one logical Back operation, regardless of whether it came from
    /// Escape, the touch button, or browser history.
    fn back_once(&mut self, screen: &mut Screen) -> bool {
        match self.import_view {
            Some(ImportView::Sources) => {
                self.import_view = None;
                self.import_error = None;
                return true;
            }
            Some(ImportView::Examples | ImportView::Github) => {
                self.import_view = Some(ImportView::Sources);
                self.import_error = None;
                return true;
            }
            // A repository request cannot be cancelled once fetch has begun.
            Some(ImportView::Loading) => return false,
            None => {}
        }
        if self.plot_zoom.take().is_some() {
            return true;
        }

        let next = match screen {
            Screen::Decks => None,
            Screen::Course(_) => Some(Screen::Decks),
            Screen::Lessons(lessons) if lessons.selected.is_some() => {
                lessons.selected = None;
                return true;
            }
            Screen::Lessons(lessons) => Some(Screen::Course(lessons.deck.clone())),
            Screen::Questions(bank) => Some(Screen::Course(bank.deck.clone())),
            Screen::Progress(progress) => Some(Screen::Course(progress.deck.clone())),
            Screen::Study(study) => self.apply(study, Action::Quit),
            Screen::Summary(_) => {
                self.decks = self.store.decks().unwrap_or_default();
                Some(Screen::Decks)
            }
            Screen::Review(review) => Some(std::mem::replace(&mut review.back, Screen::Decks)),
            Screen::MathCheck | Screen::PlotCheck => Some(Screen::Decks),
        };
        if let Some(next) = next {
            *screen = next;
            true
        } else {
            false
        }
    }

    fn show_plot_zoom(&mut self, ctx: &egui::Context, close_requested: bool) {
        let Some(figure) = self.plot_zoom.clone() else {
            return;
        };
        let width = (ctx.content_rect().width() - 96.0).clamp(280.0, 920.0);
        let height = (ctx.content_rect().height() - 190.0).clamp(220.0, 620.0);
        let frame = egui::Frame::new()
            .inner_margin(24)
            .fill(Palette::SURFACE)
            .stroke(Stroke::new(1.0, Palette::LINE_BRIGHT));
        let modal = egui::Modal::new(Id::new("plot-zoom"))
            .backdrop_color(Palette::BG.gamma_multiply(0.86))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.colored_label(Palette::TEXT_FAINT, tracked("enlarged plot"));
                ui.add_space(8.0);
                let enlarged = plot::layout_large(ui.painter(), &figure, width, height);
                let (rect, _) = ui.allocate_exact_size(enlarged.size, Sense::hover());
                enlarged.paint_rotated(ui.painter(), rect.min, rect.min, 0.0, 1.0);
                ui.add_space(8.0);
                ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::Button::new(tracked("close")),
                )
                .clicked()
            });
        if close_requested || modal.should_close() || modal.inner {
            self.plot_zoom = None;
        }
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
        match request {
            Request::ImportLocalDeck => self.start_deck_dialog(ctx),
            Request::ImportGithub(url) => self.start_repository_import(ctx, url),
            Request::ExportDatabase => self.start_export_dialog(ctx),
        }
    }

    fn start_deck_dialog(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("idiosepius-file-dialog".into())
            .spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title("Import deck packs")
                    .add_filter("deck packs", &["json", "zip"])
                    .pick_files()
                    .map_or(Picked::Nothing, Picked::Decks);
                let _ = tx.send(picked);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.dialog = Some(rx),
            Err(error) => self.error = Some(format!("could not open a file dialog: {error}")),
        }
    }

    fn start_export_dialog(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let suggested = default_export_name();
        let spawned = std::thread::Builder::new()
            .name("idiosepius-file-dialog".into())
            .spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title("Export study database")
                    .add_filter("SQLite database", &["db"])
                    .set_file_name(suggested)
                    .save_file()
                    .map_or(Picked::Nothing, Picked::ExportTo);
                let _ = tx.send(picked);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.dialog = Some(rx),
            Err(error) => self.error = Some(format!("could not open a file dialog: {error}")),
        }
    }

    fn start_repository_import(&mut self, ctx: &egui::Context, url: String) {
        if self.dialog.is_some() {
            return;
        }
        let return_to = self.repository_return_view;
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("idiosepius-github-import".into())
            .spawn(move || {
                let result = crate::native_github::load_repository(&url)
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send(Picked::Repository(result, return_to));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.dialog = Some(rx),
            Err(error) => {
                self.import_view = Some(return_to);
                self.import_error =
                    Some(format!("could not start GitHub repository import: {error}"));
            }
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
            Picked::Repository(Ok(files), _) => self.import_picked_files(files),
            Picked::Repository(Err(error), return_to) => {
                self.repository_return_view = return_to;
                self.import_view = Some(return_to);
                self.import_error = Some(error);
            }
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

        self.import_picked_files(files);
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
            Screen::Course(deck) => format!(
                "{}\n\nLessons\nQuestions\nProgress\n\nAll active questions are available for study.",
                deck.title
            ),
            Screen::Lessons(screen) => {
                match screen.selected.and_then(|index| screen.lessons.get(index)) {
                    Some(lesson) => {
                        let topic = screen
                            .topics
                            .get(&lesson.topic_id)
                            .map(String::as_str)
                            .unwrap_or("Uncategorised");
                        let mut out = format!(
                            "{}\n\n{}\n{}\n\n{}",
                            screen.deck.title,
                            topic,
                            lesson.title,
                            lesson_text(lesson, &screen.facts)
                        );
                        if let Some(source) = &lesson.source {
                            let _ = write!(out, "\n\nSource: {source}");
                        }
                        if !lesson.practice.is_empty() {
                            let _ =
                                write!(out, "\n\nPractice\n{} questions", lesson.practice.len());
                        }
                        out
                    }
                    None => {
                        let mut out = format!("{}\n\nLessons", screen.deck.title);
                        if screen.lessons.is_empty() {
                            out.push_str("\n\nNo lessons have been authored for this deck yet.");
                        }
                        for lesson in &screen.lessons {
                            let mark = if screen.read.contains(&lesson.id) {
                                "Read"
                            } else {
                                "Unread"
                            };
                            let _ = write!(
                                out,
                                "\n\n{}\n{}\n{} · {} questions",
                                lesson.title,
                                lesson.summary,
                                mark,
                                lesson.practice.len()
                            );
                        }
                        out
                    }
                }
            }
            Screen::Questions(bank) => {
                let mut out = format!(
                    "{}\n\nQuestion bank\n{} questions",
                    bank.deck.title,
                    bank.questions.len()
                );
                for question in &bank.questions {
                    let result = bank.latest_results.get(&question.id).copied();
                    if !bank.filters.includes(result) || bank.collapsed.contains(&question.topic_id)
                    {
                        continue;
                    }
                    let topic = question
                        .topic_id
                        .and_then(|id| bank.topics.get(&id))
                        .map(String::as_str)
                        .unwrap_or("Uncategorised");
                    let status = match result {
                        Some(true) => "Correct",
                        Some(false) => "Incorrect",
                        None => "Not yet attempted",
                    };
                    let _ = write!(
                        out,
                        "\n\n{}\n{} · {}\n{}",
                        topic,
                        question.uid,
                        status,
                        question.prompt_text()
                    );
                }
                out
            }
            Screen::Progress(progress) => {
                let stat = &progress.deck_stats;
                let mut out = format!(
                    "{}\n\nProgress\nReadiness: {:.0}%\nAccuracy: {:.0}%\n{} of {} questions attempted",
                    progress.deck.title,
                    stat.readiness * 100.0,
                    stat.accuracy * 100.0,
                    stat.attempted,
                    stat.questions
                );
                for topic in &progress.topics {
                    let _ = write!(
                        out,
                        "\n\n{}\n{} questions · {:.0}% accuracy · {} solid",
                        topic.title,
                        topic.questions,
                        topic.accuracy * 100.0,
                        topic.solid
                    );
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
            Screen::PlotCheck => "Plot renderer\n\nBode plot\n\nNyquist plot".into(),
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
        let scroll_to_end =
            self.shot.as_ref().and_then(|shot| shot.screen.as_deref()) == Some("decks-scroll");

        // Keep a little page around the instrument on ordinary and large
        // windows, but let the panel fill genuinely short viewports. The old
        // fixed 560 pt cap made a tall monitor scroll a list that had ample
        // room available below it.
        let panel_height = if avail.height() >= 600.0 {
            (avail.height() - 40.0).min(840.0)
        } else {
            avail.height()
        };
        let panel = Rect::from_center_size(
            avail.center(),
            Vec2::new(avail.width().min(660.0), panel_height),
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

        // On a short window the panel reaches the viewport edge, so use the
        // slightly smaller rect that still fits above the rule. Elsewhere the
        // full-size coin moves up just enough for its ink to clear the rule.
        let (coin_size, coin_y) = if panel.top() <= avail.top() + 1.0 {
            (60.0, 30.0)
        } else {
            (68.0, 28.0)
        };
        let coin_rect = Rect::from_center_size(
            panel.right_top() + Vec2::new(-38.0, coin_y),
            Vec2::splat(coin_size),
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

        let decks = self.decks.clone();
        let list_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 78.0),
            panel.right_bottom() - Vec2::new(0.0, 82.0),
        );
        if list_rect.is_positive() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
                ui.set_clip_rect(list_rect);
                ui.spacing_mut().scroll.bar_width = 5.0;
                ui.spacing_mut().scroll.floating = false;
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("deck-list")
                    .auto_shrink([false, false]);
                scroll.show(ui, |ui| {
                        // This list owns its gaps explicitly. Inheriting the
                        // global item spacing here used to add a second,
                        // invisible layer of padding after every deck.
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.set_width(list_rect.width() - 10.0);
                        if decks.is_empty() {
                            let (empty, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 42.0),
                                Sense::hover(),
                            );
                            ui.painter().text(
                                empty.left_top(),
                                Align2::LEFT_TOP,
                                "No decks yet. Import a pack — one or more .json files, or a .zip of them.",
                                text::body(),
                                Palette::TEXT_DIM,
                            );
                        }
                        for deck in &decks {
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 104.0),
                                Sense::hover(),
                            );
                            let counts =
                                scheduler::counts(&self.store, deck.id).unwrap_or_default();
                            let stat =
                                stats::deck_stats(&self.store, deck.id).unwrap_or_default();
                            match deck_row(ui, row, deck, counts, stat) {
                                DeckRowAction::Open => {
                                    next = Some(Screen::Course(deck.clone()));
                                }
                                DeckRowAction::Shuffle => {
                                    next = self.begin(deck.clone(), Mode::Practice);
                                }
                                DeckRowAction::None => {}
                            }
                            ui.add_space(10.0);
                        }

                        // Importing scrolls with the decks because it produces
                        // another member of this list.
                        let (import, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 54.0),
                            Sense::hover(),
                        );
                        if dashed_row(ui, import, "import deck") {
                            self.import_view = Some(ImportView::Sources);
                            self.import_error = None;
                        }
                        // A stroke centred exactly on the scroll content's
                        // final edge loses its lower half to clipping. This
                        // also gives the last action a little landing room
                        // when the list genuinely has to scroll.
                        ui.add_space(8.0);
                        if scroll_to_end {
                            let reveal = Rect::from_min_max(
                                import.min,
                                Pos2::new(import.right(), import.bottom() + 8.0),
                            );
                            ui.scroll_to_rect(reveal, Some(egui::Align::BOTTOM));
                        }
                    });
            });
        }

        // The database is the whole course and the whole history, so taking a
        // copy of it belongs on the screen that lists what is in it — at the
        // bottom, out of the way of the decks themselves.
        let export = Pos2::new(panel.right(), panel.bottom() - 80.0);
        if chrome_button(ui, export, "export database") {
            self.request = Some(Request::ExportDatabase);
        }

        ui.painter().text(
            Pos2::new(panel.left(), panel.bottom() - 18.0),
            Align2::LEFT_BOTTOM,
            tracked("open / shuffle"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        if panel.width() >= 620.0 {
            ui.painter().text(
                Pos2::new(panel.right(), panel.bottom() - 18.0),
                Align2::RIGHT_BOTTOM,
                tracked("ctrl ± size  ·  ctrl 0 reset"),
                text::label(),
                Palette::TEXT_FAINT,
            );
        }

        next
    }

    fn course_screen(&mut self, ui: &mut egui::Ui, deck: &Deck) -> Option<Screen> {
        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "course-coin");
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(660.0), full.height().min(580.0)),
        );
        let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();
        let stat = stats::deck_stats(&self.store, deck.id).unwrap_or_default();
        let lesson_count = self.store.lesson_count(deck.id).unwrap_or(0);
        let p = ui.painter();
        let header_x = header_left(full, panel);

        p.text(
            Pos2::new(header_x, panel.top()),
            Align2::LEFT_TOP,
            tracked("course"),
            text::label(),
            Palette::TEXT_FAINT,
        );
        p.text(
            Pos2::new(header_x, panel.top() + 26.0),
            Align2::LEFT_TOP,
            &deck.title,
            text::title(),
            Palette::TEXT,
        );
        if panel.width() >= 560.0 {
            p.text(
                Pos2::new(panel.right().min(full.right() - 64.0), panel.top() + 20.0),
                Align2::RIGHT_TOP,
                format!("{:.0}%", stat.readiness * 100.0),
                text::number(),
                if stat.readiness > 0.6 {
                    Palette::CORRECT
                } else {
                    Palette::TEXT_DIM
                },
            );
        }
        p.text(
            Pos2::new(header_x, panel.top() + 60.0),
            Align2::LEFT_TOP,
            format!(
                "{} questions   {} new   {} due",
                counts.total, counts.fresh, counts.due
            ),
            text::small(),
            Palette::TEXT_DIM,
        );
        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 88.0),
                panel.right_top() + Vec2::new(0.0, 88.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        let destinations = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 100.0),
            panel.right_bottom() - Vec2::new(0.0, 44.0),
        );
        let mut destination = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(destinations), |ui| {
            ui.set_clip_rect(destinations);
            egui::ScrollArea::vertical()
                .id_salt(("course", deck.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(destinations.width() - 8.0);
                    for (name, description, meta) in [
                        (
                            "lessons",
                            "Learn or revisit the course in order.",
                            format!("{lesson_count} readings"),
                        ),
                        (
                            "questions",
                            "Browse the complete unlocked question bank.",
                            format!("{} available", counts.total),
                        ),
                        (
                            "progress",
                            "Readiness, topic accuracy and weak questions.",
                            format!("{} attempted", stat.attempted),
                        ),
                    ] {
                        let (row, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 86.0),
                            Sense::hover(),
                        );
                        if navigation_row(
                            ui,
                            row,
                            ("course", deck.id, name),
                            name,
                            description,
                            &meta,
                        ) {
                            destination = Some(name);
                        }
                        ui.add_space(12.0);
                    }
                });
        });
        match destination {
            Some("lessons") => return self.lessons(deck.clone()),
            Some("questions") => return self.question_bank(deck.clone()),
            Some("progress") => return self.progress(deck.clone()),
            _ => {}
        }

        ui.painter().text(
            panel.left_bottom(),
            Align2::LEFT_BOTTOM,
            tracked("back is at top right"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        None
    }

    fn lessons(&mut self, deck: Deck) -> Option<Screen> {
        let lessons = match self.store.lessons(deck.id) {
            Ok(lessons) => lessons,
            Err(error) => {
                self.error = Some(format!("could not load lessons: {error}"));
                return None;
            }
        };
        let topics = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|topic| (topic.id, topic.title))
            .collect();
        let read = self.store.read_lesson_ids(deck.id).unwrap_or_default();
        let facts = Rc::new(Facts::load(&self.store, deck.id));
        Some(Screen::Lessons(Box::new(Lessons {
            deck,
            lessons,
            topics,
            read,
            selected: None,
            facts,
            initial_scroll: None,
        })))
    }

    fn lessons_screen(&mut self, ui: &mut egui::Ui, screen: &mut Lessons) -> Option<Screen> {
        if screen.selected.is_some() {
            return self.lesson_reader(ui, screen);
        }

        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "lessons-coin");
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(720.0), full.height() - 24.0),
        );
        let read_count = screen
            .lessons
            .iter()
            .filter(|lesson| screen.read.contains(&lesson.id))
            .count();
        let header_x = header_left(full, panel);
        {
            let p = ui.painter();
            p.text(
                Pos2::new(header_x, panel.top()),
                Align2::LEFT_TOP,
                tracked("lessons"),
                text::label(),
                Palette::ACCENT,
            );
            p.text(
                Pos2::new(header_x, panel.top() + 28.0),
                Align2::LEFT_TOP,
                &screen.deck.title,
                text::title(),
                Palette::TEXT,
            );
            if panel.width() >= 560.0 {
                p.text(
                    Pos2::new(panel.right().min(full.right() - 64.0), panel.top() + 30.0),
                    Align2::RIGHT_TOP,
                    tracked(&format!("{read_count} / {} read", screen.lessons.len())),
                    text::label(),
                    Palette::TEXT_DIM,
                );
            }
            p.line_segment(
                [
                    panel.left_top() + Vec2::new(0.0, 68.0),
                    panel.right_top() + Vec2::new(0.0, 68.0),
                ],
                Stroke::new(1.0, Palette::LINE),
            );
        }

        let list_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 82.0),
            panel.right_bottom() - Vec2::new(0.0, 28.0),
        );
        let lessons = screen.lessons.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            egui::ScrollArea::vertical()
                .id_salt(("lessons", screen.deck.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(list_rect.width() - 10.0);
                    if lessons.is_empty() {
                        let (empty, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 142.0),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(empty, 0, Palette::SURFACE);
                        ui.painter().rect_stroke(
                            empty,
                            0,
                            Stroke::new(1.0, Palette::LINE),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            empty.left_top() + Vec2::new(22.0, 22.0),
                            Align2::LEFT_TOP,
                            tracked("no lessons yet"),
                            text::label(),
                            Palette::TEXT_FAINT,
                        );
                        ui.painter().text(
                            empty.left_top() + Vec2::new(22.0, 54.0),
                            Align2::LEFT_TOP,
                            "Lessons will appear here as they are authored.",
                            text::body(),
                            Palette::TEXT,
                        );
                        ui.painter().text(
                            empty.left_top() + Vec2::new(22.0, 88.0),
                            Align2::LEFT_TOP,
                            "All questions remain unlocked and available from the course screen.",
                            text::small(),
                            Palette::TEXT_DIM,
                        );
                    }

                    let mut last_topic = None;
                    for (index, lesson) in lessons.iter().enumerate() {
                        if last_topic != Some(lesson.topic_id) {
                            if last_topic.is_some() {
                                ui.add_space(12.0);
                            }
                            let topic = screen
                                .topics
                                .get(&lesson.topic_id)
                                .map(String::as_str)
                                .unwrap_or("Uncategorised");
                            lesson_heading(ui, topic);
                            ui.add_space(6.0);
                            last_topic = Some(lesson.topic_id);
                        }
                        let read = screen.read.contains(&lesson.id);
                        if lesson_row(ui, lesson, read) {
                            screen.selected = Some(index);
                        }
                        ui.add_space(8.0);
                    }
                });
        });
        ui.painter().text(
            panel.left_bottom(),
            Align2::LEFT_BOTTOM,
            tracked("back is at top right"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        None
    }

    fn lesson_reader(&mut self, ui: &mut egui::Ui, screen: &mut Lessons) -> Option<Screen> {
        let index = screen.selected?;
        let lesson = screen.lessons.get(index)?.clone();
        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "lesson-reader-coin");
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(760.0), full.height() - 24.0),
        );
        let topic = screen
            .topics
            .get(&lesson.topic_id)
            .map(String::as_str)
            .unwrap_or("Uncategorised");
        let header_x = header_left(full, panel);
        {
            let p = ui.painter();
            p.text(
                Pos2::new(header_x, panel.top()),
                Align2::LEFT_TOP,
                tracked(topic),
                text::label(),
                Palette::ACCENT,
            );
            p.text(
                Pos2::new(header_x, panel.top() + 26.0),
                Align2::LEFT_TOP,
                &lesson.title,
                text::title(),
                Palette::TEXT,
            );
            p.line_segment(
                [
                    panel.left_top() + Vec2::new(0.0, 68.0),
                    panel.right_top() + Vec2::new(0.0, 68.0),
                ],
                Stroke::new(1.0, Palette::LINE),
            );
        }

        let body_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 80.0),
            panel.right_bottom() - Vec2::new(0.0, 28.0),
        );
        let requested_scroll = screen.initial_scroll.take();
        ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
            ui.set_clip_rect(body_rect);
            egui::ScrollArea::vertical()
                .id_salt(("lesson-reader", lesson.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(offset) = requested_scroll {
                        ui.scroll_with_delta(Vec2::new(0.0, -offset));
                    }
                    ui.set_width(body_rect.width() - 12.0);
                    explain::prose(ui, &lesson.summary, 15.0, Palette::TEXT_DIM);
                    ui.add_space(14.0);
                    for block in &lesson.body {
                        lesson_block(ui, block, &screen.facts);
                        ui.add_space(12.0);
                    }
                    // The same glossary a deep reading gets: a lesson uses
                    // more notation than any one card does, not less.
                    if explain::glossary(ui, &explain::lesson_symbols(&lesson, &screen.facts)) {
                        ui.add_space(12.0);
                    }
                    if let Some(source) = &lesson.source {
                        separator(ui);
                        ui.add_space(4.0);
                        explain::prose(ui, &format!("Source: {source}"), 11.5, Palette::TEXT_FAINT);
                    }
                    ui.add_space(12.0);
                    let row_width = ui.available_width();
                    let read_rect = ui
                        .allocate_exact_size(Vec2::new(row_width, 68.0), Sense::hover())
                        .0;
                    let is_read = screen.read.contains(&lesson.id);
                    if navigation_row(
                        ui,
                        read_rect,
                        ("lesson-read", lesson.id),
                        if is_read { "read" } else { "mark read" },
                        if is_read {
                            "This reading is recorded in your progress."
                        } else {
                            "Record this reading in the append-only study log."
                        },
                        if is_read { "recorded" } else { "not yet" },
                    ) && !is_read
                    {
                        self.mark_lesson_read(screen, lesson.id);
                    }
                    if !lesson.practice.is_empty() {
                        ui.add_space(8.0);
                        let practice_rect = ui
                            .allocate_exact_size(Vec2::new(row_width, 76.0), Sense::hover())
                            .0;
                        if navigation_row(
                            ui,
                            practice_rect,
                            ("lesson-practice", lesson.id),
                            "practice this lesson",
                            "Study exactly the questions taught here, in authored order.",
                            &format!("{} questions", lesson.practice.len()),
                        ) {
                            match self
                                .store
                                .questions_by_uids(screen.deck.id, &lesson.practice)
                            {
                                Ok(questions) if !questions.is_empty() => {
                                    self.pending_lesson_practice =
                                        Some((screen.deck.clone(), questions));
                                }
                                Ok(_) => {
                                    self.error = Some(
                                        "none of this lesson's practice questions are active"
                                            .into(),
                                    )
                                }
                                Err(error) => {
                                    self.error =
                                        Some(format!("could not load lesson practice: {error}"))
                                }
                            }
                        }
                    }
                    ui.add_space(12.0);
                });
        });
        ui.painter().text(
            panel.left_bottom(),
            Align2::LEFT_BOTTOM,
            tracked("back is at top right"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        None
    }

    fn mark_lesson_read(&mut self, screen: &mut Lessons, lesson_id: i64) {
        let result = (|| -> anyhow::Result<()> {
            let mut session = Session::start(self.store.clone(), screen.deck.id, Mode::Lesson)?;
            session.read_lesson(lesson_id);
            let errors = session.take_errors();
            session.end()?;
            if let Some(error) = errors.into_iter().next() {
                anyhow::bail!(error);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                screen.read.insert(lesson_id);
                self.notify("lesson marked read");
            }
            Err(error) => self.error = Some(format!("could not mark lesson read: {error}")),
        }
    }

    fn question_bank(&mut self, deck: Deck) -> Option<Screen> {
        let questions = match self.store.questions(deck.id) {
            Ok(questions) => questions,
            Err(e) => {
                self.error = Some(format!("could not load questions: {e}"));
                return None;
            }
        };
        let topics = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|topic: Topic| (topic.id, topic.title))
            .collect();
        let latest_results = self
            .store
            .latest_question_results(deck.id)
            .unwrap_or_default();
        Some(Screen::Questions(Box::new(QuestionBank {
            deck,
            questions,
            topics,
            latest_results,
            filters: QuestionFilters::default(),
            collapsed: HashSet::new(),
            initial_scroll: None,
        })))
    }

    fn questions_screen(&mut self, ui: &mut egui::Ui, bank: &mut QuestionBank) -> Option<Screen> {
        const GROUP_HEADER_HEIGHT: f32 = 44.0;

        struct Group {
            topic_id: Option<i64>,
            title: String,
            questions: Vec<usize>,
        }

        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "questions-coin");
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(720.0), full.height() - 24.0),
        );
        let p = ui.painter();
        let header_x = header_left(full, panel);
        p.text(
            Pos2::new(header_x, panel.top()),
            Align2::LEFT_TOP,
            tracked("question bank"),
            text::label(),
            Palette::ACCENT,
        );
        p.text(
            Pos2::new(header_x, panel.top() + 26.0),
            Align2::LEFT_TOP,
            &bank.deck.title,
            text::title(),
            Palette::TEXT,
        );
        if panel.width() >= 560.0 {
            p.text(
                Pos2::new(panel.right().min(full.right() - 64.0), panel.top() + 30.0),
                Align2::RIGHT_TOP,
                tracked(&format!("{} unlocked", bank.questions.len())),
                text::label(),
                Palette::TEXT_DIM,
            );
        }
        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 66.0),
                panel.right_top() + Vec2::new(0.0, 66.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        let compact_filters = panel.width() < 560.0;
        let filters_height = if compact_filters { 100.0 } else { 52.0 };
        let filters_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 74.0),
            panel.right_top() + Vec2::new(0.0, 74.0 + filters_height),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(filters_rect), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("SHOW")
                        .font(text::label())
                        .color(Palette::TEXT_FAINT),
                );
                question_filter(ui, &mut bank.filters.correct, "correct");
                question_filter(ui, &mut bank.filters.incorrect, "incorrect");
                question_filter(ui, &mut bank.filters.unattempted, "not yet attempted");
            });
        });

        // Build real groups rather than relying on adjacent rows happening to
        // share a title. `Store::questions` already follows topic order; this
        // coalescing also guarantees one header per topic if that ever changes.
        let mut groups: Vec<Group> = Vec::new();
        for (index, question) in bank.questions.iter().enumerate() {
            let result = bank.latest_results.get(&question.id).copied();
            if !bank.filters.includes(result) {
                continue;
            }
            let topic_id = question.topic_id;
            if let Some(group) = groups.iter_mut().find(|group| group.topic_id == topic_id) {
                group.questions.push(index);
            } else {
                groups.push(Group {
                    topic_id,
                    title: topic_id
                        .and_then(|id| bank.topics.get(&id))
                        .cloned()
                        .unwrap_or_else(|| "Uncategorised".to_owned()),
                    questions: vec![index],
                });
            }
        }

        let body = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 80.0 + filters_height),
            panel.right_bottom() - Vec2::new(0.0, 34.0),
        );
        let mut open = None;
        let mut toggle_group = None;
        let mut sticky_group = None;
        if body.is_positive() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.set_clip_rect(body);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.spacing_mut().scroll.bar_width = 5.0;
                ui.spacing_mut().scroll.floating = false;
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("question-bank")
                    .auto_shrink([false, false])
                    .scroll_bar_rect(Rect::from_min_max(
                        body.left_top() + Vec2::new(0.0, GROUP_HEADER_HEIGHT),
                        body.right_bottom(),
                    ));
                let requested_scroll = bank.initial_scroll.take();
                scroll.show_viewport(ui, |ui, viewport| {
                    if let Some(offset) = requested_scroll {
                        ui.scroll_with_delta(Vec2::new(0.0, -offset));
                    }
                    ui.set_width(body.width() - 10.0);
                    let content_top = ui.max_rect().top();
                    for group in &groups {
                        let group_top = ui.cursor().top() - content_top;
                        if sticky_group.is_none() || group_top <= viewport.min.y + 1.0 {
                            sticky_group =
                                Some((group.topic_id, group.title.clone(), group.questions.len()));
                        }

                        let (heading, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), GROUP_HEADER_HEIGHT),
                            Sense::hover(),
                        );
                        let collapsed = bank.collapsed.contains(&group.topic_id);
                        if question_group_header(
                            ui,
                            heading,
                            ("group", group.topic_id),
                            &group.title,
                            group.questions.len(),
                            collapsed,
                            false,
                        ) {
                            toggle_group = Some(group.topic_id);
                        }
                        if collapsed {
                            ui.add_space(8.0);
                            continue;
                        }

                        for &index in &group.questions {
                            let question = &bank.questions[index];
                            let prompt = question.prompt_text();
                            let doc = richtext::layout(
                                ui.painter(),
                                &prompt,
                                13.5,
                                (ui.available_width() - 130.0).max(120.0),
                            );
                            let height = (doc.height() + 24.0).max(48.0);
                            let response = ui
                                .push_id(("question-row", question.id), |ui| {
                                    ui.add_sized(
                                        [ui.available_width(), height],
                                        egui::Button::new("").frame(false),
                                    )
                                })
                                .inner;
                            let row = response.rect;
                            let hot = response.hovered() || response.has_focus();
                            let result = bank.latest_results.get(&question.id).copied();
                            let (mark, mark_colour) = match result {
                                Some(true) => ("✓", Palette::CORRECT),
                                Some(false) => ("×", Palette::WRONG),
                                None => ("·", Palette::TEXT_FAINT),
                            };
                            let p = ui.painter();
                            p.rect_filled(
                                row,
                                0,
                                if hot { Palette::CARD } else { Palette::SURFACE },
                            );
                            p.rect_stroke(
                                row,
                                0,
                                Stroke::new(1.0, if hot { Palette::ACCENT } else { Palette::LINE }),
                                egui::StrokeKind::Inside,
                            );
                            p.text(
                                row.left_top() + Vec2::new(12.0, 15.0),
                                Align2::LEFT_TOP,
                                mark,
                                text::small(),
                                mark_colour,
                            );
                            p.text(
                                row.left_top() + Vec2::new(30.0, 15.0),
                                Align2::LEFT_TOP,
                                &question.uid,
                                text::small(),
                                Palette::TEXT_FAINT,
                            );
                            doc.paint(
                                p,
                                row.left_top() + Vec2::new(130.0, 12.0),
                                if hot {
                                    Palette::TEXT
                                } else {
                                    Palette::TEXT_DIM
                                },
                                1.0,
                            );
                            if response.clicked() {
                                open = Some(index);
                            }
                        }
                        ui.add_space(8.0);
                    }
                });
            });
        }

        // Paint this after the scroll contents so it stays above them. Its
        // button is the same collapse control as the in-flow group header.
        if let Some((topic_id, title, count)) = sticky_group {
            let sticky = Rect::from_min_size(
                body.left_top(),
                Vec2::new(body.width() - 10.0, GROUP_HEADER_HEIGHT),
            );
            let collapsed = bank.collapsed.contains(&topic_id);
            if question_group_header(
                ui,
                sticky,
                ("sticky-group", topic_id),
                &title,
                count,
                collapsed,
                true,
            ) {
                toggle_group = Some(topic_id);
            }
        } else if body.is_positive() {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    body.left_top(),
                    Vec2::new(body.width() - 10.0, GROUP_HEADER_HEIGHT),
                ),
                0,
                Palette::BG,
            );
            ui.painter().text(
                body.left_top() + Vec2::new(12.0, 9.0),
                Align2::LEFT_TOP,
                tracked("no questions match"),
                text::label(),
                Palette::TEXT_FAINT,
            );
        }

        if let Some(topic_id) = toggle_group {
            if !bank.collapsed.insert(topic_id) {
                bank.collapsed.remove(&topic_id);
            }
        }

        if let Some(index) = open {
            if let Some(question) = bank.questions.get(index).cloned() {
                self.pending_question = Some((bank.deck.clone(), question));
            }
        }

        ui.painter().text(
            panel.left_bottom(),
            Align2::LEFT_BOTTOM,
            tracked("tap a question to answer it"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        None
    }

    fn progress(&mut self, deck: Deck) -> Option<Screen> {
        let deck_stats = match stats::deck_stats(&self.store, deck.id) {
            Ok(stats) => stats,
            Err(e) => {
                self.error = Some(format!("could not load progress: {e}"));
                return None;
            }
        };
        let topics = stats::topic_stats(&self.store, deck.id).unwrap_or_default();
        let weakest = stats::weakest(&self.store, deck.id, 8).unwrap_or_default();
        let topic_names = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|topic: Topic| (topic.id, topic.title))
            .collect();
        let facts = Rc::new(Facts::load(&self.store, deck.id));
        Some(Screen::Progress(Box::new(Progress {
            deck,
            deck_stats,
            topics,
            weakest,
            topic_names,
            facts,
        })))
    }

    fn progress_screen(&mut self, ui: &mut egui::Ui, progress: &mut Progress) -> Option<Screen> {
        let full = ui.available_rect_before_wrap();
        corner_coin(ui, full, &mut self.coin, "progress-coin");
        let panel = Rect::from_center_size(
            full.center(),
            Vec2::new(full.width().min(680.0), full.height() - 24.0),
        );
        let stat = &progress.deck_stats;
        let p = ui.painter();
        let header_x = header_left(full, panel);
        p.text(
            Pos2::new(header_x, panel.top()),
            Align2::LEFT_TOP,
            tracked("progress"),
            text::label(),
            Palette::ACCENT,
        );
        p.text(
            Pos2::new(header_x, panel.top() + 26.0),
            Align2::LEFT_TOP,
            &progress.deck.title,
            text::title(),
            Palette::TEXT,
        );
        if panel.width() >= 560.0 {
            p.text(
                Pos2::new(panel.right().min(full.right() - 64.0), panel.top() + 18.0),
                Align2::RIGHT_TOP,
                format!("{:.0}%", stat.readiness * 100.0),
                text::number(),
                Palette::ACCENT,
            );
        }
        p.text(
            Pos2::new(header_x, panel.top() + 58.0),
            Align2::LEFT_TOP,
            format!(
                "{} / {} attempted   {:.0}% accuracy",
                stat.attempted,
                stat.questions,
                stat.accuracy * 100.0
            ),
            text::small(),
            Palette::TEXT_DIM,
        );
        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 84.0),
                panel.right_top() + Vec2::new(0.0, 84.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        let body = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 100.0),
            panel.right_bottom() - Vec2::new(0.0, 34.0),
        );
        let mut open_weak = None;
        explain::scroll_column(ui, body, "course-progress", |ui| {
            let (heading, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
            ui.painter().text(
                heading.left_top(),
                Align2::LEFT_TOP,
                tracked("topics"),
                text::label(),
                Palette::TEXT_FAINT,
            );
            for topic in &progress.topics {
                let (row, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::hover());
                let ratio = if topic.questions > 0 {
                    topic.solid as f32 / topic.questions as f32
                } else {
                    0.0
                };
                let p = ui.painter();
                p.text(
                    row.left_top() + Vec2::new(0.0, 3.0),
                    Align2::LEFT_TOP,
                    &topic.title,
                    text::small(),
                    Palette::TEXT,
                );
                p.text(
                    row.right_top() + Vec2::new(0.0, 3.0),
                    Align2::RIGHT_TOP,
                    format!(
                        "{} cards   {:.0}% accuracy",
                        topic.questions,
                        topic.accuracy * 100.0
                    ),
                    text::small(),
                    Palette::TEXT_DIM,
                );
                let bar = Rect::from_min_size(
                    row.left_bottom() - Vec2::new(0.0, 9.0),
                    Vec2::new(row.width(), 3.0),
                );
                p.rect_filled(bar, 0, Palette::LINE);
                let mut filled = bar;
                filled.set_width(bar.width() * ratio);
                p.rect_filled(filled, 0, Palette::ACCENT);
            }

            ui.add_space(10.0);
            let (heading, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
            ui.painter().text(
                heading.left_top(),
                Align2::LEFT_TOP,
                tracked("weak questions"),
                text::label(),
                Palette::TEXT_FAINT,
            );
            if progress.weakest.is_empty() {
                explain::prose(ui, "No answered questions yet.", 13.5, Palette::TEXT_DIM);
            }
            for (index, weak) in progress.weakest.iter().enumerate() {
                let response = ui.add_sized(
                    [ui.available_width(), 44.0],
                    egui::Button::new("").frame(false),
                );
                let hot = response.hovered() || response.has_focus();
                let p = ui.painter();
                if hot {
                    p.rect_filled(response.rect, 0, Palette::CARD);
                }
                p.text(
                    response.rect.left_center() + Vec2::new(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{:.0}%", weak.ema * 100.0),
                    text::small(),
                    if weak.ema < 0.4 {
                        Palette::WRONG
                    } else {
                        Palette::TEXT_FAINT
                    },
                );
                let short: String = weak.prompt.chars().take(72).collect();
                p.text(
                    response.rect.left_center() + Vec2::new(62.0, 0.0),
                    Align2::LEFT_CENTER,
                    short,
                    text::small(),
                    if hot {
                        Palette::TEXT
                    } else {
                        Palette::TEXT_DIM
                    },
                );
                if response.clicked() {
                    open_weak = Some(index);
                }
            }
        });
        ui.painter().text(
            panel.left_bottom(),
            Align2::LEFT_BOTTOM,
            tracked("back is at top right"),
            text::label(),
            Palette::TEXT_FAINT,
        );

        if let Some(index) = open_weak {
            let items = progress
                .weakest
                .iter()
                .filter_map(|weak| self.store.question(weak.question_id).ok().flatten())
                .map(|question| Answered {
                    question,
                    response: None,
                    grade: None,
                })
                .collect();
            self.open_review(
                items,
                index,
                progress.facts.clone(),
                progress.topic_names.clone(),
            );
        }

        None
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
            route: StudyRoute::Scheduled,
            grab: None,
        };
        self.coin.spin();
        self.deal_next(&mut study);
        Some(Screen::Study(Box::new(study)))
    }

    fn begin_single(&mut self, deck: Deck, question: Question, back: Screen) -> Screen {
        let mut session = match Session::start(self.store.clone(), deck.id, Mode::Practice) {
            Ok(session) => session,
            Err(e) => {
                self.error = Some(format!("could not start session: {e}"));
                return back;
            }
        };
        let topics = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|topic: Topic| (topic.id, topic.title))
            .collect();
        let facts = Rc::new(Facts::load(&self.store, deck.id));
        let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();
        session.show(question.id);
        self.coin.spin();
        Screen::Study(Box::new(Study {
            session,
            deck,
            topics,
            facts,
            current: Some(question),
            motion: Motion::deal(),
            recent: Vec::new(),
            history: Vec::new(),
            selected: Vec::new(),
            feedback: None,
            answered: 0,
            correct: 0,
            counts,
            route: StudyRoute::Single {
                back: Box::new(back),
            },
            grab: None,
        }))
    }

    fn begin_lesson_practice(
        &mut self,
        deck: Deck,
        mut questions: Vec<Question>,
        back: Screen,
    ) -> Screen {
        let mut session = match Session::start(self.store.clone(), deck.id, Mode::Practice) {
            Ok(session) => session,
            Err(error) => {
                self.error = Some(format!("could not start lesson practice: {error}"));
                return back;
            }
        };
        if questions.is_empty() {
            return back;
        }
        let current = questions.remove(0);
        session.show(current.id);
        let topics = self
            .store
            .topics(deck.id)
            .unwrap_or_default()
            .into_iter()
            .map(|topic| (topic.id, topic.title))
            .collect();
        let facts = Rc::new(Facts::load(&self.store, deck.id));
        let counts = scheduler::counts(&self.store, deck.id).unwrap_or_default();
        self.coin.spin();
        Screen::Study(Box::new(Study {
            session,
            deck,
            topics,
            facts,
            current: Some(current),
            motion: Motion::deal(),
            recent: Vec::new(),
            history: Vec::new(),
            selected: Vec::new(),
            feedback: None,
            answered: 0,
            correct: 0,
            counts,
            route: StudyRoute::Lesson {
                back: Box::new(back),
                remaining: questions,
            },
            grab: None,
        }))
    }

    fn finish_single(&mut self, study: &mut Study) -> Option<Screen> {
        if !matches!(study.route, StudyRoute::Single { .. }) {
            return None;
        }
        if let Err(e) = study.session.end() {
            self.error = Some(format!("could not close session: {e}"));
        }
        let StudyRoute::Single { mut back } =
            std::mem::replace(&mut study.route, StudyRoute::Scheduled)
        else {
            unreachable!();
        };
        if let Screen::Questions(bank) = back.as_mut() {
            bank.latest_results = self
                .store
                .latest_question_results(bank.deck.id)
                .unwrap_or_default();
        }
        Some(*back)
    }

    fn finish_lesson_practice(&mut self, study: &mut Study) -> Option<Screen> {
        if !matches!(study.route, StudyRoute::Lesson { .. }) {
            return None;
        }
        if let Err(error) = study.session.end() {
            self.error = Some(format!("could not close lesson practice: {error}"));
        }
        let StudyRoute::Lesson { back, .. } =
            std::mem::replace(&mut study.route, StudyRoute::Scheduled)
        else {
            unreachable!();
        };
        Some(*back)
    }

    fn deal_next(&mut self, study: &mut Study) {
        if let StudyRoute::Lesson { remaining, .. } = &mut study.route {
            if remaining.is_empty() {
                study.current = None;
                return;
            }
            let question = remaining.remove(0);
            study.session.show(question.id);
            study.current = Some(question);
            study.motion = Motion::deal();
            study.selected.clear();
            study.counts = scheduler::counts(&self.store, study.deck.id).unwrap_or_default();
            return;
        }
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

        // The question actions are real touch targets now, not a line of key
        // hints. Reserve enough room for their square faces and labels.
        let stage = Rect::from_min_max(
            full.left_top() + Vec2::new(0.0, 74.0),
            full.right_bottom() - Vec2::new(0.0, 94.0),
        );

        // A card that has been answered and flown off the screen is retired
        // here, once its animation has finished.
        if study.motion.is_gone(full)
            && study.feedback.is_none()
            && matches!(study.route, StudyRoute::Scheduled)
        {
            self.deal_next(study);
        }

        let mut action = Action::None;

        if study.feedback.is_none() {
            match study.current.clone() {
                Some(q) => match &q.body {
                    Body::TrueFalse { .. } => {
                        const ANSWER_HEIGHT: f32 = 48.0;
                        const ANSWER_GAP: f32 = 12.0;
                        let card_width = stage.width().min(560.0) - 40.0;
                        let text_size = if q.prompt_text().chars().count() > 180 {
                            16.5
                        } else {
                            19.0
                        };
                        let prompt_height =
                            blocks::layout(ui.painter(), &q.prompt, text_size, card_width - 56.0)
                                .height();
                        let card_height = (prompt_height + 120.0).max(400.0);
                        let mut card_stage = stage;
                        card_stage.set_bottom(stage.bottom() - ANSWER_GAP - ANSWER_HEIGHT);
                        let card_rect = Rect::from_center_size(
                            card_stage.center(),
                            Vec2::new(
                                card_width,
                                card_stage.height().min(card_height + 20.0) - 20.0,
                            ),
                        );
                        let controls = Rect::from_min_size(
                            Pos2::new(card_rect.left() + 18.0, card_rect.bottom() + ANSWER_GAP),
                            Vec2::new(card_rect.width() - 36.0, ANSWER_HEIGHT),
                        );
                        action = self.true_false_card(ui, study, &q, card_rect, controls);
                    }
                    Body::MultipleChoice { options, multi } => {
                        let opts = options.clone();
                        let multi = *multi;
                        action = self.choice_card(ui, study, &q, &opts, multi, stage);
                    }
                },
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
        }

        if study.feedback.is_none()
            && let Some(q) = study.current.as_ref()
        {
            let multi = matches!(&q.body, Body::MultipleChoice { multi: true, .. });
            let button_action = question_actions(ui, full, multi);
            if !matches!(button_action, Action::None) {
                action = button_action;
            }
        }

        // The explanation stays up until it is dismissed, right or wrong. A
        // correct answer is exactly when a misconception is cheapest to fix,
        // and a panel that fades on its own is one you learn to ignore.
        if study.feedback.is_some() {
            if let Some(button_action) = self.feedback_panel(ui, study, full) {
                action = button_action;
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
            if study.feedback.is_some() {
                if i.key_pressed(D) {
                    return Some(Action::Deeper);
                }
                if study.feedback.as_ref().is_some_and(|fb| fb.grade.is_some()) && i.key_pressed(U)
                {
                    return Some(Action::Undo);
                }
                let advance = if matches!(study.route, StudyRoute::Single { .. }) {
                    i.key_pressed(B)
                } else {
                    i.key_pressed(N)
                };
                return (advance || i.key_pressed(Space) || i.key_pressed(Enter))
                    .then_some(Action::Continue);
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
                let mut selected = study.selected.clone();
                selected.sort_unstable();
                let r = Response::MultipleChoice { selected };
                self.apply(study, Action::Answer(r, Input::Click))
            }

            Action::Continue => {
                study.feedback = None;
                if let Some(back) = self.finish_single(study) {
                    return Some(back);
                }
                // A true/false card is already off screen; a choice card is
                // still sitting there and needs replacing now.
                self.deal_next(study);
                if study.current.is_none()
                    && let Some(back) = self.finish_lesson_practice(study)
                {
                    return Some(back);
                }
                None
            }

            Action::Skip => {
                if let Some(q) = study.current.clone() {
                    study.session.skip(q.id);
                    if let Some(back) = self.finish_single(study) {
                        return Some(back);
                    }
                    self.deal_next(study);
                    if study.current.is_none()
                        && let Some(back) = self.finish_lesson_practice(study)
                    {
                        return Some(back);
                    }
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
                    });
                }
                None
            }

            // Undo is a mis-click correction for the result currently on
            // screen. Once Next is pressed there is no UI route back to it.
            Action::Undo => {
                let Some(expected_question) = study
                    .feedback
                    .as_ref()
                    .filter(|feedback| feedback.grade.is_some())
                    .map(|feedback| feedback.question.id)
                else {
                    return None;
                };
                match study.session.undo_last() {
                    Ok(Some(question_id)) => {
                        debug_assert_eq!(question_id, expected_question);
                        study.answered = study.answered.saturating_sub(1);
                        if let Some(index) = study.history.iter().rposition(|item| {
                            item.question.id == question_id && item.grade.is_some()
                        }) {
                            let undone = study.history.remove(index);
                            if undone.grade.is_some_and(|grade| grade.correct) {
                                study.correct = study.correct.saturating_sub(1);
                            }
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

            Action::Quit => {
                if let Some(back) = self.finish_single(study) {
                    return Some(back);
                }
                if let Some(back) = self.finish_lesson_practice(study) {
                    return Some(back);
                }
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
    let _ = write!(out, "Question\n{}", content_transcript(&question.prompt));

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

    if full.width() >= 620.0 {
        p.text(
            top.left_center() + Vec2::new(54.0, 0.0),
            Align2::LEFT_CENTER,
            &study.deck.title,
            text::label(),
            Palette::TEXT_DIM,
        );
    }

    let acc = if study.answered > 0 {
        study.correct as f32 / study.answered as f32
    } else {
        0.0
    };
    p.text(
        if full.width() < 520.0 {
            let fraction = if full.width() < 360.0 { 0.30 } else { 0.36 };
            Pos2::new(full.left() + full.width() * fraction, top.center().y)
        } else {
            top.center()
        },
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
        top.right_center() + Vec2::new(-66.0, 0.0),
        Align2::RIGHT_CENTER,
        if full.width() < 520.0 {
            right
        } else {
            tracked(&right)
        },
        if full.width() < 520.0 {
            text::small()
        } else {
            text::label()
        },
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
}

const ACTION_FACE: f32 = 50.0;
const ACTION_STEP: f32 = 86.0;

/// A card-game action: one square symbol with its hotkey-labelled verb below.
///
/// Only the face is interactive. The label explains it, but does not turn the
/// empty space around the control into a surprise touch target.
fn action_button(
    ui: &egui::Ui,
    id: Id,
    center: Pos2,
    symbol: &str,
    label: &str,
    accent: Color32,
) -> bool {
    let face = Rect::from_center_size(center, Vec2::splat(ACTION_FACE));
    let response = ui.interact(face, id, Sense::click());
    let hover = ui
        .ctx()
        .animate_bool(id.with("hover"), response.hovered() || response.has_focus());
    let border = accent.gamma_multiply(0.55 + 0.45 * hover);
    let ink = if response.hovered() || response.has_focus() {
        accent
    } else {
        Palette::TEXT_DIM
    };

    let p = ui.painter();
    p.rect_filled(face, 0, Palette::CARD.gamma_multiply(0.78 + 0.22 * hover));
    p.rect_stroke(
        face,
        0,
        Stroke::new(1.0 + hover, border),
        egui::StrokeKind::Inside,
    );
    p.text(
        face.center(),
        Align2::CENTER_CENTER,
        symbol,
        text::title(),
        ink,
    );
    p.text(
        face.center_bottom() + Vec2::new(0.0, 7.0),
        Align2::CENTER_TOP,
        label,
        text::label(),
        ink,
    );
    response.clicked()
}

/// Actions available while a card is still asking its question.
fn question_actions(ui: &egui::Ui, full: Rect, multi: bool) -> Action {
    let count = if multi { 3 } else { 2 };
    let first_x = full.center().x - ACTION_STEP * (count - 1) as f32 / 2.0;
    let y = full.bottom() - 56.0;

    if action_button(
        ui,
        Id::new("question-explain"),
        Pos2::new(first_x, y),
        "?",
        "(E)xplain",
        Palette::ACCENT,
    ) {
        return Action::Explain;
    }
    if action_button(
        ui,
        Id::new("question-skip"),
        Pos2::new(first_x + ACTION_STEP, y),
        "»",
        "(S)kip",
        Palette::SKIP,
    ) {
        return Action::Skip;
    }
    if multi
        && action_button(
            ui,
            Id::new("question-confirm"),
            Pos2::new(first_x + 2.0 * ACTION_STEP, y),
            "↵",
            "(↵) Confirm",
            Palette::ACCENT,
        )
    {
        return Action::CommitPicks;
    }
    Action::None
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

fn header_left(full: Rect, panel: Rect) -> f32 {
    if panel.top() < full.top() + 56.0 && panel.left() < full.left() + 56.0 {
        full.left() + 56.0
    } else {
        panel.left()
    }
}

enum DeckRowAction {
    None,
    Open,
    Shuffle,
}

fn deck_row(
    ui: &mut egui::Ui,
    row: Rect,
    deck: &Deck,
    counts: scheduler::Counts,
    stat: stats::DeckStats,
) -> DeckRowAction {
    let quick = Rect::from_min_size(
        Pos2::new(row.right() - row.height(), row.top()),
        Vec2::splat(row.height()),
    );
    let open = Rect::from_min_max(row.left_top(), quick.left_bottom());
    let open_resp = ui
        .push_id(("deck-open", deck.id), |ui| {
            ui.put(open, egui::Button::new("").frame(false))
        })
        .inner;
    let quick_resp = ui
        .push_id(("deck-shuffle", deck.id), |ui| {
            ui.put(quick, egui::Button::new("").frame(false))
        })
        .inner
        .on_hover_text("shuffle all questions");
    let open_hot = open_resp.hovered() || open_resp.has_focus();
    let quick_hot = quick_resp.hovered() || quick_resp.has_focus();

    let p = ui.painter();
    p.rect_filled(
        open,
        0,
        if open_hot {
            Palette::CARD
        } else {
            Palette::SURFACE
        },
    );
    p.rect_filled(
        quick,
        0,
        if quick_hot {
            Palette::CARD
        } else {
            Palette::SURFACE
        },
    );
    p.rect_stroke(
        open,
        0,
        Stroke::new(
            1.0,
            if open_hot {
                Palette::ACCENT
            } else {
                Palette::LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    p.rect_stroke(
        quick,
        0,
        Stroke::new(
            1.0,
            if quick_hot {
                Palette::ACCENT
            } else {
                Palette::LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
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

    let bar = Rect::from_min_size(
        row.left_bottom() + Vec2::new(18.0, -26.0),
        Vec2::new(open.width() - 36.0, 3.0),
    );
    p.rect_filled(bar, 0, Palette::LINE);
    let mut filled = bar;
    filled.set_width(bar.width() * stat.readiness as f32);
    p.rect_filled(filled, 0, Palette::ACCENT);
    if row.width() >= 520.0 {
        p.text(
            open.right_top() + Vec2::new(-18.0, 16.0),
            Align2::RIGHT_TOP,
            format!("{:.0}%", stat.readiness * 100.0),
            text::number(),
            if stat.readiness > 0.6 {
                Palette::CORRECT
            } else {
                Palette::TEXT_DIM
            },
        );
        if let Some(exam) = deck.exam_at {
            let left = exam - now_ms();
            let (label, colour) = if left > 0 {
                (format!("exam in {}", fmt_span(left)), Palette::ACCENT)
            } else {
                ("exam passed".to_owned(), Palette::TEXT_FAINT)
            };
            p.text(
                open.right_top() + Vec2::new(-18.0, 52.0),
                Align2::RIGHT_TOP,
                tracked(&label),
                text::label(),
                colour,
            );
        }
    }
    paint_dice(p, quick.shrink(25.0), quick_hot);

    if open_resp.clicked() {
        DeckRowAction::Open
    } else if quick_resp.clicked() {
        DeckRowAction::Shuffle
    } else {
        DeckRowAction::None
    }
}

fn question_filter(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    let label = tracked(label);
    let width = ui
        .painter()
        .layout_no_wrap(label.clone(), text::label(), Palette::TEXT)
        .rect
        .width()
        + 24.0;
    if ui
        .add_sized(
            [width, 44.0],
            egui::Button::new(
                egui::RichText::new(label)
                    .font(text::label())
                    .color(if *value {
                        Palette::TEXT
                    } else {
                        Palette::TEXT_FAINT
                    }),
            )
            .selected(*value),
        )
        .clicked()
    {
        *value = !*value;
    }
}

fn question_group_header(
    ui: &mut egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    count: usize,
    collapsed: bool,
    sticky: bool,
) -> bool {
    let response = ui
        .push_id(id, |ui| ui.put(rect, egui::Button::new("").frame(false)))
        .inner
        .on_hover_text(if collapsed {
            "expand group"
        } else {
            "collapse group"
        });
    let hot = response.hovered() || response.has_focus();
    let p = ui.painter();
    p.rect_filled(
        rect,
        0,
        if hot {
            Palette::CARD
        } else if sticky {
            Palette::CARD_DEEP
        } else {
            Palette::BG
        },
    );
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, if hot { Palette::ACCENT } else { Palette::LINE }),
    );

    let box_rect =
        Rect::from_center_size(rect.left_center() + Vec2::new(10.0, 0.0), Vec2::splat(12.0));
    let ink = if hot {
        Palette::ACCENT
    } else {
        Palette::TEXT_FAINT
    };
    p.rect_stroke(box_rect, 0, Stroke::new(1.0, ink), egui::StrokeKind::Inside);
    p.line_segment(
        [
            Pos2::new(box_rect.left() + 3.0, box_rect.center().y),
            Pos2::new(box_rect.right() - 3.0, box_rect.center().y),
        ],
        Stroke::new(1.0, ink),
    );
    if collapsed {
        p.line_segment(
            [
                Pos2::new(box_rect.center().x, box_rect.top() + 3.0),
                Pos2::new(box_rect.center().x, box_rect.bottom() - 3.0),
            ],
            Stroke::new(1.0, ink),
        );
    }
    p.text(
        rect.left_center() + Vec2::new(28.0, 0.0),
        Align2::LEFT_CENTER,
        tracked(title),
        text::label(),
        if hot {
            Palette::ACCENT
        } else {
            Palette::TEXT_FAINT
        },
    );
    p.text(
        rect.right_center() - Vec2::new(12.0, 0.0),
        Align2::RIGHT_CENTER,
        tracked(&format!("{count} questions")),
        text::label(),
        Palette::TEXT_FAINT,
    );
    response.clicked()
}

fn lesson_heading(ui: &mut egui::Ui, heading: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    let label = tracked(heading);
    let galley = ui
        .painter()
        .layout_no_wrap(label, text::label(), Palette::ACCENT);
    ui.painter()
        .galley(rect.left_top(), galley.clone(), Palette::ACCENT);
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + galley.rect.width() + 14.0, rect.top() + 8.0),
            Pos2::new(rect.right(), rect.top() + 8.0),
        ],
        Stroke::new(1.0, Palette::LINE),
    );
}

fn lesson_row(ui: &mut egui::Ui, lesson: &Lesson, read: bool) -> bool {
    let width = ui.available_width();
    let compact = width < 520.0;
    let summary_width = if compact {
        (width - 36.0).max(80.0)
    } else {
        (width - 170.0).max(80.0)
    };
    let summary = richtext::layout(ui.painter(), &lesson.summary, 12.5, summary_width);
    let height = (summary.height() + 78.0).max(102.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let response = ui
        .push_id(("lesson-row", lesson.id), |ui| {
            ui.put(
                rect,
                egui::Button::new("")
                    .frame(false)
                    .sense(Sense::click())
                    .min_size(rect.size()),
            )
        })
        .inner;
    let hot = response.hovered() || response.has_focus();
    let border = if hot { Palette::ACCENT } else { Palette::LINE };
    let p = ui.painter();
    p.rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    p.rect_stroke(rect, 0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    p.rect_filled(
        Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
        0,
        if read {
            Palette::CORRECT
        } else if hot {
            Palette::ACCENT
        } else {
            Palette::LINE_BRIGHT
        },
    );
    p.text(
        rect.left_top() + Vec2::new(18.0, 14.0),
        Align2::LEFT_TOP,
        &lesson.title,
        text::body(),
        if hot { Palette::ACCENT } else { Palette::TEXT },
    );
    if !compact {
        p.text(
            rect.right_top() + Vec2::new(-16.0, 17.0),
            Align2::RIGHT_TOP,
            tracked(if read { "read" } else { "unread" }),
            text::label(),
            if read {
                Palette::CORRECT
            } else {
                Palette::TEXT_FAINT
            },
        );
    }
    summary.paint(
        p,
        rect.left_top() + Vec2::new(18.0, 44.0),
        Palette::TEXT_DIM,
        1.0,
    );
    p.text(
        rect.right_bottom() - Vec2::new(16.0, 14.0),
        Align2::RIGHT_BOTTOM,
        tracked(&format!("{} questions", lesson.practice.len())),
        text::label(),
        Palette::TEXT_FAINT,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &lesson.title));
    response.clicked()
}

fn lesson_block(ui: &mut egui::Ui, block: &LessonBlock, facts: &Facts) {
    match block {
        LessonBlock::Text(text) if !text.trim().is_empty() => {
            explain::prose(ui, text, 15.5, Palette::TEXT)
        }
        LessonBlock::Text(_) => {}
        LessonBlock::Heading { heading } => lesson_heading(ui, heading),
        LessonBlock::Math { math } => {
            let width = ui.available_width().max(40.0);
            let doc = richtext::layout(ui.painter(), &format!("${math}$"), 17.0, width);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(width, doc.height() + 8.0), Sense::hover());
            let x = rect.center().x - doc.size.x / 2.0;
            doc.paint(
                ui.painter(),
                Pos2::new(x.max(rect.left()), rect.top() + 4.0),
                Palette::TEXT,
                1.0,
            );
        }
        LessonBlock::Fact { fact } => {
            if let Some(fact) = facts.get(fact) {
                explain::fact_block(ui, fact);
            }
        }
        LessonBlock::Figure { figure } => blocks::show(
            ui,
            &[ContentBlock::Figure {
                figure: figure.clone(),
            }],
            15.0,
            Palette::TEXT,
        ),
    }
}

fn lesson_text(lesson: &Lesson, facts: &Facts) -> String {
    let mut parts = Vec::new();
    for block in &lesson.body {
        match block {
            LessonBlock::Text(text) if !text.trim().is_empty() => {
                parts.push(text.trim().to_owned())
            }
            LessonBlock::Text(_) => {}
            LessonBlock::Heading { heading } => parts.push(heading.trim().to_owned()),
            LessonBlock::Math { math } => parts.push(format!("$${}$$", math.trim())),
            LessonBlock::Figure { figure } => parts.push(format!("[{}]", figure.kind_name())),
            LessonBlock::Fact { fact } => {
                let Some(fact) = facts.get(fact) else {
                    continue;
                };
                let mut quoted = String::new();
                if let Some(title) = &fact.title {
                    quoted.push_str(title.trim());
                }
                match fact.kind {
                    FactKind::Formula => {
                        if let Some(label) = &fact.label {
                            if !quoted.is_empty() {
                                quoted.push('\n');
                            }
                            let _ = write!(quoted, "$${label}$$");
                        }
                    }
                    FactKind::Symbol => {
                        if let Some(label) = &fact.label {
                            if !quoted.is_empty() {
                                quoted.push('\n');
                            }
                            quoted.push_str(label);
                            if let Some(name) = &fact.name {
                                let _ = write!(quoted, " ({name})");
                            }
                        }
                    }
                    FactKind::Note => {}
                }
                let body = content_transcript(&fact.body);
                if !body.is_empty() {
                    if !quoted.is_empty() {
                        quoted.push('\n');
                    }
                    quoted.push_str(&body);
                }
                if !quoted.is_empty() {
                    parts.push(quoted);
                }
            }
        }
    }

    let symbols = explain::lesson_symbols(lesson, facts);
    if !symbols.is_empty() {
        let mut glossary = String::from("Symbols");
        for fact in symbols {
            let _ = write!(glossary, "\n\n{}", fact.label.clone().unwrap_or_default());
            if let Some(name) = &fact.name {
                let _ = write!(glossary, " ({name})");
            }
            let body = content_transcript(&fact.body);
            if !body.is_empty() {
                let _ = write!(glossary, "\n{body}");
            }
        }
        parts.push(glossary);
    }

    parts.join("\n\n")
}

/// One destination on the course screen.
///
/// It is a button rather than a bare interaction rectangle so keyboard focus
/// and Enter/Space activation come for free even though all painting is custom.
fn navigation_row(
    ui: &mut egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    description: &str,
    meta: &str,
) -> bool {
    let response = ui
        .push_id(id, |ui| {
            ui.put(
                rect,
                egui::Button::new("")
                    .frame(false)
                    .sense(Sense::click())
                    .min_size(rect.size()),
            )
        })
        .inner;
    let hot = response.hovered() || response.has_focus();
    let border = if hot { Palette::ACCENT } else { Palette::LINE };
    let p = ui.painter();
    p.rect_filled(rect, 0, if hot { Palette::CARD } else { Palette::SURFACE });
    p.rect_stroke(rect, 0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    p.rect_filled(
        Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
        0,
        if hot {
            Palette::ACCENT
        } else {
            Palette::LINE_BRIGHT
        },
    );
    p.text(
        rect.left_top() + Vec2::new(20.0, 17.0),
        Align2::LEFT_TOP,
        tracked(label),
        text::label(),
        if hot { Palette::ACCENT } else { Palette::TEXT },
    );
    let description_width = if rect.width() < 380.0 {
        rect.width() - 40.0
    } else {
        (rect.width() - 230.0).max(220.0)
    };
    let description = richtext::layout(p, description, 12.5, description_width);
    description.paint(
        p,
        rect.left_top() + Vec2::new(20.0, 44.0),
        Palette::TEXT_DIM,
        1.0,
    );
    if rect.width() >= 380.0 {
        p.text(
            if rect.width() < 520.0 {
                rect.right_top() + Vec2::new(-18.0, 17.0)
            } else {
                rect.right_center() - Vec2::new(18.0, 0.0)
            },
            if rect.width() < 520.0 {
                Align2::RIGHT_TOP
            } else {
                Align2::RIGHT_CENTER
            },
            tracked(meta),
            text::label(),
            if hot {
                Palette::ACCENT
            } else {
                Palette::TEXT_FAINT
            },
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.clicked()
}

/// Two outlined dice for the home-screen quick-study segment.
///
/// Both the faces and pips stay square, and drawing them ourselves avoids
/// depending on a Unicode glyph that may be absent from the selected font.
fn paint_dice(painter: &egui::Painter, rect: Rect, hot: bool) {
    let ink = if hot {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    let side = rect.width().min(rect.height()) * 0.62;
    let back = Rect::from_min_size(rect.right_top() + Vec2::new(-side, 0.0), Vec2::splat(side));
    let front = Rect::from_min_size(
        rect.left_bottom() + Vec2::new(0.0, -side),
        Vec2::splat(side),
    );
    paint_die(painter, back, &[0, 2, 6, 8], Palette::LINE_BRIGHT);
    painter.rect_filled(front, 0, Palette::SURFACE);
    paint_die(painter, front, &[0, 2, 4, 6, 8], ink);
}

fn paint_die(painter: &egui::Painter, rect: Rect, pips: &[usize], ink: Color32) {
    painter.rect_stroke(rect, 0, Stroke::new(1.5, ink), egui::StrokeKind::Inside);
    let xs = [rect.left() + 7.0, rect.center().x, rect.right() - 7.0];
    let ys = [rect.top() + 7.0, rect.center().y, rect.bottom() - 7.0];
    for &pip in pips {
        let point = Pos2::new(xs[pip % 3], ys[pip / 3]);
        painter.rect_filled(Rect::from_center_size(point, Vec2::splat(3.5)), 0, ink);
    }
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
    let rect = Rect::from_min_size(right_top - Vec2::new(width, 0.0), Vec2::new(width, 44.0));
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

fn choice_option_row(
    ui: &mut egui::Ui,
    row: Rect,
    question_id: i64,
    index: usize,
    doc: &richtext::Doc,
    picked: bool,
) -> bool {
    let response = ui.interact(row, Id::new(("opt", question_id, index)), Sense::click());
    let hot = response.hovered() || response.has_focus();
    let hover = ui
        .ctx()
        .animate_bool(Id::new(("opt-hover", question_id, index)), hot);
    let (border, fill, label_col) = if picked {
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

    let painter = ui.painter();
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(row, 0, fill);
    }
    painter.rect_stroke(
        row,
        0,
        Stroke::new(1.0 + 0.7 * hover, border),
        egui::StrokeKind::Inside,
    );
    let key_box = Rect::from_min_size(row.left_top(), Vec2::new(34.0, row.height()));
    painter.line_segment(
        [key_box.right_top(), key_box.right_bottom()],
        Stroke::new(1.0, border),
    );
    painter.text(
        key_box.center(),
        Align2::CENTER_CENTER,
        format!("{}", index + 1),
        text::label(),
        label_col,
    );
    doc.paint(
        painter,
        row.left_top() + Vec2::new(48.0, 11.0),
        label_col,
        1.0,
    );
    response.clicked()
}

impl App {
    fn true_false_card(
        &mut self,
        ui: &mut egui::Ui,
        study: &mut Study,
        q: &Question,
        rect: Rect,
        controls: Rect,
    ) -> Action {
        let mut action = Action::None;
        let mut false_clicked = false;
        let mut true_clicked = false;
        let mut false_hot = false;
        let mut true_hot = false;
        let interactive = study.feedback.is_none() && !study.motion.is_flying();
        let mut hovered = false;

        if interactive {
            let false_button = Rect::from_min_max(
                controls.left_top(),
                Pos2::new(controls.center().x - 3.0, controls.bottom()),
            );
            let true_button = Rect::from_min_max(
                Pos2::new(controls.center().x + 3.0, controls.top()),
                controls.right_bottom(),
            );
            let false_response =
                ui.interact(false_button, Id::new(("tf-false", q.id)), Sense::click());
            let true_response =
                ui.interact(true_button, Id::new(("tf-true", q.id)), Sense::click());
            false_clicked = false_response.clicked();
            true_clicked = true_response.clicked();
            false_hot = false_response.hovered() || false_response.has_focus();
            true_hot = true_response.hovered() || true_response.has_focus();

            let resp = ui.interact(
                rect.translate(study.motion.offset),
                Id::new(("tf", q.id)),
                Sense::drag(),
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
        let p = ui.painter().clone();

        card::deck_behind(&p, rect, 3, Palette::CARD_DEEP, Palette::LINE);

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
            &p,
            drawn,
            angle,
            Palette::TEXT_DIM.gamma_multiply(opacity),
            hover,
        );
        card::face(
            &p,
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
            &p,
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
        let size = if q.prompt_text().chars().count() > 180 {
            16.5
        } else {
            19.0
        };
        let prompt = blocks::layout(&p, &q.prompt, size, wrap);
        let content_top = drawn.top() + 52.0;
        let content_bottom = drawn.bottom() - 28.0;
        let centered = drawn.center().y - prompt.height() / 2.0 - 6.0;
        let local_y = centered
            .max(content_top)
            .min((content_bottom - prompt.height()).max(content_top));
        let local = Pos2::new(drawn.left() + 28.0, local_y);
        prompt.paint_rotated(&p, local, pivot, angle, Palette::TEXT, opacity);
        prompt.interact_figures(ui, local, pivot, angle, Id::new(("tf-plot", q.id)));

        // The swipe directions are also explicit touch targets in their own
        // row directly below the physical card.
        let false_button = Rect::from_min_max(
            controls.left_top(),
            Pos2::new(controls.center().x - 3.0, controls.bottom()),
        );
        let true_button = Rect::from_min_max(
            Pos2::new(controls.center().x + 3.0, controls.top()),
            controls.right_bottom(),
        );
        for (button, colour, hot) in [
            (false_button, Palette::VIOLET, false_hot),
            (true_button, Palette::ACCENT, true_hot),
        ] {
            p.rect_filled(
                button,
                0,
                colour
                    .gamma_multiply(if hot { 0.18 } else { 0.08 })
                    .gamma_multiply(opacity),
            );
            p.rect_stroke(
                button,
                0,
                Stroke::new(
                    if hot { 2.0 } else { 1.0 },
                    colour
                        .gamma_multiply(if hot { 1.0 } else { 0.72 })
                        .gamma_multiply(opacity),
                ),
                egui::StrokeKind::Inside,
            );
        }
        p.text(
            false_button.center(),
            Align2::CENTER_CENTER,
            "◀  FALSE",
            text::label(),
            if false_hot {
                Palette::VIOLET
            } else {
                Palette::TEXT_DIM
            }
            .gamma_multiply(opacity),
        );
        p.text(
            true_button.center(),
            Align2::CENTER_CENTER,
            "TRUE  ▶",
            text::label(),
            if true_hot {
                Palette::ACCENT
            } else {
                Palette::TEXT_DIM
            }
            .gamma_multiply(opacity),
        );

        if false_clicked {
            study.motion.launch(-1.0);
            action = Action::Answer(Response::TrueFalse { value: false }, Input::Click);
        } else if true_clicked {
            study.motion.launch(1.0);
            action = Action::Answer(Response::TrueFalse { value: true }, Input::Click);
        }

        card::stamp(
            &p,
            drawn,
            angle,
            "FALSE",
            Palette::VIOLET,
            Align2::LEFT_TOP,
            (-progress).max(0.0),
        );
        card::stamp(
            &p,
            drawn,
            angle,
            "TRUE",
            Palette::ACCENT,
            Align2::RIGHT_TOP,
            progress.max(0.0),
        );

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
        let width = stage.width().min(660.0);
        let wrap = width - 48.0;

        // Measure first: the card is sized to its content, so a two-line
        // question does not sit in a half-empty box.
        let prompt = blocks::layout(ui.painter(), &q.prompt, 17.5, wrap);
        let option_docs: Vec<_> = options
            .iter()
            .map(|o| richtext::layout(ui.painter(), &o.text, 15.0, wrap - 54.0))
            .collect();
        let options_h: f32 = option_docs
            .iter()
            .map(|doc| (doc.height() + 22.0).max(44.0) + 8.0)
            .sum();
        let content_h = 48.0 + prompt.height() + 22.0 + options_h + 20.0;
        let needs_scroll = content_h > stage.height();

        let card_rect = Rect::from_center_size(
            stage.center(),
            Vec2::new(width, content_h.min(stage.height())),
        );

        let opacity = study.motion.opacity();
        let p = ui.painter().clone();
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

        if needs_scroll {
            // Only cards that have exhausted the stage get an inner viewport.
            // Keeping ordinary cards out of a ScrollArea avoids both phantom
            // scroll bars and content moving under the fixed topic header.
            let body_rect = Rect::from_min_max(
                card_rect.left_top() + Vec2::new(18.0, 46.0),
                card_rect.right_bottom() - Vec2::new(8.0, 10.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                ui.set_clip_rect(body_rect);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.spacing_mut().scroll.bar_width = 5.0;
                ui.spacing_mut().scroll.floating = false;
                egui::ScrollArea::vertical()
                    .id_salt(("choice-card", q.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(body_rect.width() - 10.0);
                        let (prompt_rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), prompt.height()),
                            Sense::hover(),
                        );
                        prompt.paint(&p, prompt_rect.left_top(), Palette::TEXT, opacity);
                        prompt.interact_figures(
                            ui,
                            prompt_rect.left_top(),
                            card_rect.center(),
                            0.0,
                            Id::new(("choice-plot", q.id)),
                        );
                        ui.add_space(22.0);
                        for (index, doc) in option_docs.iter().enumerate() {
                            let height = (doc.height() + 22.0).max(44.0);
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), height),
                                Sense::hover(),
                            );
                            if choice_option_row(
                                ui,
                                row,
                                q.id,
                                index,
                                doc,
                                study.selected.contains(&index),
                            ) {
                                action = Action::Pick(index, multi);
                            }
                            ui.add_space(8.0);
                        }
                    });
            });
        } else {
            let prompt_pos = card_rect.left_top() + Vec2::new(24.0, 48.0);
            prompt.paint(&p, prompt_pos, Palette::TEXT, opacity);
            prompt.interact_figures(
                ui,
                prompt_pos,
                card_rect.center(),
                0.0,
                Id::new(("choice-plot", q.id)),
            );
            let mut y = prompt_pos.y + prompt.height() + 22.0;
            for (index, doc) in option_docs.iter().enumerate() {
                let height = (doc.height() + 22.0).max(44.0);
                let row = Rect::from_min_size(
                    Pos2::new(card_rect.left() + 24.0, y),
                    Vec2::new(card_rect.width() - 48.0, height),
                );
                if choice_option_row(ui, row, q.id, index, doc, study.selected.contains(&index)) {
                    action = Action::Pick(index, multi);
                }
                y += height + 8.0;
            }
        }

        action
    }
}

/// The verdict, the truth, and the explanation — the panel you actually learn
/// from, so it scrolls and it waits for you.
///
/// Returns the touch action chosen beside or below the panel.
impl App {
    fn feedback_panel(
        &mut self,
        ui: &mut egui::Ui,
        study: &mut Study,
        full: Rect,
    ) -> Option<Action> {
        let fb = study.feedback.as_ref()?;
        let (colour, verdict) = match fb.grade {
            Some(grade) if grade.correct => (Palette::CORRECT, "CORRECT"),
            Some(grade) if grade.score > 0.0 => (Palette::WRONG, "PARTLY RIGHT"),
            Some(_) => (Palette::WRONG, "WRONG"),
            None => (Palette::ACCENT, "EXPLANATION"),
        };

        // Grow in over ~120 ms so the verdict does not blink into being.
        let t = (fb.since.elapsed().as_secs_f32() / 0.12).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - t).powi(3);

        const SIDE_SPACE: f32 = 64.0;
        let width = full
            .width()
            .min(640.0)
            .min((full.width() - 2.0 * SIDE_SPACE).max(192.0));
        let top = full.top() + 74.0;
        let bottom = full.bottom() - 12.0;
        let height = (bottom - top).max(160.0);
        let panel = Rect::from_center_size(
            Pos2::new(full.center().x, (top + bottom) / 2.0),
            Vec2::new(width, height * (0.94 + 0.06 * ease)),
        );
        let p = ui.painter();
        p.rect_filled(panel, 0, Palette::CARD);
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

        let topic = fb
            .question
            .topic_id
            .and_then(|id| study.topics.get(&id))
            .cloned()
            .unwrap_or_default();
        let facts = study.facts.clone();
        let depth = fb.depth;
        let body_rect = panel.shrink2(Vec2::new(20.0, 10.0));
        let mut chosen = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
            ui.set_clip_rect(body_rect);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.spacing_mut().scroll.bar_width = 5.0;
            ui.spacing_mut().scroll.floating = false;
            ui.spacing_mut().scroll.bar_inner_margin = 4.0;
            egui::ScrollArea::vertical()
                .id_salt(("feedback-card", fb.question.id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(body_rect.width() - 10.0);
                    ui.add_space(8.0);

                    let (topic_row, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
                    ui.painter().text(
                        topic_row.left_top(),
                        Align2::LEFT_TOP,
                        tracked(&topic),
                        text::label(),
                        Palette::TEXT_FAINT,
                    );
                    blocks::show(ui, &fb.question.prompt, 18.0, Palette::TEXT);
                    ui.add_space(6.0);
                    feedback_answer(ui, fb);

                    ui.add_space(10.0);
                    separator(ui);
                    ui.add_space(8.0);

                    let (verdict_row, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
                    ui.painter().text(
                        verdict_row.left_top(),
                        Align2::LEFT_TOP,
                        tracked(verdict),
                        text::label(),
                        colour,
                    );
                    ui.painter().text(
                        verdict_row.right_top(),
                        Align2::RIGHT_TOP,
                        fb.outcome
                            .as_ref()
                            .map_or_else(|| "-".to_owned(), |outcome| fmt_ms(outcome.latency_ms)),
                        text::label(),
                        Palette::TEXT_FAINT,
                    );
                    explain::body(ui, &fb.question, &facts, depth);

                    ui.add_space(8.0);
                    let (depth_row, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 76.0), Sense::hover());
                    let depth_label = match depth {
                        Depth::Short => "(D)eeper",
                        Depth::Deep => "(D) Shorter",
                    };
                    if action_button(
                        ui,
                        Id::new(("feedback-depth", fb.question.id)),
                        depth_row.center_top() + Vec2::new(0.0, ACTION_FACE / 2.0),
                        "≡",
                        depth_label,
                        Palette::ACCENT,
                    ) {
                        chosen = Some(Action::Deeper);
                    }
                    ui.add_space(8.0);
                });
        });

        let side_y = panel.center().y - 8.0;
        let side_offset = ACTION_FACE / 2.0 + 8.0;
        if fb.grade.is_some()
            && action_button(
                ui,
                Id::new(("feedback-undo", fb.question.id)),
                Pos2::new(panel.left() - side_offset, side_y),
                "↶",
                "(U)ndo",
                Palette::TEXT_DIM,
            )
        {
            return Some(Action::Undo);
        }
        let single = matches!(study.route, StudyRoute::Single { .. });
        if action_button(
            ui,
            Id::new(("feedback-next", fb.question.id)),
            Pos2::new(panel.right() + side_offset, side_y),
            if single { "←" } else { "→" },
            if single { "(B)ack" } else { "(N)ext" },
            Palette::ACCENT,
        ) {
            return Some(Action::Continue);
        }
        chosen
    }
}

/// The answered portion of the compound result card.
fn feedback_answer(ui: &mut egui::Ui, fb: &Feedback) {
    match (&fb.question.body, &fb.response) {
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
            let colour = match fb.grade {
                Some(grade) if grade.correct => Palette::CORRECT,
                Some(_) => Palette::WRONG,
                None => Palette::TEXT_DIM,
            };
            explain::prose(ui, &line, 14.5, colour);
        }
        (Body::MultipleChoice { options, .. }, response) => {
            let selected: Vec<usize> = match response {
                Some(Response::MultipleChoice { selected }) => selected.clone(),
                _ => Vec::new(),
            };
            if selected.is_empty() {
                explain::prose(
                    ui,
                    "you selected no options",
                    13.5,
                    match fb.grade {
                        Some(grade) if grade.correct => Palette::CORRECT,
                        Some(_) => Palette::WRONG,
                        None => Palette::TEXT_DIM,
                    },
                );
                ui.add_space(4.0);
            }
            let notes = explain::option_notes(options, &selected, explain::NoteView::Picked);
            for (index, option) in options.iter().enumerate() {
                let chose = selected.contains(&index);
                let colour = match (option.correct, chose) {
                    (true, _) => Palette::CORRECT,
                    (false, true) => Palette::WRONG,
                    (false, false) => Palette::TEXT_FAINT,
                };
                let mark = match (option.correct, chose) {
                    (true, true) => "+",
                    (true, false) => "·",
                    (false, true) => "!",
                    (false, false) => " ",
                };
                ui.horizontal_top(|ui| {
                    let (mark_rect, _) =
                        ui.allocate_exact_size(Vec2::new(18.0, 20.0), Sense::hover());
                    ui.painter().text(
                        mark_rect.left_top(),
                        Align2::LEFT_TOP,
                        mark,
                        text::small(),
                        colour,
                    );
                    explain::prose(ui, &option.text, 14.5, colour);
                });
                if let Some(note) = notes[index] {
                    ui.horizontal_top(|ui| {
                        ui.add_space(18.0);
                        explain::prose(ui, note, 13.0, colour.gamma_multiply(0.75));
                    });
                }
            }
        }
    }
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
    (
        "nested / brace",
        r"$\underbrace{\frac{1}{1 + \frac{K}{s(1 + sT)}}}_{\text{closed loop}}$",
    ),
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
        // The table `cs-sta-046` sets, so the sheet checks what content ships.
        "Routh array",
        r"$\begin{array}{r|cc} s^3 & \lambda & 2\lambda \\ s^2 & 2\lambda & 1 \\ \hline s^1 & \frac{4\lambda - 1}{2} & 0 \\ s^0 & 1 & 0 \end{array}$",
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

fn nyquist_check_figure() -> Figure {
    Figure::Nyquist {
        num: vec![120.0],
        den: vec![1.0, 6.0, 11.0, 6.0],
    }
}

fn plot_check(ui: &mut egui::Ui) {
    let full = ui.available_rect_before_wrap();
    let painter = ui.painter().clone();
    painter.text(
        full.left_top() + Vec2::new(24.0, 16.0),
        Align2::LEFT_TOP,
        tracked("plot renderer"),
        text::label(),
        Palette::ACCENT,
    );

    let gap = 24.0;
    let width = ((full.width() - gap * 3.0) / 2.0).max(260.0);
    let top = full.top() + 58.0;
    let examples = [
        (
            "BODE REFERENCES",
            Figure::Bode {
                num: vec![20.0],
                den: vec![1.0, 3.0, 2.0],
                phase: true,
            },
        ),
        ("NYQUIST CRITICAL POINT", nyquist_check_figure()),
    ];
    for (index, (label, figure)) in examples.into_iter().enumerate() {
        let left = full.left() + gap + index as f32 * (width + gap);
        painter.text(
            Pos2::new(left, top),
            Align2::LEFT_TOP,
            tracked(label),
            text::label(),
            Palette::TEXT_FAINT,
        );
        let rendered = plot::layout(&painter, &figure, width);
        let pos = Pos2::new(left, top + 28.0);
        let frame = Rect::from_min_size(pos, rendered.size);
        painter.rect_filled(frame, 0, Palette::SURFACE);
        painter.rect_stroke(
            frame,
            0,
            Stroke::new(1.0, Palette::LINE),
            egui::StrokeKind::Inside,
        );
        rendered.paint_rotated(&painter, pos, pos, 0.0, 1.0);
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
        let compact_header = panel.width() < 560.0;
        p.text(
            Pos2::new(
                if compact_header {
                    header_left(full, panel)
                } else {
                    panel.left() + 24.0
                },
                panel.top() + 18.0,
            ),
            Align2::LEFT_TOP,
            tracked(&format!(
                "look back  {}/{}",
                review.idx + 1,
                review.items.len()
            )),
            text::label(),
            Palette::TEXT_FAINT,
        );
        if !compact_header {
            p.text(
                panel.center_top() + Vec2::new(0.0, 18.0),
                Align2::CENTER_TOP,
                tracked(&topic),
                text::label(),
                Palette::TEXT_FAINT,
            );
            p.text(
                Pos2::new(
                    (panel.right() - 24.0).min(full.right() - 64.0),
                    panel.top() + 18.0,
                ),
                Align2::RIGHT_TOP,
                tracked(match item.grade {
                    Some(g) if g.correct => "you were right",
                    Some(_) => "you were wrong",
                    None => "not answered yet",
                }),
                text::label(),
                colour,
            );
        }
        p.line_segment(
            [
                panel.left_top() + Vec2::new(0.0, 44.0),
                panel.right_top() + Vec2::new(0.0, 44.0),
            ],
            Stroke::new(1.0, Palette::LINE),
        );

        const FOOT: f32 = 72.0;
        let body_rect = Rect::from_min_max(
            panel.left_top() + Vec2::new(24.0, 56.0),
            panel.right_bottom() - Vec2::new(16.0, FOOT),
        );

        let facts = review.facts.clone();
        let depth = review.depth;
        explain::scroll_column(ui, body_rect, "review", |ui| {
            blocks::show(ui, &q.prompt, 18.0, Palette::TEXT);
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
        let first_x = footer.center().x - 72.0;
        let button_y = footer.top() + ACTION_FACE / 2.0;
        let prev_touch = action_button(
            ui,
            Id::new("review-prev"),
            Pos2::new(first_x, button_y),
            "←",
            "previous",
            Palette::TEXT_DIM,
        );
        let deeper_touch = action_button(
            ui,
            Id::new("review-depth"),
            Pos2::new(first_x + 72.0, button_y),
            "≡",
            match depth {
                Depth::Short => "deeper",
                Depth::Deep => "shorter",
            },
            Palette::ACCENT,
        );
        let next_touch = action_button(
            ui,
            Id::new("review-next"),
            Pos2::new(first_x + 144.0, button_y),
            "→",
            "next",
            Palette::TEXT_DIM,
        );

        let (prev, next, deeper) = ui.ctx().input(|i| {
            use egui::Key::*;
            (
                i.key_pressed(ArrowLeft),
                i.key_pressed(ArrowRight),
                i.key_pressed(D),
            )
        });
        if prev || prev_touch {
            review.idx = review.idx.saturating_sub(1);
        }
        if next || next_touch {
            review.idx = (review.idx + 1).min(review.items.len() - 1);
        }
        if deeper || deeper_touch {
            review.depth = review.depth.toggled();
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
            Pos2::new(panel.right().min(full.right() - 64.0), panel.top() + 30.0),
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
        let weak_body = Rect::from_min_max(
            panel.left_top() + Vec2::new(0.0, 152.0),
            panel.right_bottom() - Vec2::new(0.0, 64.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(weak_body), |ui| {
            ui.set_clip_rect(weak_body);
            egui::ScrollArea::vertical()
                .id_salt("summary-weak")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(weak_body.width() - 8.0);
                    for (i, w) in sum.weakest.iter().take(7).enumerate() {
                        let response = ui.add_sized(
                            [ui.available_width(), 44.0],
                            egui::Button::new("").frame(false),
                        );
                        let hot = response.hovered() || response.has_focus();
                        if response.clicked() {
                            open = Some(i);
                        }
                        let p = ui.painter();
                        if hot {
                            p.rect_filled(response.rect, 0, Palette::CARD);
                            p.line_segment(
                                [response.rect.left_top(), response.rect.left_bottom()],
                                Stroke::new(2.0, Palette::ACCENT),
                            );
                        }
                        let short: String = w.prompt.chars().take(62).collect();
                        p.text(
                            response.rect.left_center() + Vec2::new(52.0, 0.0),
                            Align2::LEFT_CENTER,
                            short,
                            text::small(),
                            if hot {
                                Palette::TEXT
                            } else {
                                Palette::TEXT_DIM
                            },
                        );
                        p.text(
                            response.rect.left_center() + Vec2::new(8.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("{:>3.0}%", w.ema * 100.0),
                            text::small(),
                            if w.ema < 0.4 {
                                Palette::WRONG
                            } else {
                                Palette::TEXT_FAINT
                            },
                        );
                    }
                });
        });

        if let Some(i) = open {
            self.open_weakest(sum, i);
        }

        let gap = 12.0;
        let button_width = (panel.width() - gap) / 2.0;
        let btn = Rect::from_min_size(
            Pos2::new(panel.left(), panel.bottom() - 52.0),
            Vec2::new(button_width, 44.0),
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
            Pos2::new(panel.left() + button_width + gap, panel.bottom() - 52.0),
            Vec2::new(button_width, 44.0),
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

        if resp.clicked() {
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
    use idiosepius_core::ContentBlock;

    fn begin_with_body(body: Body) -> (App, Box<Study>) {
        let context = egui::Context::default();
        let store = Store::open_in_memory().unwrap();
        let deck_id = store
            .upsert_deck("interaction", "Interaction", None, None)
            .unwrap();
        store
            .upsert_question(&idiosepius_core::NewQuestion {
                deck_id,
                topic_id: None,
                uid: "interaction".into(),
                prompt: vec![ContentBlock::text("Choose carefully.")],
                body,
                explanation: Some("That is the answer.".into()),
                explain: Default::default(),
                difficulty: 1,
                source: None,
                tags: Vec::new(),
            })
            .unwrap();

        let mut app = App::new(&context, store, None);
        let deck = app.decks[0].clone();
        let Some(Screen::Study(study)) = app.begin(deck, Mode::Practice) else {
            panic!("study should start");
        };
        (app, study)
    }

    fn clipboard_question() -> Question {
        Question {
            id: 1,
            deck_id: 1,
            topic_id: None,
            uid: "clipboard".into(),
            prompt: vec![ContentBlock::text(
                r"Which value follows from $G(s)=\frac{1}{s+1}$?",
            )],
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
    fn one_back_operation_serves_study_summary_and_decks() {
        let (mut app, study) = begin_with_body(Body::TrueFalse { answer: true });
        let mut screen = Screen::Study(study);
        assert_eq!(screen_depth(&screen), 1);

        assert!(app.back_once(&mut screen));
        assert!(matches!(screen, Screen::Summary(_)));
        assert_eq!(
            screen_depth(&screen),
            1,
            "ending study replaces its history entry"
        );

        assert!(app.back_once(&mut screen));
        assert!(matches!(screen, Screen::Decks));
        assert_eq!(screen_depth(&screen), 0);
    }

    #[test]
    fn question_filters_are_independent() {
        let mut filters = QuestionFilters::default();
        assert!(filters.includes(Some(true)));
        assert!(filters.includes(Some(false)));
        assert!(filters.includes(None));

        filters.correct = false;
        filters.unattempted = false;
        assert!(!filters.includes(Some(true)));
        assert!(filters.includes(Some(false)));
        assert!(!filters.includes(None));
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
                prompt: vec![ContentBlock::text("The statement is true.")],
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

    #[test]
    fn an_empty_multi_selection_can_be_confirmed() {
        let body = Body::MultipleChoice {
            options: vec![
                idiosepius_core::Choice::new("First", false),
                idiosepius_core::Choice::new("Second", false),
            ],
            multi: true,
        };
        let (mut app, mut study) = begin_with_body(body);

        assert!(study.selected.is_empty());
        app.apply(&mut study, Action::CommitPicks);

        let feedback = study.feedback.as_ref().expect("answer should be recorded");
        assert_eq!(
            feedback.response,
            Some(Response::MultipleChoice {
                selected: Vec::new()
            })
        );
        assert_eq!(feedback.grade, Some(Grade::RIGHT));
    }

    #[test]
    fn undo_only_applies_to_the_answer_currently_showing() {
        let (mut app, mut study) = begin_with_body(Body::TrueFalse { answer: true });
        let question_id = study.current.as_ref().unwrap().id;

        app.apply(
            &mut study,
            Action::Answer(Response::TrueFalse { value: true }, Input::Key),
        );
        assert_eq!(study.answered, 1);
        assert!(study.feedback.is_some());

        app.apply(&mut study, Action::Undo);
        assert_eq!(study.answered, 0);
        assert_eq!(study.correct, 0);
        assert!(study.feedback.is_none());
        assert_eq!(study.current.as_ref().map(|q| q.id), Some(question_id));
        assert!(study.history.is_empty());

        app.apply(
            &mut study,
            Action::Answer(Response::TrueFalse { value: true }, Input::Key),
        );
        app.apply(&mut study, Action::Continue);
        app.apply(&mut study, Action::Undo);
        assert_eq!(study.answered, 1, "Undo expires when Next is chosen");
        assert_eq!(study.correct, 1);
        assert_eq!(study.history.len(), 1);
    }

    #[test]
    fn a_question_bank_answer_returns_to_a_refreshed_bank() {
        let (mut app, scheduled) = begin_with_body(Body::TrueFalse { answer: true });
        let deck = scheduled.deck.clone();
        let question = scheduled.current.clone().unwrap();
        let mut bank = app.question_bank(deck.clone()).unwrap();
        if let Screen::Questions(bank) = &mut bank {
            bank.filters.correct = false;
            bank.filters.incorrect = false;
            bank.filters.unattempted = true;
        }
        let Screen::Study(mut single) = app.begin_single(deck, question.clone(), bank) else {
            panic!("single-question study should start");
        };

        assert!(
            app.apply(
                &mut single,
                Action::Answer(Response::TrueFalse { value: true }, Input::Key),
            )
            .is_none(),
            "feedback stays visible until the learner continues"
        );
        let Some(Screen::Questions(bank)) = app.apply(&mut single, Action::Continue) else {
            panic!("continuing should return to the question bank");
        };
        assert_eq!(bank.latest_results.get(&question.id), Some(&true));
        assert!(
            !bank
                .filters
                .includes(bank.latest_results.get(&question.id).copied()),
            "the answered question leaves a not-yet-attempted-only view"
        );
    }

    #[test]
    fn lesson_practice_uses_only_the_authored_order_and_returns_to_the_lesson() {
        let context = egui::Context::default();
        let store = Store::open_in_memory().unwrap();
        let deck_id = store.upsert_deck("lesson", "Lesson", None, None).unwrap();
        for uid in ["second", "first", "outside"] {
            store
                .upsert_question(&idiosepius_core::NewQuestion {
                    deck_id,
                    topic_id: None,
                    uid: uid.into(),
                    prompt: vec![ContentBlock::text(uid)],
                    body: Body::TrueFalse { answer: true },
                    explanation: None,
                    explain: Default::default(),
                    difficulty: 1,
                    source: None,
                    tags: Vec::new(),
                })
                .unwrap();
        }
        let questions = store
            .questions_by_uids(deck_id, &["first".into(), "second".into()])
            .unwrap();
        let mut app = App::new(&context, store, None);
        let deck = app.decks[0].clone();
        let Screen::Study(mut study) = app.begin_lesson_practice(deck, questions, Screen::Decks)
        else {
            panic!("lesson practice should start");
        };

        assert_eq!(study.current.as_ref().unwrap().uid, "first");
        app.apply(
            &mut study,
            Action::Answer(Response::TrueFalse { value: true }, Input::Key),
        );
        assert!(app.apply(&mut study, Action::Continue).is_none());
        assert_eq!(study.current.as_ref().unwrap().uid, "second");

        let returned = app.apply(&mut study, Action::Skip);
        assert!(matches!(returned, Some(Screen::Decks)));
        assert!(
            study
                .history
                .iter()
                .all(|answer| answer.question.uid != "outside")
        );
    }
}
