//! Ordered prose and figures, laid out as one card-local block.
//!
//! Questions and facts share the same authored representation. Keeping the
//! stacking here means a tilted card and an ordinary review column cannot
//! disagree about where an interspersed figure belongs.

use eframe::egui::{Color32, Painter, Pos2, Sense, Ui, Vec2};
use idiosepius_core::ContentBlock;

use crate::plot::{self, Plot};
use crate::richtext::{self, Doc};

const GAP: f32 = 12.0;

#[derive(Clone)]
pub struct Blocks {
    pub size: Vec2,
    items: Vec<Item>,
}

#[derive(Clone)]
enum Item {
    Text { pos: Pos2, doc: Doc },
    Figure { pos: Pos2, plot: Plot },
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
                Item::Figure { pos, plot } => {
                    plot.paint_rotated(painter, *pos + top_left.to_vec2(), pivot, angle, opacity)
                }
            }
        }
    }
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
    let (rect, _) = ui.allocate_exact_size(blocks.size, Sense::hover());
    blocks.paint(ui.painter(), rect.min, color, 1.0);
}
