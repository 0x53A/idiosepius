//! Card-local plot layout and rigidly rotated painting.
//!
//! Transfer-function arithmetic lives in `idiosepius_core::figure`. This
//! module only turns its data-space geometry into the same explicit points and
//! text shapes that the rest of a tilted swipe card uses.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontId, Id, Painter, Pos2, Rect, Stroke, TextureHandle, TextureOptions, Vec2,
};
use eframe::epaint::{Galley, Mesh, Shape, TextShape};
use idiosepius_core::Figure;
use idiosepius_core::figure::{Axis, Guide, Panel};

use crate::card::rotate;
use crate::math::{self, Formula};
use crate::theme::Palette;

const LEFT: f32 = 48.0;
const RIGHT: f32 = 12.0;
const TOP: f32 = 12.0;
const BOTTOM: f32 = 30.0;

/// A figure laid out in local card coordinates.
#[derive(Clone)]
pub struct Plot {
    pub size: Vec2,
    shapes: Vec<PlotShape>,
}

#[derive(Clone)]
enum PlotShape {
    Line {
        points: Vec<Pos2>,
        stroke: Stroke,
    },
    Text {
        pos: Pos2,
        galley: Arc<Galley>,
        color: Color32,
    },
    Formula {
        pos: Pos2,
        formula: Formula,
        color: Color32,
    },
    Image {
        rect: Rect,
        texture: TextureHandle,
    },
}

impl Plot {
    pub fn height(&self) -> f32 {
        self.size.y
    }

    pub fn paint_rotated(
        &self,
        painter: &Painter,
        top_left: Pos2,
        pivot: Pos2,
        angle: f32,
        opacity: f32,
    ) {
        let at = |p: Pos2| rotate(p + top_left.to_vec2(), pivot, angle);
        for shape in &self.shapes {
            match shape {
                PlotShape::Line { points, stroke } => {
                    painter.add(Shape::line(
                        points.iter().map(|p| at(*p)).collect(),
                        Stroke::new(stroke.width, stroke.color.gamma_multiply(opacity)),
                    ));
                }
                PlotShape::Text { pos, galley, color } => {
                    let ink = color.gamma_multiply(opacity);
                    painter.add(
                        TextShape::new(at(*pos), galley.clone(), ink)
                            .with_override_text_color(ink)
                            .with_angle(angle),
                    );
                }
                PlotShape::Formula {
                    pos,
                    formula,
                    color,
                } => formula.paint_rotated(
                    painter,
                    *pos + top_left.to_vec2(),
                    pivot,
                    angle,
                    *color,
                    opacity,
                ),
                PlotShape::Image { rect, texture } => {
                    let screen = rect.translate(top_left.to_vec2());
                    let mut mesh = Mesh::with_texture(texture.id());
                    mesh.add_rect_with_uv(
                        screen,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE.gamma_multiply(opacity),
                    );
                    for vertex in &mut mesh.vertices {
                        vertex.pos = rotate(vertex.pos, pivot, angle);
                    }
                    painter.add(Shape::mesh(mesh));
                }
            }
        }
    }
}

/// Lay out one authored figure at a card-local width.
pub fn layout(painter: &Painter, spec: &Figure, width: f32) -> Plot {
    layout_with_height(painter, spec, width, None)
}

/// Lay out an enlarged figure for the plot modal.
pub fn layout_large(painter: &Painter, spec: &Figure, width: f32, height: f32) -> Plot {
    layout_with_height(painter, spec, width, Some(height))
}

fn layout_with_height(painter: &Painter, spec: &Figure, width: f32, height: Option<f32>) -> Plot {
    match spec {
        Figure::Svg { src } => layout_svg(painter, src, width, height),
        _ => spec
            .plot()
            .ok()
            .flatten()
            .map(|data| layout_vector(painter, &data.panels, width, height))
            .unwrap_or_else(|| error_plot(painter, width, "invalid figure")),
    }
}

fn layout_vector(
    painter: &Painter,
    panels: &[Panel],
    width: f32,
    available_height: Option<f32>,
) -> Plot {
    let panel_height = available_height.map_or_else(
        || if panels.len() > 1 { 124.0 } else { 188.0 },
        |height| {
            (height / panels.len().max(1) as f32).clamp(
                if panels.len() > 1 { 150.0 } else { 220.0 },
                if panels.len() > 1 { 300.0 } else { 520.0 },
            )
        },
    );
    let mut shapes = Vec::new();
    for (index, panel) in panels.iter().enumerate() {
        layout_panel(painter, panel, index, width, panel_height, &mut shapes);
    }
    Plot {
        size: Vec2::new(width, panel_height * panels.len() as f32),
        shapes,
    }
}

