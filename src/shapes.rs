//! Shape recognition for "hold at the end of a stroke": straight lines
//! (snapped to 15° steps when close), arrows and circles. Deliberately
//! forgiving: the user asked for a shape by holding still.
//!
//! Works in page units; `tolerance` callers pass is screen-scaled.

use std::f32::consts::{PI, TAU};

use crate::ink::Point;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Line,
    Arrow,
    Circle,
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub kind: ShapeKind,
    pub points: Vec<Point>,
}

/// Lines snap to multiples of this angle…
const SNAP_STEP: f32 = 15.0 * PI / 180.0;
/// …when within this much of one.
const SNAP_WITHIN: f32 = 4.0 * PI / 180.0;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn xy(p: &Point) -> (f32, f32) {
    (p.x, p.y)
}

fn path_length(points: &[Point]) -> f32 {
    points.windows(2).map(|w| dist(xy(&w[0]), xy(&w[1]))).sum()
}

fn mean_pressure(points: &[Point]) -> f32 {
    points.iter().map(|p| p.p).sum::<f32>() / points.len().max(1) as f32
}

/// Largest distance of `points` from the segment a–b.
fn max_deviation(points: &[Point], a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    points.iter().map(|p| ((p.x - a.0) * dy - (p.y - a.1) * dx).abs() / len).fold(0.0, f32::max)
}

/// Snap the end of a line to the nearest 15° direction if it's close.
pub fn snap_angle(start: (f32, f32), end: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        return end;
    }
    let angle = dy.atan2(dx);
    let snapped = (angle / SNAP_STEP).round() * SNAP_STEP;
    if (angle - snapped).abs() <= SNAP_WITHIN {
        (start.0 + len * snapped.cos(), start.1 + len * snapped.sin())
    } else {
        end
    }
}

pub fn line_points(a: (f32, f32), b: (f32, f32), p: f32) -> Vec<Point> {
    // A few points along the line so erasers and selection work as usual.
    let n = (dist(a, b) / 8.0).ceil().clamp(1.0, 200.0) as usize;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            Point { x: a.0 + (b.0 - a.0) * t, y: a.1 + (b.1 - a.1) * t, p }
        })
        .collect()
}

/// An arrow as one stroke: shaft, then out and back along each head side.
pub fn arrow_points(a: (f32, f32), tip: (f32, f32), p: f32) -> Vec<Point> {
    let len = dist(a, tip);
    let head = (len * 0.18).clamp(8.0, 40.0);
    let back = (a.1 - tip.1).atan2(a.0 - tip.0);
    let side = |s: f32| {
        let ang = back + s * PI / 6.0;
        (tip.0 + head * ang.cos(), tip.1 + head * ang.sin())
    };
    let mut pts = line_points(a, tip, p);
    pts.extend(line_points(tip, side(1.0), p).into_iter().skip(1));
    pts.extend(line_points(side(1.0), tip, p).into_iter().skip(1));
    pts.extend(line_points(tip, side(-1.0), p).into_iter().skip(1));
    pts
}

fn try_line(points: &[Point], tolerance: f32) -> Option<Shape> {
    let (a, b) = (xy(points.first()?), xy(points.last()?));
    let len = dist(a, b);
    if len < tolerance * 4.0 || max_deviation(points, a, b) > (len * 0.12).max(tolerance * 3.0) {
        return None;
    }
    let b = snap_angle(a, b);
    Some(Shape { kind: ShapeKind::Line, points: line_points(a, b, mean_pressure(points)) })
}

/// A straight shaft to the point farthest from the start, then a short
/// tail heading back (the arrowhead stroke).
fn try_arrow(points: &[Point], tolerance: f32) -> Option<Shape> {
    let a = xy(points.first()?);
    let (tip_i, _) = points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, dist(a, xy(p))))
        .max_by(|x, y| x.1.total_cmp(&y.1))?;
    let tip = xy(&points[tip_i]);
    let shaft = dist(a, tip);
    let tail = &points[tip_i..];
    let tail_len = path_length(tail);
    if shaft < tolerance * 6.0 || tail.len() < 3 || tail_len < shaft * 0.08 || tail_len > shaft * 0.6 {
        return None;
    }
    if max_deviation(&points[..=tip_i], a, tip) > (shaft * 0.12).max(tolerance * 3.0) {
        return None;
    }
    // The tail must head back toward the start, not continue onward.
    let end = xy(tail.last()?);
    let (sx, sy) = ((a.0 - tip.0) / shaft, (a.1 - tip.1) / shaft);
    let (tx, ty) = (end.0 - tip.0, end.1 - tip.1);
    let tl = (tx * tx + ty * ty).sqrt().max(1e-3);
    if (sx * tx + sy * ty) / tl < 0.3 {
        return None;
    }
    let tip = snap_angle(a, tip);
    Some(Shape { kind: ShapeKind::Arrow, points: arrow_points(a, tip, mean_pressure(points)) })
}

