//! Small hand-drawn tool icons (the icon theme has no pen/eraser/lasso/hand),
//! shared by the toolbar and the radial menu editor.

use adw::prelude::*;

use crate::ink::{InkColor, Tool};
use crate::settings::{PenSlot, RadialItem};

/// Glyphs are drawn on a 22×22 grid and scaled to the widget.
const GRID: f64 = 22.0;

/// Side of a glyph, matching the header's icon size.
pub fn glyph_size() -> i32 {
    (20.0 * crate::platform::ui_scale()).round() as i32
}

fn glyph(draw: impl Fn(&gtk::DrawingArea, &gtk::cairo::Context, f64, f64) + 'static) -> gtk::DrawingArea {
    let size = glyph_size();
    let area = gtk::DrawingArea::builder().content_width(size).content_height(size).build();
    area.set_draw_func(move |area, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let s = w.min(h) / GRID;
        cr.translate((w - GRID * s) / 2.0, (h - GRID * s) / 2.0);
        cr.scale(s, s);
        draw(area, cr, GRID, GRID);
    });
    area
}

pub fn draw_color(color: InkColor) -> (f64, f64, f64) {
    let (r, g, b) = color.resolve(adw::StyleManager::default().is_dark());
    (r as f64, g as f64, b as f64)
}

/// Preview of a pen slot, OneNote-style: a pen (or a chisel-tip
/// highlighter) in its color, standing upright, with a thin outline so ink
/// matching the background stays visible. Thicker pens get a wider tip.
pub fn slot_glyph(slot: &PenSlot) -> gtk::DrawingArea {
    let slot = slot.clone();
    glyph(move |area, cr, w, _h| {
        let cx = w / 2.0;
        let (r, g, b) = draw_color(slot.color);
        let fg = area.color();
        let outline = |cr: &gtk::cairo::Context| {
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.45);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        };
        match slot.tool {
            Tool::Pen => {
                let (lo, hi) = PenSlot::range(Tool::Pen);
                let t = ((slot.width_mm as f64 - lo) / (hi - lo)).clamp(0.0, 1.0);
                let half = 3.0;
                let tip = 0.6 + t * 1.4;
                // Barrel, then the cone down to the tip.
                cr.move_to(cx - half, 1.5);
                cr.line_to(cx + half, 1.5);
                cr.line_to(cx + half, 13.0);
                cr.line_to(cx + tip, 19.5);
                cr.line_to(cx - tip, 19.5);
                cr.line_to(cx - half, 13.0);
                cr.close_path();
                cr.set_source_rgb(r, g, b);
                let _ = cr.fill_preserve();
                outline(cr);
                // A band where the barrel meets the cone.
                cr.move_to(cx - half, 13.0);
                cr.line_to(cx + half, 13.0);
                outline(cr);
            }
            Tool::Highlighter => {
                let half = 4.5;
                cr.move_to(cx - half, 1.5);
                cr.line_to(cx + half, 1.5);
                cr.line_to(cx + half, 12.0);
                cr.line_to(cx + 3.0, 15.0);
                // Chisel tip, slanted.
                cr.line_to(cx + 3.0, 18.0);
                cr.line_to(cx - 3.0, 20.0);
                cr.line_to(cx - 3.0, 15.0);
                cr.line_to(cx - half, 12.0);
                cr.close_path();
                cr.set_source_rgba(r, g, b, 0.85);
                let _ = cr.fill_preserve();
                outline(cr);
            }
        }
    })
}

pub fn eraser_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.translate(w / 2.0, h / 2.0);
        cr.rotate(-std::f64::consts::FRAC_PI_4);
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.rectangle(-8.0, -4.5, 16.0, 9.0);
        let _ = cr.stroke();
        cr.rectangle(-8.0, -4.5, 6.0, 9.0);
        let _ = cr.fill();
    })
}

