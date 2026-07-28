//! Browser shell for the OPFS-backed study database.
//!
//! The live Turso files stay in memory so the ordinary synchronous application
//! can use them unchanged. Between egui frames, a checkpointed SQLite snapshot
//! is written to origin-private storage. Import and export are deliberately
//! explicit because Firefox does not expose a persistent read/write handle to
//! an arbitrary user-selected file.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use eframe::egui::{self, Align2, Id, Rect, Sense, Stroke, Vec2};
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::app::{App, Request};
use crate::import::PickedFile;
use crate::theme::{Palette, text, tracked};

enum Event {
    DatabasePicked(Result<Option<Vec<u8>>, String>),
    DecksPicked(Result<Option<Vec<PickedFile>>, String>),
    RepositoryLoaded(Result<Vec<PickedFile>, String>),
    Saved(Result<(), String>),
}

pub(crate) struct BrowserApp {
    app: Option<App>,
    events: Rc<RefCell<Vec<Event>>>,
    status: Option<String>,
    error: Option<String>,
    last_saved_generation: u64,
    save_in_flight: bool,
    /// A UI Back already changed the Rust screen; ignore the matching
    /// asynchronous `popstate` when `history.go` catches up.
    ignore_pop_to: Option<usize>,
}

pub(crate) struct InitialState {
    database: Vec<u8>,
    wal: Vec<u8>,
    error: Option<String>,
}

impl InitialState {
    pub(crate) async fn load() -> Self {
        match load_opfs().await {
            Ok((database, wal)) => Self {
                database,
                wal,
                error: None,
            },
            Err(error) => Self {
                database: Vec::new(),
                wal: Vec::new(),
                error: Some(format!(
                    "Could not read browser storage: {}",
                    display_js(error)
                )),
            },
        }
    }
}

impl BrowserApp {
    pub(crate) fn new(ctx: &egui::Context, initial: InitialState) -> Self {
        crate::theme::install(ctx);
        ctx.options_mut(|options| options.zoom_with_keyboard = true);
        browser_history_init(0);

        let InitialState {
            database,
            wal,
            mut error,
        } = initial;
        let app = if database.is_empty() {
            None
        } else {
            match idiosepius_core::Store::open_browser(database, wal) {
                Ok(store) => Some(App::new(ctx, store, None)),
                Err(open_error) => {
                    error = Some(format!(
                        "Could not open the database stored in this browser: {open_error:#}"
                    ));
                    None
                }
            }
        };

        Self {
            app,
            events: Rc::new(RefCell::new(Vec::new())),
            status: None,
            error,
            last_saved_generation: 0,
            save_in_flight: false,
            ignore_pop_to: None,
        }
    }

    fn poll_browser_history(&mut self) -> bool {
        let mut handled = false;
        let mut projected_depth = self.app.as_ref().map_or(0, App::navigation_depth);
        loop {
            let target = browser_take_history_depth();
            if target < 0 {
                break;
            }
            let target = target as usize;
            if self.ignore_pop_to == Some(target) {
                self.ignore_pop_to = None;
                continue;
            }
            let Some(app) = &mut self.app else {
                browser_replace_history(0);
                continue;
            };
            if target < projected_depth {
                app.request_back(projected_depth - target);
                projected_depth = target;
                handled = true;
            } else if target > projected_depth {
                // Screen state is deliberately not resurrected after leaving a
                // session. Bounce an attempted Forward back to the live view
                // instead of leaving a duplicate root entry behind.
                self.ignore_pop_to = Some(projected_depth);
                browser_go_history(-((target - projected_depth) as i32));
            }
        }
        handled
    }

    fn open_database(&mut self, ctx: &egui::Context, database: Vec<u8>) {
        match idiosepius_core::Store::open_browser(database, Vec::new()) {
            Ok(store) => {
                self.app = Some(App::new(ctx, store, None));
                self.error = None;
                self.status = Some("Database ready".into());
                self.last_saved_generation = 0;
            }
            Err(error) => {
                self.error = Some(format!("That is not a usable study database: {error:#}"));
            }
        }
    }

    fn create_database(&mut self, ctx: &egui::Context) {
        self.open_database(ctx, Vec::new());
    }