/// A roughly closed loop becomes a circle: centered on the loop, with its
/// average radius. Ovals become circles too.
fn try_circle(points: &[Point], tolerance: f32) -> Option<Shape> {
    if points.len() < 6 {
        return None;
    }
    let perimeter = path_length(points);
    let (first, last) = (xy(&points[0]), xy(points.last()?));
    if perimeter < tolerance * 10.0 || dist(first, last) > perimeter * 0.3 {
        return None;
    }
    // Resample evenly by arc length so slow parts don't dominate.
    let samples = resample(points, 96);
    let m = samples.len() as f32;
    let (cx, cy) = samples.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 / m, a.1 + p.1 / m));
    let radius = samples.iter().map(|p| dist(*p, (cx, cy))).sum::<f32>() / m;
    if radius < tolerance * 2.0 {
        return None;
    }
    // Close to round (loose), and it goes (nearly) all the way around.
    // Hand-drawn circles usually overshoot the start; only the first full
    // turn counts for the roundness check below.
    let mut swept = 0.0f32;
    let mut one_turn = samples.len();
    let mut prev: Option<f32> = None;
    for (k, p) in samples.iter().enumerate() {
        if (dist(*p, (cx, cy)) / radius - 1.0).abs() > 0.4 {
            return None;
        }
        let ang = (p.1 - cy).atan2(p.0 - cx);
        if let Some(pa) = prev {
            let mut d = ang - pa;
            if d > PI {
                d -= TAU;
            } else if d < -PI {
                d += TAU;
            }
            swept += d;
            if swept.abs() >= TAU && one_turn == samples.len() {
                one_turn = k + 1;
            }
        }
        prev = Some(ang);
    }
    if swept.abs() < TAU * 0.75 {
        return None;
    }
    // Round, not boxy: circularity 4πA/P² is 1 for a circle, ~0.79 for a
    // square; wobbly hand circles land around 0.9, ovals ~0.98.
    if circularity(&samples[..one_turn]) < 0.84 {
        return None;
    }
    Some(Shape { kind: ShapeKind::Circle, points: circle_points((cx, cy), radius, mean_pressure(points)) })
}

/// 4πA/P² of the loop (closed by joining its ends).
fn circularity(loop_points: &[(f32, f32)]) -> f32 {
    let n = loop_points.len();
    let (mut area, mut perimeter) = (0.0f32, 0.0f32);
    for i in 0..n {
        let (a, b) = (loop_points[i], loop_points[(i + 1) % n]);
        area += a.0 * b.1 - b.0 * a.1;
        perimeter += dist(a, b);
    }
    let area = area.abs() / 2.0;
    if perimeter <= 0.0 { 0.0 } else { 4.0 * PI * area / (perimeter * perimeter) }
}

/// A closed circle as stroke points.
pub fn circle_points(center: (f32, f32), radius: f32, p: f32) -> Vec<Point> {
    let steps = 96;
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32 * TAU;
            Point { x: center.0 + radius * t.cos(), y: center.1 + radius * t.sin(), p }
        })
        .collect()
}

/// Center of a recognized circle's points.
pub fn circle_center(points: &[Point]) -> (f32, f32) {
    // The last point repeats the first; leave it out.
    let pts = &points[..points.len().saturating_sub(1).max(1)];
    let n = pts.len() as f32;
    pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x / n, a.1 + p.y / n))
}

/// `count` points evenly spaced along the stroke.
fn resample(points: &[Point], count: usize) -> Vec<(f32, f32)> {
    let total = path_length(points);
    if total <= 0.0 {
        return points.iter().map(xy).collect();
    }
    let step = total / (count - 1) as f32;
    let mut out = vec![xy(&points[0])];
    let mut acc = 0.0;
    for w in points.windows(2) {
        let (a, b) = (xy(&w[0]), xy(&w[1]));
        let seg = dist(a, b);
        if seg <= 0.0 {
            continue;
        }
        let mut t0 = 0.0;
        while acc + seg * (1.0 - t0) >= step {
            let need = step - acc;
            t0 += need / seg;
            out.push((a.0 + (b.0 - a.0) * t0, a.1 + (b.1 - a.1) * t0));
            acc = 0.0;
        }
        acc += seg * (1.0 - t0);
    }
    out
}

