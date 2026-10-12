//! LaTeX math via RaTeX, drawn as GSK vector paths (crisp at any zoom).
//!
//! RaTeX parses and lays out the formula into a display list of glyphs,
//! rules and paths in em units; we turn that into one fill path (plus a
//! few stroked paths) with the baseline at y = 0.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ab_glyph::{Font, FontRef, OutlineCurve};
use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};
use ratex_font::FontId;
use ratex_layout::LayoutOptions;
use ratex_types::{DisplayItem, MathStyle, PathCommand};

/// A laid-out formula. Sizes are in em; draw scales them to the font size.
pub struct Formula {
    pub width: f32,
    /// Above the baseline.
    pub ascent: f32,
    /// Below the baseline.
    pub descent: f32,
    fill: Option<gsk::Path>,
    strokes: Option<gsk::Path>,
}

impl Formula {
    /// Draw with the baseline's left end at (x, y), `em` page units per em.
    pub fn draw(&self, snapshot: &gtk::Snapshot, x: f32, y: f32, em: f32, color: &gdk::RGBA) {
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        snapshot.scale(em, em);
        if let Some(fill) = &self.fill {
            snapshot.append_fill(fill, gsk::FillRule::Winding, color);
        }
        if let Some(strokes) = &self.strokes {
            snapshot.append_stroke(strokes, &gsk::Stroke::new(0.04), color);
        }
        snapshot.restore();
    }
}

type Cache = HashMap<(String, bool), Result<Rc<Formula>, String>>;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// Lay out `latex` (inline or display style), cached.
pub fn formula(latex: &str, display: bool) -> Result<Rc<Formula>, String> {
    let key = (latex.to_string(), display);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let result = build(latex, display).map(Rc::new);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        // Typing creates many intermediate formulas; keep memory bounded.
        if c.len() > 500 {
            c.clear();
        }
        c.insert(key, result.clone());
    });
    result
}

fn build(latex: &str, display: bool) -> Result<Formula, String> {
    let nodes = ratex_parser::parse(latex).map_err(|e| e.to_string())?;
    let options = LayoutOptions {
        style: if display { MathStyle::Display } else { MathStyle::Text },
        ..Default::default()
    };
    let list = ratex_layout::to_display_list(&ratex_layout::layout(&nodes, &options));
    let fonts = ratex_font_loader::load_fonts_for_items("", &list.items)?;
    let mut refs: HashMap<FontId, FontRef<'_>> = HashMap::new();
    for (id, bytes) in fonts.iter() {
        if let Ok(f) = FontRef::try_from_slice_and_index(bytes, 0) {
            refs.insert(*id, f);
        }
    }

    // Display list y grows downward from the top; shift so the baseline is 0.
    let top = list.height as f32;
    let fill = gsk::PathBuilder::new();
    let strokes = gsk::PathBuilder::new();
    let (mut any_fill, mut any_stroke) = (false, false);
    for item in &list.items {
        match item {
            DisplayItem::GlyphPath { x, y, scale, font, char_code, .. } => {
                let id = FontId::parse(font).unwrap_or(FontId::MainRegular);
                let Some(f) = refs.get(&id).or_else(|| refs.get(&FontId::MainRegular)) else { continue };
                let ch = ratex_font::katex_ttf_glyph_char(id, *char_code);
                let glyph = f.glyph_id(ch);
                let Some(curves) = ratex_font_loader::outline_cache::get_or_compute_outline(id, f, glyph) else {
                    continue;
                };
                let s = *scale as f32 / f.units_per_em().unwrap_or(1000.0);
                append_outline(&fill, &curves, *x as f32, *y as f32 - top, s);
                any_fill = true;
            }
            DisplayItem::Line { x, y, width, thickness, .. } => {
                let t = (*thickness as f32).max(0.02);
                fill.add_rect(&graphene::Rect::new(*x as f32, *y as f32 - top - t / 2.0, *width as f32, t));
                any_fill = true;
            }
            DisplayItem::Rect { x, y, width, height, .. } => {
                fill.add_rect(&graphene::Rect::new(*x as f32, *y as f32 - top, *width as f32, *height as f32));
                any_fill = true;
            }
            DisplayItem::Path { x, y, commands, fill: filled, .. } => {
                let target = if *filled { &fill } else { &strokes };
                append_commands(target, commands, *x as f32, *y as f32 - top);
                if *filled {
                    any_fill = true;
                } else {
                    any_stroke = true;
                }
            }
        }
    }
    Ok(Formula {
        width: list.width as f32,
        ascent: list.height as f32,
        descent: list.depth as f32,
        fill: any_fill.then(|| fill.to_path()),
        strokes: any_stroke.then(|| strokes.to_path()),
    })
}

