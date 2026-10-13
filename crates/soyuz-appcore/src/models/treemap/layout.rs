//! Squarified treemap layout (Bruls, Huizing, van Wijk 2000). Pure geometry, no egui:
//! [`squarify`] turns weights sorted in descending order into rectangles whose areas are
//! proportional to the weights and whose aspect ratios stay close to 1. Moved from the GUI's
//! `ui/tools/treemap/layout.rs` (Phase T7).

/// Axis-aligned rectangle in layout space (`x`, `y` = top-left).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Tile {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Tile {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Tile { x, y, w, h }
    }

    pub fn area(&self) -> f64 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    /// Area of the intersection with `o`.
    pub fn overlap(&self, o: &Tile) -> f64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }

    pub fn aspect(&self) -> f64 {
        if self.w <= 0.0 || self.h <= 0.0 { f64::INFINITY } else { (self.w / self.h).max(self.h / self.w) }
    }
}

/// Worst aspect ratio of a row of areas laid along a side of length `side`.
fn worst(sum: f64, min: f64, max: f64, side: f64) -> f64 {
    if sum <= 0.0 || min <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = sum * sum;
    let w2 = side * side;
    (w2 * max / s2).max(s2 / (w2 * min))
}

/// Lay out `weights` (sorted descending; zero / negative weights get empty tiles) inside
/// `bounds`. The result is parallel to `weights`.
pub fn squarify(weights: &[f64], bounds: Tile) -> Vec<Tile> {
    let n = weights.len();
    let mut out = vec![Tile::new(bounds.x, bounds.y, 0.0, 0.0); n];
    let total: f64 = weights.iter().map(|w| w.max(0.0)).sum();
    if n == 0 || total <= 0.0 || bounds.area() <= 0.0 {
        return out;
    }
    let scale = bounds.area() / total;
    let areas: Vec<f64> = weights.iter().map(|w| w.max(0.0) * scale).collect();
    let mut r = bounds;
    let mut i = 0;
    while i < n {
        if areas[i] <= 0.0 {
            out[i] = Tile::new(r.x, r.y, 0.0, 0.0);
            i += 1;
            continue;
        }
        let side = r.w.min(r.h);
        // Grow the row while the worst aspect ratio improves.
        let mut end = i + 1;
        let mut sum = areas[i];
        let mut min = areas[i];
        let mut max = areas[i];
        let mut best = worst(sum, min, max, side);
        while end < n && areas[end] > 0.0 {
            let a = areas[end];
            let w = worst(sum + a, min.min(a), max.max(a), side);
            if w > best {
                break;
            }
            best = w;
            sum += a;
            min = min.min(a);
            max = max.max(a);
            end += 1;
        }
        // The last row absorbs the remaining space exactly (no float drift).
        let last = end >= n || areas[end..].iter().all(|a| *a <= 0.0);
        if r.w >= r.h {
            // Vertical strip on the left.
            let strip = if last { r.w } else { (sum / r.h).min(r.w) };
            let mut y = r.y;
            for k in i..end {
                let h = if k + 1 == end { r.y + r.h - y } else { areas[k] / strip };
                out[k] = Tile::new(r.x, y, strip, h.max(0.0));
                y += h;
            }
            r.x += strip;
            r.w -= strip;
        } else {
            // Horizontal strip on top.
            let strip = if last { r.h } else { (sum / r.w).min(r.h) };
            let mut x = r.x;
            for k in i..end {
                let w = if k + 1 == end { r.x + r.w - x } else { areas[k] / strip };
                out[k] = Tile::new(x, r.y, w.max(0.0), strip);
                x += w;
            }
            r.y += strip;
            r.h -= strip;
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(weights: &[f64], b: Tile) -> Vec<Tile> {
        let tiles = squarify(weights, b);
        assert_eq!(tiles.len(), weights.len());
        let total: f64 = weights.iter().sum();
        for (t, w) in tiles.iter().zip(weights) {
            let expect = w / total * b.area();
            assert!((t.area() - expect).abs() <= 1e-6 * b.area().max(1.0), "area {} vs {}", t.area(), expect);
            assert!(t.x >= b.x - 1e-9 && t.y >= b.y - 1e-9, "inside (min) {t:?}");
            assert!(t.x + t.w <= b.x + b.w + 1e-6 && t.y + t.h <= b.y + b.h + 1e-6, "inside (max) {t:?}");
        }
        for i in 0..tiles.len() {
            for j in i + 1..tiles.len() {
                assert!(tiles[i].overlap(&tiles[j]) < 1e-6, "tiles {i} and {j} overlap: {:?} {:?}", tiles[i], tiles[j]);
            }
        }
        let covered: f64 = tiles.iter().map(Tile::area).sum();
        assert!((covered - b.area()).abs() < 1e-6 * b.area(), "fills the bounds");
        tiles
    }

    #[test]
    fn paper_example_is_proportional_and_disjoint() {
        // The example from the paper: 6x4 with areas 6,6,4,3,2,2,1.
        let tiles = check(&[6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0], Tile::new(0.0, 0.0, 6.0, 4.0));
        // First row: the two 6s side by side in a 3x4 strip → aspect 1.5 at worst.
        assert!(tiles[0].aspect() <= 1.5 + 1e-9);
        assert!(tiles[1].aspect() <= 1.5 + 1e-9);
    }

    #[test]
    fn many_uneven_weights() {
        let mut w: Vec<f64> = (1..=60).map(|i| ((i * 7919) % 1000 + 1) as f64).collect();
        w.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let tiles = check(&w, Tile::new(10.0, 20.0, 800.0, 450.0));
        // Squarified tiles stay reasonably square (the first, largest ones especially).
        assert!(tiles[0].aspect() < 3.0, "aspect {}", tiles[0].aspect());
    }

    #[test]
    fn single_and_tall_bounds() {
        let t = check(&[5.0], Tile::new(0.0, 0.0, 10.0, 3.0));
        assert_eq!(t[0], Tile::new(0.0, 0.0, 10.0, 3.0));
        check(&[9.0, 3.0, 3.0, 1.0], Tile::new(0.0, 0.0, 2.0, 50.0));
    }

    #[test]
    fn zero_weights_and_empty_input() {
        assert!(squarify(&[], Tile::new(0.0, 0.0, 1.0, 1.0)).is_empty());
        let t = squarify(&[4.0, 0.0], Tile::new(0.0, 0.0, 2.0, 2.0));
        assert!((t[0].area() - 4.0).abs() < 1e-9);
        assert_eq!(t[1].area(), 0.0);
        let t = squarify(&[0.0, 0.0], Tile::new(0.0, 0.0, 2.0, 2.0));
        assert!(t.iter().all(|t| t.area() == 0.0));
    }
}