/// Recognize a stroke. `tolerance` is in page units (a few screen px).
pub fn recognize(points: &[Point], tolerance: f32) -> Option<Shape> {
    if points.len() < 2 {
        return None;
    }
    // Arrows first: their head would otherwise pass as a wobbly line.
    try_arrow(points, tolerance).or_else(|| try_line(points, tolerance)).or_else(|| try_circle(points, tolerance))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(f: impl Fn(f32) -> (f32, f32), n: usize) -> Vec<Point> {
        (0..=n).map(|i| {
            let (x, y) = f(i as f32 / n as f32);
            Point { x, y, p: 0.5 }
        })
        .collect()
    }

    #[test]
    fn wobbly_line_snaps_to_angle() {
        // ~29° with a little wobble -> 30°.
        let a = 29.0f32.to_radians();
        let line = pts(|t| (t * 200.0 * a.cos() + (t * 40.0).sin(), t * 200.0 * a.sin()), 60);
        let s = recognize(&line, 3.0).unwrap();
        assert_eq!(s.kind, ShapeKind::Line);
        let end = s.points.last().unwrap();
        let ang = end.y.atan2(end.x).to_degrees();
        assert!((ang - 30.0).abs() < 0.01, "{ang}");
        // 37° is far from any 15° step: kept.
        assert_eq!(snap_angle((0.0, 0.0), (100.0 * 37f32.to_radians().cos(), 100.0 * 37f32.to_radians().sin())).0,
                   100.0 * 37f32.to_radians().cos());
    }

    #[test]
    fn loops_become_circles() {
        let circle = pts(|t| (100.0 + 50.0 * (t * TAU).cos(), 100.0 + 52.0 * (t * TAU).sin()), 80);
        assert_eq!(recognize(&circle, 3.0).unwrap().kind, ShapeKind::Circle);
        // A wobbly, slightly oval loop that doesn't quite close.
        let shaky = pts(
            |t| {
                let a = t * TAU * 0.9;
                let r = 50.0 + 4.0 * (t * 23.0).sin();
                (100.0 + 1.3 * r * a.cos(), 100.0 + r * a.sin())
            },
            80,
        );
        let s = recognize(&shaky, 3.0).unwrap();
        assert_eq!(s.kind, ShapeKind::Circle);
        // Output is a true circle.
        let (cx, cy) = s.points.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.y));
        let n = s.points.len() as f32;
        let r: Vec<f32> = s.points.iter().map(|p| dist((p.x, p.y), (cx / n, cy / n))).collect();
        assert!(r.iter().all(|v| (v - r[0]).abs() < 1.5));
    }

    #[test]
    fn overshooting_wobbly_circle() {
        // 1.25 turns, slightly egg-shaped and wobbly, like a quick hand circle.
        let circle = pts(
            |t| {
                let a = t * TAU * 1.25;
                let r = 60.0 + 6.0 * (t * 17.0).sin() + 5.0 * a.cos();
                (200.0 + r * a.cos(), 200.0 + 0.9 * r * a.sin())
            },
            120,
        );
        assert_eq!(recognize(&circle, 3.0).unwrap().kind, ShapeKind::Circle);
    }

    #[test]
    fn squares_are_not_circles() {
        // A hand-drawn square loop.
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 2.0)];
        let mut square = Vec::new();
        for w in corners.windows(2) {
            square.extend(pts(|t| (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t), 20));
        }
        assert!(recognize(&square, 3.0).is_none());
    }

    #[test]
    fn shaky_line_still_a_line() {
        let line = pts(|t| (t * 200.0, 6.0 * (t * 31.0).sin()), 60);
        assert_eq!(recognize(&line, 3.0).unwrap().kind, ShapeKind::Line);
    }

    #[test]
    fn arrow_with_head() {
        let mut stroke = pts(|t| (t * 200.0, 0.0), 40);
        stroke.extend(pts(|t| (200.0 - t * 25.0, -t * 15.0), 8));
        assert_eq!(recognize(&stroke, 3.0).unwrap().kind, ShapeKind::Arrow);
    }

    #[test]
    fn handwriting_is_left_alone() {
        // A squiggle (like cursive) is neither a line nor a loop.
        let squiggle = pts(|t| (t * 150.0, 30.0 * (t * 18.0).sin()), 120);
        assert!(recognize(&squiggle, 3.0).is_none());
        // An open "C" isn't a circle.
        let c = pts(|t| (50.0 * (t * PI * 1.3).cos(), 50.0 * (t * PI * 1.3).sin()), 60);
        assert!(recognize(&c, 3.0).is_none());
    }
}