/// Dashed lasso loop.
pub fn select_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.set_dash(&[3.0, 2.0], 0.0);
        cr.save().ok();
        cr.translate(w / 2.0, h / 2.0 - 1.0);
        cr.scale(1.0, 0.7);
        cr.arc(0.0, 0.0, 8.0, 0.0, std::f64::consts::TAU);
        cr.restore().ok();
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        cr.move_to(w / 2.0 - 3.0, h / 2.0 + 4.0);
        cr.line_to(w / 2.0 - 5.0, h / 2.0 + 9.0);
        let _ = cr.stroke();
    })
}

/// Four-way arrows ("move the page").
pub fn hand_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.6);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (x, y) = (w / 2.0, h / 2.0);
        for (dx, dy) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
            let (tx, ty) = (x + dx * 8.0, y + dy * 8.0);
            cr.move_to(x, y);
            cr.line_to(tx, ty);
            cr.move_to(tx - dy * 3.0 - dx * 3.0, ty + dx * 3.0 - dy * 3.0);
            cr.line_to(tx, ty);
            cr.line_to(tx + dy * 3.0 - dx * 3.0, ty - dx * 3.0 - dy * 3.0);
        }
        let _ = cr.stroke();
    })
}

/// A circle and a straight line ("snap to shapes").
pub fn shapes_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (x, y) = (w / 2.0, h / 2.0);
        cr.arc(x - 2.5, y - 2.5, 5.0, 0.0, std::f64::consts::TAU);
        cr.new_sub_path();
        cr.move_to(x - 6.0, y + 7.0);
        cr.line_to(x + 8.0, y - 7.0);
        let _ = cr.stroke();
    })
}

/// A gentle wave ("smoothing").
pub fn wave_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.6);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (x, y) = (w / 2.0, h / 2.0);
        cr.move_to(x - 8.0, y + 1.0);
        cr.curve_to(x - 4.0, y - 7.0, x, y - 7.0, x, y);
        cr.curve_to(x, y + 7.0, x + 4.0, y + 7.0, x + 8.0, y - 1.0);
        let _ = cr.stroke();
    })
}

/// A serif capital T, the size and weight of the other line glyphs: a
/// bar with short downturned ends, a stem, and a foot.
pub fn text_glyph() -> gtk::DrawingArea {
    glyph(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.set_line_cap(gtk::cairo::LineCap::Butt);
        cr.set_line_join(gtk::cairo::LineJoin::Miter);
        let (x, y) = (w / 2.0, h / 2.0);
        let (top, bottom, half) = (y - 6.5, y + 6.5, 6.0);
        // Bar with serifs turned down at both ends.
        cr.move_to(x - half, top + 2.5);
        cr.line_to(x - half, top);
        cr.line_to(x + half, top);
        cr.line_to(x + half, top + 2.5);
        // Stem and foot.
        cr.move_to(x, top);
        cr.line_to(x, bottom);
        cr.move_to(x - 2.5, bottom);
        cr.line_to(x + 2.5, bottom);
        let _ = cr.stroke();
    })
}

/// Icon for a radial menu item.
pub fn radial_glyph(item: RadialItem, pens: &[PenSlot]) -> gtk::Widget {
    match item {
        RadialItem::Slot(i) => match pens.get(i) {
            Some(p) => slot_glyph(p).upcast(),
            None => gtk::Image::from_icon_name("action-unavailable-symbolic").upcast(),
        },
        RadialItem::Eraser => eraser_glyph().upcast(),
        RadialItem::Select => select_glyph().upcast(),
        RadialItem::Hand => hand_glyph().upcast(),
        RadialItem::Text => text_glyph().upcast(),
        RadialItem::None => gtk::Box::builder().width_request(glyph_size()).height_request(glyph_size()).build().upcast(),
        other => gtk::Image::from_icon_name(other.icon_name().unwrap_or("image-missing-symbolic")).upcast(),
    }
}
