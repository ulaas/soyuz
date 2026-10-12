//! Plain 2D geometry for app state (points in logical pixels), without a toolkit type.
//! Semantics match egui's `emath` where the GUI converts back and forth (`ui::geom`).

use std::ops::{Add, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// An axis-aligned rectangle; `min` is the top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

pub const fn pt(x: f32, y: f32) -> Point {
    Point { x, y }
}

pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Point {
    pub const ZERO: Point = pt(0.0, 0.0);
}

impl Vec2 {
    pub const ZERO: Vec2 = vec2(0.0, 0.0);
}

impl Add<Vec2> for Point {
    type Output = Point;
    fn add(self, v: Vec2) -> Point {
        pt(self.x + v.x, self.y + v.y)
    }
}

impl Sub<Vec2> for Point {
    type Output = Point;
    fn sub(self, v: Vec2) -> Point {
        pt(self.x - v.x, self.y - v.y)
    }
}

impl Rect {
    /// Contains nothing; growing it keeps it empty.
    pub const NOTHING: Rect = Rect { min: pt(f32::INFINITY, f32::INFINITY), max: pt(-f32::INFINITY, -f32::INFINITY) };

    pub fn from_min_size(min: Point, size: Vec2) -> Rect {
        Rect { min, max: min + size }
    }

    pub fn from_two_pos(a: Point, b: Point) -> Rect {
        Rect { min: pt(a.x.min(b.x), a.y.min(b.y)), max: pt(a.x.max(b.x), a.y.max(b.y)) }
    }

    /// Inclusive on every edge.
    pub fn contains(&self, p: Point) -> bool {
        self.min.x <= p.x && p.x <= self.max.x && self.min.y <= p.y && p.y <= self.max.y
    }

    /// Grow by `by` on every side.
    pub fn expand2(self, by: Vec2) -> Rect {
        Rect { min: self.min - by, max: self.max + by }
    }

    pub fn center(&self) -> Point {
        pt((self.min.x + self.max.x) / 2.0, (self.min.y + self.max.y) / 2.0)
    }

    pub fn top(&self) -> f32 {
        self.min.y
    }

    pub fn bottom(&self) -> f32 {
        self.max.y
    }

    pub fn left(&self) -> f32 {
        self.min.x
    }

    pub fn right(&self) -> f32 {
        self.max.x
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_basics() {
        let r = Rect::from_min_size(pt(10.0, 20.0), vec2(100.0, 50.0));
        assert!(r.contains(pt(10.0, 20.0)) && r.contains(pt(110.0, 70.0)));
        assert!(!r.contains(pt(110.1, 70.0)));
        assert_eq!(r.center(), pt(60.0, 45.0));
        assert!(r.expand2(vec2(0.0, 5.0)).contains(pt(50.0, 74.0)));
        assert!(!Rect::NOTHING.contains(Point::ZERO));
        assert!(!Rect::NOTHING.expand2(vec2(10.0, 10.0)).contains(Point::ZERO));
        assert_eq!(Rect::from_two_pos(pt(5.0, 1.0), pt(1.0, 5.0)), Rect::from_min_size(pt(1.0, 1.0), vec2(4.0, 4.0)));
    }
}
