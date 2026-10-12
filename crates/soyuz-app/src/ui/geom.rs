//! Conversions between the core's plain geometry (`crate::geom`) and egui's.

use crate::geom;

pub fn pos(p: geom::Point) -> egui::Pos2 {
    egui::pos2(p.x, p.y)
}

pub fn point(p: egui::Pos2) -> geom::Point {
    geom::pt(p.x, p.y)
}

pub fn vec(v: geom::Vec2) -> egui::Vec2 {
    egui::vec2(v.x, v.y)
}

pub fn size(v: egui::Vec2) -> geom::Vec2 {
    geom::vec2(v.x, v.y)
}

pub fn rect(r: geom::Rect) -> egui::Rect {
    egui::Rect::from_min_max(pos(r.min), pos(r.max))
}

pub fn area(r: egui::Rect) -> geom::Rect {
    geom::Rect { min: point(r.min), max: point(r.max) }
}
