//! Ink data and geometry: strokes, input smoothing, outlines and erasing.
//!
//! Coordinates are page units (CSS pixels at 100% zoom, 96 per inch).

use std::rc::Rc;

use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const PX_PER_MM: f32 = 96.0 / 25.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Pen,
    Highlighter,
}

/// A stroke color. `Ink` is the adaptive default: black on light pages,
/// white on dark ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkColor {
    Ink,
    Rgb(u8, u8, u8),
}

impl InkColor {
    pub fn to_hex(self) -> String {
        match self {
            InkColor::Ink => "ink".into(),
            InkColor::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        if s == "ink" {
            return Some(InkColor::Ink);
        }
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(InkColor::Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    /// Display color as linear-ish 0..1 RGB for the given theme.
    pub fn resolve(self, dark: bool) -> (f32, f32, f32) {
        match self {
            InkColor::Ink if dark => (0.92, 0.92, 0.92),
            InkColor::Ink => (0.11, 0.11, 0.11),
            InkColor::Rgb(r, g, b) => {
                let c = |v: u8| v as f32 / 255.0;
                let (r, g, b) = (c(r), c(g), c(b));
                if dark {
                    // Lift colors a bit so they stay readable on dark pages.
                    let lift = |v: f32| v + (1.0 - v) * 0.2;
                    (lift(r), lift(g), lift(b))
                } else {
                    (r, g, b)
                }
            }
        }
    }
}

impl Serialize for InkColor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for InkColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        InkColor::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("bad color {s:?}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// Pen pressure 0..1 (0.5 for mice).
    pub p: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bounds {
    fn of(points: &[Point], pad: f32) -> Self {
        let mut b = Bounds { x0: f32::MAX, y0: f32::MAX, x1: f32::MIN, y1: f32::MIN };
        for p in points {
            b.x0 = b.x0.min(p.x);
            b.y0 = b.y0.min(p.y);
            b.x1 = b.x1.max(p.x);
            b.y1 = b.y1.max(p.y);
        }
        Bounds { x0: b.x0 - pad, y0: b.y0 - pad, x1: b.x1 + pad, y1: b.y1 + pad }
    }

    fn near(&self, x: f32, y: f32, r: f32) -> bool {
        x >= self.x0 - r && x <= self.x1 + r && y >= self.y0 - r && y <= self.y1 + r
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub tool: Tool,
    pub color: InkColor,
    /// Nominal width in page units.
    pub width: f32,
    /// Fountain pen: width follows pressure. Otherwise a ballpoint with an
    /// almost even line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fountain: bool,
    #[serde(with = "packed_points")]
    pub points: Vec<Point>,
    #[serde(skip)]
    pub bounds: Bounds,
}

impl Stroke {
    pub fn new(tool: Tool, color: InkColor, width: f32) -> Self {
        Stroke { tool, color, width, fountain: false, points: Vec::new(), bounds: Bounds::default() }
    }

    pub fn update_bounds(&mut self) {
        self.bounds = Bounds::of(&self.points, self.width * 1.0);
    }

    /// Radius of the stroke at a point with the given pressure.
    fn radius(&self, p: f32) -> f32 {
        match self.tool {
            // Nominal width at half pressure. Fountain: thin at a light
            // touch, up to ~1.8x when pressing hard; ballpoint: 0.7x-1.3x.
            Tool::Pen if self.fountain => self.width / 2.0 * (0.2 + 1.6 * p.clamp(0.0, 1.0)),
            Tool::Pen => self.width / 2.0 * (0.7 + 0.6 * p.clamp(0.0, 1.0)),
            Tool::Highlighter => self.width / 2.0,
        }
    }

    /// Closed outline polygon of the stroke (left side, end cap, right side
    /// back, start cap). Render it filled with the non-zero rule.
    pub fn outline(&self) -> Vec<(f32, f32)> {
        let pts = &self.points;
        match pts.len() {
            0 => return Vec::new(),
            1 => return circle(pts[0].x, pts[0].y, self.radius(pts[0].p).max(0.5)),
            _ => {}
        }
        let pts = &spline(pts);
        let n = pts.len();
        let mut left = Vec::with_capacity(n);
        let mut right = Vec::with_capacity(n);
        let mut dirs = Vec::with_capacity(n);
        for i in 0..n {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len = (dx * dx + dy * dy).sqrt().max(1e-4);
            let (tx, ty) = (dx / len, dy / len);
            let r = self.radius(pts[i].p).max(0.3);
            left.push((pts[i].x - ty * r, pts[i].y + tx * r));
            right.push((pts[i].x + ty * r, pts[i].y - tx * r));
            dirs.push((tx, ty, r));
        }
        let mut out = left;
        // End cap: half circle around the last point, from left to right.
        let (tx, ty, r) = dirs[n - 1];
        out.extend(half_circle(pts[n - 1].x, pts[n - 1].y, r, (tx, ty)));
        out.extend(right.into_iter().rev());
        let (tx, ty, r) = dirs[0];
        out.extend(half_circle(pts[0].x, pts[0].y, r, (-tx, -ty)));
        // ~0.02 mm: invisible even zoomed in.
        reduce(out, 0.15)
    }

    /// Whether the eraser circle touches the stroke.
    pub fn hit(&self, x: f32, y: f32, r: f32) -> bool {
        if !self.bounds.near(x, y, r) {
            return false;
        }
        let reach = |p: &Point| r + self.radius(p.p);
        if self.points.len() == 1 {
            let p = &self.points[0];
            return dist2(p.x, p.y, x, y) <= reach(p).powi(2);
        }
        self.points
            .windows(2)
            .any(|w| seg_dist2(x, y, &w[0], &w[1]) <= reach(&w[0]).max(reach(&w[1])).powi(2))
    }

    /// Cut away the parts within the eraser circle. Returns `None` if the
    /// stroke isn't touched, otherwise the remaining pieces.
    pub fn split_by_eraser(&self, x: f32, y: f32, r: f32) -> Option<Vec<Stroke>> {
        if !self.hit(x, y, r) {
            return None;
        }
        // Resample so the cut is precise even when points are far apart.
        let step = (r / 3.0).max(0.5);
        let mut dense = Vec::with_capacity(self.points.len());
        for w in self.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let d = dist2(a.x, a.y, b.x, b.y).sqrt();
            let steps = (d / step).ceil().max(1.0) as usize;
            for k in 0..steps {
                let t = k as f32 / steps as f32;
                dense.push(Point {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    p: a.p + (b.p - a.p) * t,
                });
            }
        }
        dense.extend(self.points.last());

        let mut pieces = Vec::new();
        let mut current: Vec<Point> = Vec::new();
        for p in dense {
            if dist2(p.x, p.y, x, y) <= r * r {
                if current.len() > 1 {
                    pieces.push(std::mem::take(&mut current));
                }
                current.clear();
            } else {
                current.push(p);
            }
        }
        if current.len() > 1 {
            pieces.push(current);
        }
        Some(
            pieces
                .into_iter()
                .map(|points| {
                    let mut s = Stroke { points: simplify(points, step * 0.5), ..self.clone() };
                    s.update_bounds();
                    s
                })
                .collect(),
        )
    }
}

fn dist2(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    (ax - bx).powi(2) + (ay - by).powi(2)
}

fn seg_dist2(x: f32, y: f32, a: &Point, b: &Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((x - a.x) * dx + (y - a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    dist2(x, y, a.x + dx * t, a.y + dy * t)
}

/// Drop outline points within `eps` of the line between their kept
/// neighbors (Ramer-Douglas-Peucker), so files and draws stay small.
fn reduce(pts: Vec<(f32, f32)>, eps: f32) -> Vec<(f32, f32)> {
    if pts.len() < 3 {
        return pts;
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let len = (dx * dx + dy * dy).sqrt();
        let far = (a + 1..b)
            .map(|i| {
                let (px, py) = (pts[i].0 - pa.0, pts[i].1 - pa.1);
                let d = if len > 1e-6 { (px * dy - py * dx).abs() / len } else { (px * px + py * py).sqrt() };
                (i, d)
            })
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((i, d)) = far
            && d > eps
        {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    pts.into_iter().zip(keep).filter_map(|(p, k)| k.then_some(p)).collect()
}

/// Catmull-Rom curve through the points, at most one point per 2 page units,
/// so sparse fast strokes come out round instead of faceted.
fn spline(points: &[Point]) -> Vec<Point> {
    let n = points.len();
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n - 1 {
        let (a, b, c, d) = (points[i.saturating_sub(1)], points[i], points[i + 1], points[(i + 2).min(n - 1)]);
        let steps = (dist2(b.x, b.y, c.x, c.y).sqrt() / 2.0).ceil().clamp(1.0, 8.0) as usize;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let cr = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
                    + (3.0 * b - a - 3.0 * c + d) * t * t * t)
            };
            out.push(Point { x: cr(a.x, b.x, c.x, d.x), y: cr(a.y, b.y, c.y, d.y), p: b.p + (c.p - b.p) * t });
        }
    }
    out.push(points[n - 1]);
    out
}

/// Drop points closer than `min` to the previous kept point.
fn simplify(points: Vec<Point>, min: f32) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    let last = points.last().copied();
    for p in points {
        if out.last().is_none_or(|q| dist2(p.x, p.y, q.x, q.y) >= min * min) {
            out.push(p);
        }
    }
    if let (Some(l), Some(o)) = (last, out.last())
        && *o != l {
            out.push(l);
        }
    out
}

fn circle(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    (0..16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Points on a half circle bulging in direction `(tx, ty)`, going from the
/// left side (relative to the direction) to the right side; the end points
/// themselves are excluded (they're already on the outline).
fn half_circle(cx: f32, cy: f32, r: f32, (tx, ty): (f32, f32)) -> Vec<(f32, f32)> {
    let (lx, ly) = (-ty, tx);
    (1..8)
        .map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::PI;
            let (c, s) = (a.cos(), a.sin());
            (cx + r * (c * lx + s * tx), cy + r * (c * ly + s * ty))
        })
        .collect()
}

/// Live stabilizer ("pulled string"): the drawn point trails the pen on a
/// string of length `radius` and only moves once the string is taut, so
/// small wobbles never reach the line. Pressure is passed through.
pub struct Smoother {
    radius: f32,
    anchor: Option<Point>,
}

impl Smoother {
    pub fn new(radius: f32) -> Self {
        Smoother { radius: radius.max(0.0), anchor: None }
    }

    /// Feed a raw sample; returns the new stabilized point, or `None` if
    /// the pen is still within the slack of the string.
    pub fn push(&mut self, raw: Point) -> Option<Point> {
        let Some(a) = self.anchor else {
            self.anchor = Some(raw);
            return Some(raw);
        };
        let d = dist2(a.x, a.y, raw.x, raw.y).sqrt();
        // Ease pressure so sensor noise doesn't make the edges bumpy.
        let p = a.p + (raw.p - a.p) * 0.35;
        if d <= self.radius {
            // Keep the pressure current even while the point doesn't move.
            self.anchor = Some(Point { p, ..a });
            return None;
        }
        let k = (d - self.radius) / d;
        let out = Point { x: a.x + (raw.x - a.x) * k, y: a.y + (raw.y - a.y) * k, p };
        self.anchor = Some(out);
        Some(out)
    }
}

/// Curve-fit a finished stroke: `passes` rounds of a [1, 2, 1] / 4 kernel
/// over positions and pressure, keeping both end points in place.
pub fn smooth_points(points: &[Point], passes: usize) -> Vec<Point> {
    let mut pts = points.to_vec();
    if pts.len() < 3 {
        return pts;
    }
    for _ in 0..passes {
        let prev = pts.clone();
        for i in 1..prev.len() - 1 {
            let (a, b, c) = (prev[i - 1], prev[i], prev[i + 1]);
            pts[i] = Point {
                x: (a.x + 2.0 * b.x + c.x) / 4.0,
                y: (a.y + 2.0 * b.y + c.y) / 4.0,
                p: (a.p + 2.0 * b.p + c.p) / 4.0,
            };
        }
    }
    pts
}

/// Append a point unless it's (almost) on top of the previous one.
pub fn push_point(points: &mut Vec<Point>, p: Point) {
    match points.last_mut() {
        Some(q) if dist2(p.x, p.y, q.x, q.y) < 0.25 => q.p = q.p.max(p.p),
        _ => points.push(p),
    }
}

/// Strokes on a page, shared cheaply between undo snapshots.
pub type Strokes = Vec<Rc<Stroke>>;

mod packed_points {
    //! Points as base64 of little-endian f32 triples (x, y, pressure).
    use super::*;

    pub fn serialize<S: Serializer>(points: &[Point], s: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(points.len() * 12);
        for p in points {
            for v in [p.x, p.y, p.p] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Point>, D::Error> {
        let s = String::deserialize(d)?;
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(s).map_err(serde::de::Error::custom)?;
        let f = |c: &[u8]| f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        Ok(bytes.as_chunks::<12>().0.iter().map(|c| Point { x: f(&c[0..4]), y: f(&c[4..8]), p: f(&c[8..12]) }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x0: f32, x1: f32) -> Stroke {
        let mut s = Stroke::new(Tool::Pen, InkColor::Ink, 2.0);
        let mut x = x0;
        while x <= x1 {
            s.points.push(Point { x, y: 10.0, p: 0.5 });
            x += 1.0;
        }
        s.update_bounds();
        s
    }

    #[test]
    fn roundtrip_json() {
        let mut s = line(0.0, 5.0);
        s.color = InkColor::Rgb(0x35, 0x84, 0xe4);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"#3584e4\""));
        let back: Stroke = serde_json::from_str(&json).unwrap();
        assert_eq!(back.points, s.points);
        assert_eq!(back.color, s.color);
    }

    #[test]
    fn outline_is_closed_around_points() {
        let s = line(0.0, 20.0);
        let o = s.outline();
        // Straight sides collapse to their ends; the round caps stay.
        assert!(o.len() > 8 && o.len() < 30, "{} outline points", o.len());
        // Width at half pressure equals the nominal width.
        let ys: Vec<f32> = o.iter().map(|p| p.1).collect();
        let (min, max) = ys.iter().fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
        assert!((max - min - 2.0).abs() < 0.05, "outline height {}", max - min);
        // Caps extend past the end points.
        let xs: Vec<f32> = o.iter().map(|p| p.0).collect();
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) > 20.5);
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) < -0.5);
    }

    #[test]
    fn erasers() {
        let s = line(0.0, 100.0);
        assert!(s.hit(50.0, 12.0, 2.0));
        assert!(!s.hit(50.0, 30.0, 2.0));
        let pieces = s.split_by_eraser(50.0, 10.0, 5.0).unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].points.last().unwrap().x <= 45.5);
        assert!(pieces[1].points.first().unwrap().x >= 54.5);
        assert!(s.split_by_eraser(50.0, 40.0, 5.0).is_none());
        // Erasing the start leaves one piece.
        assert_eq!(s.split_by_eraser(0.0, 10.0, 5.0).unwrap().len(), 1);
    }

