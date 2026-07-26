//! Ordered prose and figures, laid out as one card-local block.
//!
//! Questions and facts share the same authored representation. Keeping the
//! stacking here means a tilted card and an ordinary review column cannot
//! disagree about where an interspersed figure belongs.

use eframe::egui::{Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use idiosepius_core::{ContentBlock, Figure};

use crate::card;
use crate::plot::{self, Plot};
use crate::richtext::{self, Doc};
use crate::theme::Palette;

const GAP: f32 = 12.0;
const ZOOM_REQUEST: &str = "plot-zoom-request";

#[derive(Clone)]
pub struct Blocks {
    pub size: Vec2,
    items: Vec<Item>,
}

#[derive(Clone)]
enum Item {
    Text {
        pos: Pos2,
        doc: Doc,
    },
    Figure {
        pos: Pos2,
        plot: Plot,
        figure: Figure,
    },
}

impl Blocks {
    pub fn height(&self) -> f32 {
        self.size.y
    }

    pub fn paint(&self, painter: &Painter, top_left: Pos2, color: Color32, opacity: f32) {
        self.paint_rotated(painter, top_left, top_left, 0.0, color, opacity);
    }

    pub fn paint_rotated(
        &self,
        painter: &Painter,
        top_left: Pos2,
        pivot: Pos2,
        angle: f32,
        color: Color32,
        opacity: f32,
    ) {
        for item in &self.items {
            match item {
                Item::Text { pos, doc } => doc.paint_rotated(
                    painter,
                    *pos + top_left.to_vec2(),
                    pivot,
                    angle,
                    color,
                    opacity,
                ),
                Item::Figure { pos, plot, .. } => {
                    plot.paint_rotated(painter, *pos + top_left.to_vec2(), pivot, angle, opacity)
                }
            }
        }
    }

    /// Register each figure as a zoom target and return whether one was
    /// clicked. The exact hit test is performed in the card's rotated
    /// coordinate system, so a tilted plot does not gain clickable corners.
    pub fn interact_figures(
        &self,
        ui: &mut Ui,
        top_left: Pos2,
        pivot: Pos2,
        angle: f32,
        id: Id,
    ) -> bool {
        let mut clicked = false;
        for (index, item) in self.items.iter().enumerate() {
            let Item::Figure { pos, plot, figure } = item else {
                continue;
            };
            let local_rect = Rect::from_min_size(*pos + top_left.to_vec2(), plot.size);
            let corners = card::corners(local_rect, pivot, angle);
            let hit_rect = Rect::from_points(&corners);
            let pointer = ui.ctx().pointer_hover_pos();
            let exact_hover = pointer
                .is_some_and(|point| local_rect.contains(card::rotate(point, pivot, -angle)));
            let response = ui.interact(hit_rect, id.with(index), Sense::click());

            if exact_hover {
                ui.ctx().set_cursor_icon(CursorIcon::ZoomIn);
                ui.painter().add(Shape::closed_line(
                    corners,
                    Stroke::new(1.0, Palette::ACCENT),
                ));
            }
            if exact_hover && response.clicked() {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(Id::new(ZOOM_REQUEST), figure.clone());
                });
                clicked = true;
            }
        }
        clicked
    }
}

pub fn take_zoom_request(ctx: &eframe::egui::Context) -> Option<Figure> {
    ctx.data_mut(|data| {
        let id = Id::new(ZOOM_REQUEST);
        let request = data.get_temp(id);
        data.remove::<Figure>(id);
        request
    })
}

pub fn layout(painter: &Painter, authored: &[ContentBlock], text_size: f32, width: f32) -> Blocks {
    let mut items = Vec::new();
    let mut y = 0.0;

    for block in authored {
        if matches!(block, ContentBlock::Text(text) if text.trim().is_empty()) {
            continue;
        }
        if !items.is_empty() {
            y += GAP;
        }

        match block {
            ContentBlock::Text(text) => {
                let doc = richtext::layout(painter, text, text_size, width);
                let height = doc.height();
                items.push(Item::Text {
                    pos: Pos2::new(0.0, y),
                    doc,
                });
                y += height;
            }
            ContentBlock::Figure { figure } => {
                let plot = plot::layout(painter, figure, width);
                let height = plot.height();
                items.push(Item::Figure {
                    pos: Pos2::new(0.0, y),
                    plot,
                    figure: figure.clone(),
                });
                y += height;
            }
        }
    }

    Blocks {
        size: Vec2::new(width, y),
        items,
    }
}

pub fn show(ui: &mut Ui, authored: &[ContentBlock], text_size: f32, color: Color32) {
    let width = ui.available_width().max(40.0);
    let blocks = layout(ui.painter(), authored, text_size, width);
    let (rect, response) = ui.allocate_exact_size(blocks.size, Sense::hover());
    blocks.paint(ui.painter(), rect.min, color, 1.0);
    blocks.interact_figures(ui, rect.min, rect.min, 0.0, response.id.with("figure"));
}