    fn pick_database(&self, ctx: &egui::Context) {
        // Create and click the input before yielding so Firefox still considers
        // this part of the user's button gesture.
        let promise = browser_pick_files(".db,.sqlite,.sqlite3", false);
        let events = self.events.clone();
        let ctx = ctx.clone();
        spawn_local(async move {
            let result = picked_files(promise)
                .await
                .map(|mut files| files.pop().map(|file| file.bytes))
                .map_err(display_js);
            events.borrow_mut().push(Event::DatabasePicked(result));
            ctx.request_repaint();
        });
    }

    fn pick_decks(&self, ctx: &egui::Context) {
        let promise = browser_pick_files(".json,.zip", true);
        let events = self.events.clone();
        let ctx = ctx.clone();
        spawn_local(async move {
            let result = picked_files(promise)
                .await
                .map(|files| (!files.is_empty()).then_some(files))
                .map_err(display_js);
            events.borrow_mut().push(Event::DecksPicked(result));
            ctx.request_repaint();
        });
    }

    fn load_repository(&mut self, ctx: &egui::Context, url: String) {
        let promise = browser_load_github_repository(&url);
        let events = self.events.clone();
        let ctx = ctx.clone();
        spawn_local(async move {
            let result = picked_files(promise).await.map_err(display_js);
            events.borrow_mut().push(Event::RepositoryLoaded(result));
            ctx.request_repaint();
        });
    }

    fn export_database(&mut self) {
        let Some(app) = &mut self.app else { return };
        match app.export_database() {
            Ok(bytes) => {
                download_database(&bytes);
                app.notify("database exported");
            }
            Err(error) => {
                app.report_error(format!("could not export the database: {error:#}"));
            }
        }
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        let events = self.events.borrow_mut().drain(..).collect::<Vec<_>>();
        for event in events {
            match event {
                Event::DatabasePicked(Ok(Some(bytes))) => self.open_database(ctx, bytes),
                Event::DatabasePicked(Ok(None)) | Event::DecksPicked(Ok(None)) => {}
                Event::DatabasePicked(Err(error)) | Event::DecksPicked(Err(error)) => {
                    self.error = Some(error);
                }
                Event::DecksPicked(Ok(Some(files))) | Event::RepositoryLoaded(Ok(files)) => {
                    if let Some(app) = &mut self.app {
                        app.import_picked_files(files);
                    }
                }
                Event::RepositoryLoaded(Err(error)) => {
                    if let Some(app) = &mut self.app {
                        app.repository_import_failed(error);
                    }
                }
                Event::Saved(Ok(())) => {
                    self.save_in_flight = false;
                }
                Event::Saved(Err(error)) => {
                    self.save_in_flight = false;
                    self.error = Some(format!("Browser storage write failed: {error}"));
                }
            }
        }
    }

    fn persist_if_changed(&mut self, ctx: &egui::Context) {
        if self.save_in_flight {
            return;
        }
        let Some(app) = &self.app else { return };
        if app.browser_snapshot().generation == self.last_saved_generation {
            return;
        }
        let snapshot = match app.browser_checkpoint_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.error = Some(format!("Could not checkpoint browser database: {error:#}"));
                return;
            }
        };

        self.last_saved_generation = snapshot.generation;
        self.save_in_flight = true;
        let events = self.events.clone();
        let ctx = ctx.clone();
        spawn_local(async move {
            let result = save_opfs(snapshot.database, snapshot.wal)
                .await
                .map_err(display_js);
            events.borrow_mut().push(Event::Saved(result));
            ctx.request_repaint();
        });
    }

    /// Whether what is in the browser's storage is what is in the database.
    ///
    /// All that is left of the old toolbar: the buttons moved onto the deck
    /// screen, where they belong, but persistence is invisible and silent, so
    /// it still owes the user one word in the corner.
    fn storage_state(&self, ui: &egui::Ui) {
        // A storage failure says what went wrong, in full: it is the one thing
        // here the user may have to act on, and "storage error" alone would
        // not tell them whether their answers are being kept.
        let (label, font, colour) = match (&self.error, self.save_in_flight) {
            (Some(error), _) => (error.clone(), text::small(), Palette::WRONG),
            (None, true) => (tracked("saving"), text::label(), Palette::TEXT_FAINT),
            (None, false) => (tracked("saved"), text::label(), Palette::TEXT_FAINT),
        };
        ui.painter().text(
            ui.max_rect().right_bottom() + Vec2::new(-12.0, -10.0),
            Align2::RIGHT_BOTTOM,
            label,
            font,
            colour,
        );
    }

    fn setup(&mut self, ui: &mut egui::Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0, Palette::BG);
        let width = 520.0_f32.min((ui.max_rect().width() - 40.0).max(280.0));
        let height = 310.0_f32.min((ui.max_rect().height() - 40.0).max(260.0));
        let panel = Rect::from_center_size(ui.max_rect().center(), Vec2::new(width, height));
        ui.painter().rect_filled(panel, 0, Palette::SURFACE);
        ui.painter().rect_stroke(
            panel,
            0,
            Stroke::new(1.0, Palette::LINE_BRIGHT),
            egui::StrokeKind::Inside,
        );
        let inner = panel.shrink(28.0);
        let mut create = false;
        let mut import = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            egui::ScrollArea::vertical()
                .id_salt("browser-setup")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(inner.width() - 8.0);
                    ui.label(
                        egui::RichText::new(tracked("database"))
                            .font(text::title())
                            .color(Palette::TEXT),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(
                            "No study database is stored in this browser yet.",
                        )
                        .font(text::body())
                        .color(Palette::TEXT_DIM),
                    );
                    ui.add_space(16.0);
                    let (create_rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 44.0),
                        Sense::hover(),
                    );
                    create = draw_button(ui, create_rect, "create empty database");
                    ui.add_space(8.0);
                    let (import_rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 44.0),
                        Sense::hover(),
                    );
                    import = draw_button(ui, import_rect, "import database");
                    ui.add_space(18.0);
                    ui.label(
                        egui::RichText::new(self.error.as_deref().unwrap_or(
                            "Stored automatically in this browser. Export creates a normal SQLite file.",
                        ))
                        .font(text::small())
                        .color(if self.error.is_some() {
                            Palette::WRONG
                        } else {
                            Palette::TEXT_FAINT
                        }),
                    );
                });
        });
        if create {
            self.create_database(ui.ctx());
        } else if import {
            self.pick_database(ui.ctx());
        }
    }
}