    #[test]
    fn stabilizer_ignores_wobble() {
        let mut sm = Smoother::new(2.0);
        assert!(sm.push(Point { x: 0.0, y: 0.0, p: 0.5 }).is_some());
        assert!(sm.push(Point { x: 1.5, y: 0.0, p: 0.5 }).is_none());
        let p = sm.push(Point { x: 10.0, y: 0.0, p: 0.7 }).unwrap();
        assert_eq!(p.x, 8.0);
        // Pressure eases toward the pen's instead of jumping.
        assert!(p.p > 0.5 && p.p < 0.7, "pressure {}", p.p);
    }

    #[test]
    fn reduce_keeps_corners_only() {
        let mut pts: Vec<(f32, f32)> = (0..=10).map(|i| (i as f32, 0.01 * (i % 2) as f32)).collect();
        pts.extend((1..=10).map(|i| (10.0, i as f32)));
        assert_eq!(reduce(pts, 0.08), vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    }

    #[test]
    fn spline_rounds_sparse_points() {
        // Eight points on a circle of radius 20: the curve between them
        // stays close to the circle instead of cutting straight across.
        let ring: Vec<Point> = (0..=8)
            .map(|i| {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                Point { x: 20.0 * a.cos(), y: 20.0 * a.sin(), p: 0.5 }
            })
            .collect();
        let curve = spline(&ring);
        assert!(curve.len() > 60);
        assert_eq!(curve.last(), ring.last());
        let mid = curve.iter().map(|p| (p.x * p.x + p.y * p.y).sqrt()).skip(8).take(48);
        assert!(mid.clone().all(|r| (r - 20.0).abs() < 0.5), "{:?}", mid.collect::<Vec<_>>());
    }

    #[test]
    fn curve_fit_flattens_zigzag() {
        let zig: Vec<Point> =
            (0..20).map(|i| Point { x: i as f32, y: if i % 2 == 0 { 0.0 } else { 1.0 }, p: 0.5 }).collect();
        let smooth = smooth_points(&zig, 3);
        assert_eq!(smooth[0], zig[0]);
        assert_eq!(smooth[19], zig[19]);
        assert!(smooth[5..15].iter().all(|p| (p.y - 0.5).abs() < 0.2));
    }
}
