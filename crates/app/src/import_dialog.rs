//! Shared import-source chooser.
//!
//! The shell still performs each operation, but both the native app and the
//! browser present the same three routes and the same error/loading states.

use eframe::egui::{self, Align2, Id, Sense, Stroke, Vec2};

use crate::import::EXAMPLE_REPOSITORIES;
use crate::theme::{Palette, text, tracked};

const PREFERRED_CONTENT_HEIGHT: f32 = 320.0;
const SCREEN_MARGIN: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ImportView {
    Sources,
    Examples,
    Github,
    Loading,
}

pub(crate) enum ImportAction {
    None,
    Close,
    LocalFiles,
    Show(ImportView),
    LoadRepository(String, ImportView),
}

pub(crate) fn show(
    ctx: &egui::Context,
    view: ImportView,
    github_url: &mut String,
    import_error: Option<&str>,
) -> ImportAction {
    let width = (ctx.content_rect().width() - 64.0).clamp(260.0, 560.0);
    let height =
        PREFERRED_CONTENT_HEIGHT.min((ctx.content_rect().height() - SCREEN_MARGIN).max(0.0));
    let frame = egui::Frame::new()
        .inner_margin(24)
        .fill(Palette::SURFACE)
        .stroke(Stroke::new(1.0, Palette::LINE_BRIGHT));
    let modal = egui::Modal::new(Id::new("import-deck-dialog"))
        .backdrop_color(Palette::BG.gamma_multiply(0.82))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width);
            egui::ScrollArea::vertical()
                .id_salt(("import-deck-scroll", view))
                .max_height(height)
                .auto_shrink([true, false])
                .show(ui, |ui| contents(ui, view, github_url, import_error))
                .inner
        });

    let should_close = view != ImportView::Loading && modal.should_close();
    if should_close {
        ImportAction::Close
    } else {
        modal.inner
    }
}

fn contents(
    ui: &mut egui::Ui,
    view: ImportView,
    github_url: &mut String,
    import_error: Option<&str>,
) -> ImportAction {
    ui.label(
        egui::RichText::new(tracked("import deck"))
            .font(text::title())
            .color(Palette::TEXT),
    );
    ui.add_space(8.0);

    match view {
        ImportView::Sources => {
            ui.label(
                egui::RichText::new("Choose where the JSON deck packs come from.")
                    .font(text::body())
                    .color(Palette::TEXT_DIM),
            );
            ui.add_space(12.0);
            if import_choice(
                ui,
                "import local files",
                "Pick one or more .json files, or a .zip archive.",
            ) {
                ImportAction::LocalFiles
            } else if import_choice(
                ui,
                "import examples",
                "Choose one of the included public course repositories.",
            ) {
                ImportAction::Show(ImportView::Examples)
            } else if import_choice(
                ui,
                "import from github",
                "Paste the URL of a public repository containing JSON packs.",
            ) {
                ImportAction::Show(ImportView::Github)
            } else {
                ImportAction::None
            }
        }
        ImportView::Examples => {
            ui.label(
                egui::RichText::new("Load every JSON pack from a public example repository.")
                    .font(text::body())
                    .color(Palette::TEXT_DIM),
            );
            ui.add_space(12.0);
            let mut action = ImportAction::None;
            for (label, url) in EXAMPLE_REPOSITORIES {
                if import_choice(ui, label, url) {
                    action = ImportAction::LoadRepository(url.into(), ImportView::Examples);
                }
            }
            ui.add_space(4.0);
            if back_button(ui) {
                ImportAction::Show(ImportView::Sources)
            } else {
                action
            }
        }
        ImportView::Github => {
            ui.label(
                egui::RichText::new(
                    "Paste a public repository URL. Every .json file in the repository is loaded.",
                )
                .font(text::body())
                .color(Palette::TEXT_DIM),
            );
            ui.add_space(12.0);
            let response = ui.add(
                egui::TextEdit::singleline(github_url)
                    .id_salt("github-repository-url")
                    .font(text::small())
                    .hint_text("https://github.com/owner/repository")
                    .desired_width(f32::INFINITY),
            );
            if let Some(error) = import_error {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(error)
                        .font(text::small())
                        .color(Palette::WRONG),
                );
            }
            ui.add_space(8.0);
            let enter = !github_url.trim().is_empty()
                && response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let load = ui
                .add_enabled(
                    !github_url.trim().is_empty(),
                    egui::Button::new(tracked("load repository")),
                )
                .clicked();
            if load || enter {
                ImportAction::LoadRepository(github_url.trim().to_owned(), ImportView::Github)
            } else if back_button(ui) {
                ImportAction::Show(ImportView::Sources)
            } else {
                ImportAction::None
            }
        }
        ImportView::Loading => {
            ui.label(
                egui::RichText::new("Loading repository JSON files…")
                    .font(text::body())
                    .color(Palette::TEXT_DIM),
            );
            ui.add_space(10.0);
            ui.add(egui::Spinner::new().color(Palette::ACCENT));
            ImportAction::None
        }
    }
}

fn import_choice(ui: &mut egui::Ui, label: &str, detail: &str) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 62.0), Sense::click());
    let colour = if response.hovered() {
        Palette::ACCENT
    } else {
        Palette::LINE_BRIGHT
    };
    let fill = if response.hovered() {
        Palette::CARD
    } else {
        Palette::SURFACE
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0, fill);
    painter.rect_stroke(rect, 0, Stroke::new(1.0, colour), egui::StrokeKind::Inside);
    painter.text(
        rect.left_top() + Vec2::new(16.0, 12.0),
        Align2::LEFT_TOP,
        tracked(label),
        text::label(),
        if response.hovered() {
            Palette::ACCENT
        } else {
            Palette::TEXT
        },
    );
    painter.text(
        rect.left_bottom() + Vec2::new(16.0, -12.0),
        Align2::LEFT_BOTTOM,
        detail,
        text::small(),
        Palette::TEXT_DIM,
    );
    response.clicked()
}

fn back_button(ui: &mut egui::Ui) -> bool {
    ui.add(egui::Button::new(tracked("back"))).clicked()
}