impl eframe::App for BrowserApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll_events(ui.ctx());
        let history_pop = self.poll_browser_history();
        ui.painter().rect_filled(ui.max_rect(), 0, Palette::BG);

        if self.app.is_some() {
            // The app owns the whole surface: there is no browser chrome above
            // it any more, and importing and exporting are asked for from
            // inside it, the same way they are on the desktop.
            let mut body_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("browser-app")
                    .max_rect(ui.max_rect()),
            );
            let depth_before = self.app.as_ref().map_or(0, App::navigation_depth);
            let request = if let Some(app) = &mut self.app {
                eframe::App::ui(app, &mut body_ui, frame);
                app.take_request()
            } else {
                None
            };
            let depth_after = self.app.as_ref().map_or(0, App::navigation_depth);
            if history_pop {
                let target = browser_current_history_depth();
                if target >= 0 && depth_after > target as usize {
                    // Some logical Back operations replace a screen instead of
                    // reducing depth (study -> summary). Restore one app entry
                    // so the next browser Back still reaches the root.
                    browser_push_history(depth_after);
                } else {
                    browser_replace_history(depth_after);
                }
            } else if depth_after > depth_before {
                browser_push_history(depth_after);
            } else if depth_after < depth_before {
                self.ignore_pop_to = Some(depth_after);
                browser_go_history(-((depth_before - depth_after) as i32));
            }
            match request {
                Some(Request::ImportLocalDeck) => self.pick_decks(ui.ctx()),
                Some(Request::ImportGithub(url)) => self.load_repository(ui.ctx(), url),
                Some(Request::ExportDatabase) => self.export_database(),
                None => {}
            }
            self.storage_state(ui);
        } else {
            self.setup(ui);
        }

        if let Some(status) = &self.status {
            ui.painter().text(
                ui.max_rect().left_bottom() + Vec2::new(12.0, -10.0),
                Align2::LEFT_BOTTOM,
                status,
                text::small(),
                Palette::TEXT_DIM,
            );
        }
        self.persist_if_changed(ui.ctx());
    }
}

fn draw_button(ui: &mut egui::Ui, rect: Rect, label: &str) -> bool {
    let response = ui.interact(rect, Id::new(("browser-button", label)), Sense::click());
    let color = if response.hovered() {
        Palette::ACCENT
    } else {
        Palette::TEXT_DIM
    };
    ui.painter()
        .rect_stroke(rect, 0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        tracked(label),
        text::label(),
        color,
    );
    response.clicked()
}

async fn load_opfs() -> Result<(Vec<u8>, Vec<u8>), JsValue> {
    let value = JsFuture::from(browser_load_database()).await?;
    let array = js_sys::Array::from(&value);
    Ok((
        js_sys::Uint8Array::new(&array.get(0)).to_vec(),
        js_sys::Uint8Array::new(&array.get(1)).to_vec(),
    ))
}