fn layout_panel(
    painter: &Painter,
    panel: &Panel,
    index: usize,
    width: f32,
    panel_height: f32,
    shapes: &mut Vec<PlotShape>,
) {
    let offset_y = index as f32 * panel_height;
    let graph = Rect::from_min_max(
        Pos2::new(LEFT, offset_y + TOP),
        Pos2::new(width - RIGHT, offset_y + panel_height - BOTTOM),
    );

    for tick in &panel.x.ticks {
        let x = map(tick.value, &panel.x, graph.left(), graph.right());
        shapes.push(PlotShape::Line {
            points: vec![Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())],
            stroke: Stroke::new(1.0, Palette::LINE),
        });
        let label = math::layout(painter, &tick.label, 9.5);
        shapes.push(PlotShape::Formula {
            pos: Pos2::new(x - label.size.x / 2.0, graph.bottom() + 4.0),
            formula: label,
            color: Palette::TEXT_FAINT,
        });
    }
    for tick in &panel.y.ticks {
        let y = map_y(tick.value, &panel.y, graph.top(), graph.bottom());
        shapes.push(PlotShape::Line {
            points: vec![Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)],
            stroke: Stroke::new(1.0, Palette::LINE),
        });
        let label = math::layout(painter, &tick.label, 9.5);
        shapes.push(PlotShape::Formula {
            pos: Pos2::new(graph.left() - label.size.x - 6.0, y - label.size.y / 2.0),
            formula: label,
            color: Palette::TEXT_FAINT,
        });
    }

    for guide in &panel.guides {
        let points = match *guide {
            Guide::Horizontal(value) => {
                let y = map_y(value, &panel.y, graph.top(), graph.bottom());
                vec![Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)]
            }
            Guide::Vertical(value) => {
                let x = map(value, &panel.x, graph.left(), graph.right());
                vec![Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())]
            }
        };
        shapes.push(PlotShape::Line {
            points,
            stroke: Stroke::new(1.2, Palette::LINE_BRIGHT),
        });
    }

    // Axes are hairlines; if zero is visible it gets the brighter line.
    let x_axis_y = if panel.y.min <= 0.0 && panel.y.max >= 0.0 {
        map_y(0.0, &panel.y, graph.top(), graph.bottom())
    } else {
        graph.bottom()
    };
    let y_axis_x = if panel.x.min <= 0.0 && panel.x.max >= 0.0 {
        map(0.0, &panel.x, graph.left(), graph.right())
    } else {
        graph.left()
    };
    shapes.push(PlotShape::Line {
        points: vec![
            Pos2::new(graph.left(), x_axis_y),
            Pos2::new(graph.right(), x_axis_y),
        ],
        stroke: Stroke::new(1.0, Palette::LINE_BRIGHT),
    });
    shapes.push(PlotShape::Line {
        points: vec![
            Pos2::new(y_axis_x, graph.top()),
            Pos2::new(y_axis_x, graph.bottom()),
        ],
        stroke: Stroke::new(1.0, Palette::LINE_BRIGHT),
    });

    let line_color = if index == 0 {
        Palette::ACCENT
    } else {
        Palette::VIOLET
    };
    for line in &panel.lines {
        let points = line
            .points
            .iter()
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .map(|p| {
                Pos2::new(
                    map(p[0], &panel.x, graph.left(), graph.right()),
                    map_y(p[1], &panel.y, graph.top(), graph.bottom()),
                )
            })
            .collect::<Vec<_>>();
        if points.len() >= 2 {
            shapes.push(PlotShape::Line {
                points,
                stroke: Stroke::new(1.7, line_color),
            });
        }
    }

    for marker in &panel.markers {
        let at = Pos2::new(
            map(marker.point[0], &panel.x, graph.left(), graph.right()),
            map_y(marker.point[1], &panel.y, graph.top(), graph.bottom()),
        );
        let radius = 5.0;
        shapes.push(PlotShape::Line {
            points: vec![
                at + Vec2::new(-radius, -radius),
                at + Vec2::new(radius, radius),
            ],
            stroke: Stroke::new(1.7, Palette::TEXT),
        });
        shapes.push(PlotShape::Line {
            points: vec![
                at + Vec2::new(-radius, radius),
                at + Vec2::new(radius, -radius),
            ],
            stroke: Stroke::new(1.7, Palette::TEXT),
        });
    }

    for arrow in &panel.arrows {
        let tip = Pos2::new(
            map(arrow.at[0], &panel.x, graph.left(), graph.right()),
            map_y(arrow.at[1], &panel.y, graph.top(), graph.bottom()),
        );
        let toward = Pos2::new(
            map(arrow.toward[0], &panel.x, graph.left(), graph.right()),
            map_y(arrow.toward[1], &panel.y, graph.top(), graph.bottom()),
        );
        let direction = toward - tip;
        if direction.length_sq() > 0.01 {
            let direction = direction.normalized();
            let normal = Vec2::new(-direction.y, direction.x);
            let tail = tip - direction * 14.0;
            shapes.push(PlotShape::Line {
                points: vec![tail, tip],
                stroke: Stroke::new(1.9, line_color),
            });
            shapes.push(PlotShape::Line {
                points: vec![
                    tip - direction * 6.0 + normal * 4.0,
                    tip,
                    tip - direction * 6.0 - normal * 4.0,
                ],
                stroke: Stroke::new(1.9, line_color),
            });
        }
    }

    let x_label = math::layout(painter, panel.x_label, 11.0);
    shapes.push(PlotShape::Formula {
        pos: Pos2::new(
            graph.center().x - x_label.size.x / 2.0,
            graph.bottom() + 16.0,
        ),
        formula: x_label,
        color: Palette::TEXT_DIM,
    });
    let y_label = math::layout(painter, panel.y_label, 11.0);
    shapes.push(PlotShape::Formula {
        pos: Pos2::new(graph.left() + 5.0, graph.top() + 3.0),
        formula: y_label,
        color: Palette::TEXT_DIM,
    });
}