/// Font outlines are y-up in font units; flip and scale into em.
fn append_outline(b: &gsk::PathBuilder, curves: &[OutlineCurve], x: f32, y: f32, s: f32) {
    let pt = |p: ab_glyph::Point| (x + p.x * s, y - p.y * s);
    let mut last: Option<(f32, f32)> = None;
    for curve in curves {
        let (start, end) = match curve {
            OutlineCurve::Line(a, b) => (pt(*a), pt(*b)),
            OutlineCurve::Quad(a, _, c) => (pt(*a), pt(*c)),
            OutlineCurve::Cubic(a, _, _, d) => (pt(*a), pt(*d)),
        };
        let jump = last.is_none_or(|l| (l.0 - start.0).abs() > 1e-4 || (l.1 - start.1).abs() > 1e-4);
        if jump {
            if last.is_some() {
                b.close();
            }
            b.move_to(start.0, start.1);
        }
        match curve {
            OutlineCurve::Line(_, p) => {
                let p = pt(*p);
                b.line_to(p.0, p.1);
            }
            OutlineCurve::Quad(_, c, p) => {
                let (c, p) = (pt(*c), pt(*p));
                b.quad_to(c.0, c.1, p.0, p.1);
            }
            OutlineCurve::Cubic(_, c1, c2, p) => {
                let (c1, c2, p) = (pt(*c1), pt(*c2), pt(*p));
                b.cubic_to(c1.0, c1.1, c2.0, c2.1, p.0, p.1);
            }
        }
        last = Some(end);
    }
    if last.is_some() {
        b.close();
    }
}

fn append_commands(b: &gsk::PathBuilder, commands: &[PathCommand], x: f32, y: f32) {
    let p = |px: f64, py: f64| (x + px as f32, y + py as f32);
    for c in commands {
        match *c {
            PathCommand::MoveTo { x: px, y: py } => {
                let (a, bb) = p(px, py);
                b.move_to(a, bb);
            }
            PathCommand::LineTo { x: px, y: py } => {
                let (a, bb) = p(px, py);
                b.line_to(a, bb);
            }
            PathCommand::QuadTo { x1, y1, x: px, y: py } => {
                let (c, e) = (p(x1, y1), p(px, py));
                b.quad_to(c.0, c.1, e.0, e.1);
            }
            PathCommand::CubicTo { x1, y1, x2, y2, x: px, y: py } => {
                let (c1, c2, e) = (p(x1, y1), p(x2, y2), p(px, py));
                b.cubic_to(c1.0, c1.1, c2.0, c2.1, e.0, e.1);
            }
            PathCommand::Close => b.close(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_common_math() {
        for src in [r"x^2", r"\frac{a}{b}", r"\int_0^1 x^2\,dx", r"\sqrt{2}", r"\sum_{n=1}^\infty \frac{1}{n^2}"] {
            let f = formula(src, false).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(f.width > 0.0 && f.ascent > 0.0, "{src}");
            assert!(f.fill.is_some(), "{src} has no glyphs");
        }
        let inline = formula(r"\sum_{i=1}^n i", false).unwrap();
        let display = formula(r"\sum_{i=1}^n i", true).unwrap();
        assert!(display.ascent + display.descent > inline.ascent + inline.descent);
        assert!(formula(r"\frac{", false).is_err());
    }
}