async fn save_opfs(database: Vec<u8>, wal: Vec<u8>) -> Result<(), JsValue> {
    let database = js_sys::Uint8Array::from(database.as_slice());
    let wal = js_sys::Uint8Array::from(wal.as_slice());
    JsFuture::from(browser_save_database(&database, &wal)).await?;
    Ok(())
}

async fn picked_files(promise: js_sys::Promise) -> Result<Vec<PickedFile>, JsValue> {
    let value = JsFuture::from(promise).await?;
    let array = js_sys::Array::from(&value);
    let mut files = Vec::new();
    let mut index = 0;
    while index + 1 < array.length() {
        files.push(PickedFile {
            name: array
                .get(index)
                .as_string()
                .unwrap_or_else(|| "file".into()),
            bytes: js_sys::Uint8Array::new(&array.get(index + 1)).to_vec(),
        });
        index += 2;
    }
    Ok(files)
}

fn display_js(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&value, &JsValue::from_str("message"))
                .ok()?
                .as_string()
        })
        .unwrap_or_else(|| format!("{value:?}"))
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
let saveQueue = Promise.resolve();
let historyDepths = [];
let currentHistoryDepth = 0;
let historyInstalled = false;

function appHistoryState(depth) {
    const existing =
        history.state && typeof history.state === "object" ? history.state : {};
    return { ...existing, idiosepiusDepth: depth };
}

export function browser_history_init(depth) {
    currentHistoryDepth = depth;
    history.replaceState(appHistoryState(depth), "");
    if (!historyInstalled) {
        window.addEventListener("popstate", (event) => {
            const value = event.state && event.state.idiosepiusDepth;
            currentHistoryDepth = Number.isInteger(value) ? value : 0;
            historyDepths.push(currentHistoryDepth);
        });
        historyInstalled = true;
    }
}

export function browser_take_history_depth() {
    return historyDepths.length ? historyDepths.shift() : -1;
}

export function browser_current_history_depth() {
    return currentHistoryDepth;
}

export function browser_push_history(depth) {
    currentHistoryDepth = depth;
    history.pushState(appHistoryState(depth), "");
}

export function browser_replace_history(depth) {
    currentHistoryDepth = depth;
    const state = history.state;
    if (state && Number.isInteger(state.idiosepiusDepth) &&
        state.idiosepiusDepth === depth) {
        return;
    }
    history.replaceState(appHistoryState(depth), "");
}

export function browser_go_history(delta) {
    history.go(delta);
}

async function opfsRead(name) {
    const root = await navigator.storage.getDirectory();
    try {
        const handle = await root.getFileHandle(name);
        return new Uint8Array(await (await handle.getFile()).arrayBuffer());
    } catch (error) {
        if (error && error.name === "NotFoundError") return new Uint8Array();
        throw error;
    }
}

async function opfsWrite(name, bytes) {
    const root = await navigator.storage.getDirectory();
    const handle = await root.getFileHandle(name, { create: true });
    const writable = await handle.createWritable();
    await writable.write(bytes);
    await writable.close();
}

export async function browser_load_database() {
    return [await opfsRead("study.db"), await opfsRead("study.db-wal")];
}

export function browser_save_database(database, wal) {
    const dbCopy = new Uint8Array(database);
    const walCopy = new Uint8Array(wal);
    const write = saveQueue.catch(() => {}).then(async () => {
        await opfsWrite("study.db", dbCopy);
        await opfsWrite("study.db-wal", walCopy);
    });
    saveQueue = write;
    return write;
}

export function browser_pick_files(accept, multiple) {
    return new Promise((resolve, reject) => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = accept;
        input.multiple = multiple;
        input.style.display = "none";
        document.body.appendChild(input);
        input.addEventListener("change", async () => {
            try {
                const result = [];
                for (const file of Array.from(input.files || [])) {
                    result.push(file.name, new Uint8Array(await file.arrayBuffer()));
                }
                resolve(result);
            } catch (error) {
                reject(error);
            } finally {
                input.remove();
            }
        }, { once: true });
        input.addEventListener("cancel", () => {
            input.remove();
            resolve([]);
        }, { once: true });
        input.click();
    });
}