fn map(value: f64, axis: &Axis, low: f32, high: f32) -> f32 {
    let t = ((value - axis.min) / (axis.max - axis.min)).clamp(0.0, 1.0) as f32;
    low + t * (high - low)
}

fn map_y(value: f64, axis: &Axis, top: f32, bottom: f32) -> f32 {
    bottom - (map(value, axis, 0.0, 1.0) * (bottom - top))
}

#[derive(Clone)]
struct CachedSvg {
    texture: TextureHandle,
    aspect: f32,
}

fn layout_svg(painter: &Painter, src: &str, width: f32, max_height: Option<f32>) -> Plot {
    let mut hasher = DefaultHasher::new();
    src.hash(&mut hasher);
    let hash = hasher.finish();
    let id = Id::new(("figure-svg", hash));

    let cached = painter.ctx().data(|data| data.get_temp::<CachedSvg>(id));
    let cached = match cached {
        Some(cached) => cached,
        None => match rasterize_svg(painter, src, width, hash) {
            Ok(cached) => {
                painter
                    .ctx()
                    .data_mut(|data| data.insert_temp(id, cached.clone()));
                cached
            }
            Err(error) => return error_plot(painter, width, &error),
        },
    };

    let height = (width / cached.aspect).clamp(80.0, max_height.unwrap_or(280.0));
    Plot {
        size: Vec2::new(width, height),
        shapes: vec![PlotShape::Image {
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height)),
            texture: cached.texture,
        }],
    }
}

fn rasterize_svg(painter: &Painter, src: &str, width: f32, hash: u64) -> Result<CachedSvg, String> {
    let mut options = resvg::usvg::Options {
        font_family: "Hack".into(),
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    for font in egui::FontDefinitions::default().font_data.into_values() {
        options.fontdb_mut().load_font_data(font.font.to_vec());
    }
    // A pack is self-contained. Data URLs remain available, but an authored
    // SVG must never reach into the host filesystem for an image.
    let tree = resvg::usvg::Tree::from_str(src, &options).map_err(|e| e.to_string())?;
    let aspect = tree.size().width() / tree.size().height();
    let pixel_width = (width * 2.0).round().clamp(64.0, 1600.0) as u32;
    let pixel_height = ((pixel_width as f32 / aspect).round() as u32).clamp(32, 1200);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixel_width, pixel_height)
        .ok_or_else(|| "SVG dimensions are too large".to_owned())?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        pixel_width as f32 / tree.size().width(),
        pixel_height as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [pixel_width as usize, pixel_height as usize],
        pixmap.data(),
    );
    Ok(CachedSvg {
        texture: painter.ctx().load_texture(
            format!("figure-svg-{hash:016x}"),
            image,
            TextureOptions::LINEAR,
        ),
        aspect,
    })
}

fn error_plot(painter: &Painter, width: f32, message: &str) -> Plot {
    let height = 70.0;
    let galley = painter.layout(
        format!("figure error: {message}"),
        FontId::monospace(11.0),
        Palette::WRONG,
        width - 20.0,
    );
    Plot {
        size: Vec2::new(width, height),
        shapes: vec![
            PlotShape::Line {
                points: vec![Pos2::new(0.0, 0.0), Pos2::new(width, 0.0)],
                stroke: Stroke::new(1.0, Palette::WRONG),
            },
            PlotShape::Text {
                pos: Pos2::new(10.0, 10.0),
                galley,
                color: Palette::WRONG,
            },
        ],
    }
}