export async function browser_load_github_repository(repositoryUrl) {
    let parsed;
    try {
        parsed = new URL(repositoryUrl.trim());
    } catch {
        throw new Error("Enter a complete GitHub URL, for example https://github.com/owner/repository.");
    }
    const host = parsed.hostname.toLowerCase();
    const parts = parsed.pathname.split("/").filter(Boolean);
    if (
        parsed.protocol !== "https:" ||
        (host !== "github.com" && host !== "www.github.com") ||
        parts.length !== 2
    ) {
        throw new Error("Use the main URL of a public GitHub repository: https://github.com/owner/repository.");
    }

    const owner = parts[0];
    const repository = parts[1].replace(/\.git$/i, "");
    if (!owner || !repository) {
        throw new Error("The GitHub URL must include both an owner and a repository.");
    }

    const apiUrl =
        `https://api.github.com/repos/${encodeURIComponent(owner)}/` +
        `${encodeURIComponent(repository)}/git/trees/HEAD?recursive=1`;
    const treeResponse = await fetch(apiUrl, {
        headers: {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    });
    if (!treeResponse.ok) {
        if (treeResponse.status === 404) {
            throw new Error("Repository not found. Check the URL and make sure the repository is public.");
        }
        if (treeResponse.status === 403) {
            throw new Error("GitHub refused the request, possibly because its anonymous API limit was reached. Try again later.");
        }
        throw new Error(`GitHub returned ${treeResponse.status} ${treeResponse.statusText}.`);
    }

    const tree = await treeResponse.json();
    if (tree.truncated) {
        throw new Error("This repository is too large for GitHub to return its complete file list.");
    }
    const jsonEntries = (tree.tree || []).filter(
        (entry) =>
            entry.type === "blob" &&
            typeof entry.path === "string" &&
            entry.path.toLowerCase().endsWith(".json"),
    );
    if (jsonEntries.length === 0) {
        throw new Error("The repository contains no JSON files.");
    }
    if (jsonEntries.length > 256) {
        throw new Error("The repository contains more than 256 JSON files.");
    }

    const maxFileBytes = 32 * 1024 * 1024;
    const maxTotalBytes = 128 * 1024 * 1024;
    let declaredTotal = 0;
    for (const entry of jsonEntries) {
        const size = Number(entry.size) || 0;
        if (size > maxFileBytes) {
            throw new Error(`${entry.path} is larger than 32 MiB.`);
        }
        declaredTotal += size;
    }
    if (declaredTotal > maxTotalBytes) {
        throw new Error("The repository's JSON files are larger than 128 MiB in total.");
    }

    const commit = tree.sha;
    const files = await Promise.all(jsonEntries.map(async (entry) => {
        const path = entry.path.split("/").map(encodeURIComponent).join("/");
        const rawUrl =
            `https://raw.githubusercontent.com/${encodeURIComponent(owner)}/` +
            `${encodeURIComponent(repository)}/${encodeURIComponent(commit)}/${path}`;
        const response = await fetch(rawUrl);
        if (!response.ok) {
            throw new Error(`Could not load ${entry.path}: ${response.status} ${response.statusText}.`);
        }
        return [entry.path, new Uint8Array(await response.arrayBuffer())];
    }));

    let actualTotal = 0;
    const result = [];
    for (const [name, bytes] of files) {
        actualTotal += bytes.byteLength;
        if (bytes.byteLength > maxFileBytes) {
            throw new Error(`${name} is larger than 32 MiB.`);
        }
        if (actualTotal > maxTotalBytes) {
            throw new Error("The repository's JSON files are larger than 128 MiB in total.");
        }
        result.push(name, bytes);
    }
    return result;
}

export function download_database(bytes) {
    const blob = new Blob([new Uint8Array(bytes)], { type: "application/vnd.sqlite3" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "idiosepius.db";
    anchor.style.display = "none";
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
}
"#)]
extern "C" {
    fn browser_history_init(depth: usize);
    fn browser_take_history_depth() -> i32;
    fn browser_current_history_depth() -> i32;
    fn browser_push_history(depth: usize);
    fn browser_replace_history(depth: usize);
    fn browser_go_history(delta: i32);
    fn browser_load_database() -> js_sys::Promise;
    fn browser_save_database(
        database: &js_sys::Uint8Array,
        wal: &js_sys::Uint8Array,
    ) -> js_sys::Promise;
    fn browser_pick_files(accept: &str, multiple: bool) -> js_sys::Promise;
    fn browser_load_github_repository(repository_url: &str) -> js_sys::Promise;
    fn download_database(bytes: &[u8]);
}
