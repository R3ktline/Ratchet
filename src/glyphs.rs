//! Small hand-drawn tool icons (the icon theme has no pen/eraser/lasso/hand),
//! shared by the toolbar and the radial menu editor.

use adw::prelude::*;

use crate::ink::{InkColor, Tool};
use crate::settings::{PenSlot, RadialItem};

pub fn draw_color(color: InkColor) -> (f64, f64, f64) {
    let (r, g, b) = color.resolve(adw::StyleManager::default().is_dark());
    (r as f64, g as f64, b as f64)
}

/// Small preview of a pen slot: a dot sized by width, or a highlighter bar.
pub fn slot_glyph(slot: &PenSlot) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(22).content_height(22).build();
    let slot = slot.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let (r, g, b) = draw_color(slot.color);
        match slot.tool {
            Tool::Pen => {
                let (lo, hi) = PenSlot::range(Tool::Pen);
                let t = ((slot.width_mm as f64 - lo) / (hi - lo)).clamp(0.0, 1.0);
                cr.set_source_rgb(r, g, b);
                cr.arc(cx, cy, 2.5 + t.sqrt() * 6.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
            Tool::Highlighter => {
                cr.set_source_rgba(r, g, b, 0.75);
                let (bw, bh) = (16.0, 9.0);
                cr.rectangle(cx - bw / 2.0, cy - bh / 2.0, bw, bh);
                let _ = cr.fill();
            }
        }
    });
    area
}

pub fn eraser_glyph() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(22).content_height(22).build();
    area.set_draw_func(|area, cr, w, h| {
        let fg = area.color();
        cr.translate(w as f64 / 2.0, h as f64 / 2.0);
        cr.rotate(-std::f64::consts::FRAC_PI_4);
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.rectangle(-8.0, -4.5, 16.0, 9.0);
        let _ = cr.stroke();
        cr.rectangle(-8.0, -4.5, 6.0, 9.0);
        let _ = cr.fill();
    });
    area
}

/// Dashed lasso loop.
pub fn select_glyph() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(22).content_height(22).build();
    area.set_draw_func(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.5);
        cr.set_dash(&[3.0, 2.0], 0.0);
        cr.save().ok();
        cr.translate(w as f64 / 2.0, h as f64 / 2.0 - 1.0);
        cr.scale(1.0, 0.7);
        cr.arc(0.0, 0.0, 8.0, 0.0, std::f64::consts::TAU);
        cr.restore().ok();
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        cr.move_to(w as f64 / 2.0 - 3.0, h as f64 / 2.0 + 4.0);
        cr.line_to(w as f64 / 2.0 - 5.0, h as f64 / 2.0 + 9.0);
        let _ = cr.stroke();
    });
    area
}

/// Four-way arrows ("move the page").
pub fn hand_glyph() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(22).content_height(22).build();
    area.set_draw_func(|area, cr, w, h| {
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
        cr.set_line_width(1.6);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let (x, y) = (w as f64 / 2.0, h as f64 / 2.0);
        for (dx, dy) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
            let (tx, ty) = (x + dx * 8.0, y + dy * 8.0);
            cr.move_to(x, y);
            cr.line_to(tx, ty);
            cr.move_to(tx - dy * 3.0 - dx * 3.0, ty + dx * 3.0 - dy * 3.0);
            cr.line_to(tx, ty);
            cr.line_to(tx + dy * 3.0 - dx * 3.0, ty - dx * 3.0 - dy * 3.0);
        }
        let _ = cr.stroke();
    });
    area
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
        RadialItem::None => gtk::Box::builder().width_request(22).height_request(22).build().upcast(),
        other => gtk::Image::from_icon_name(other.icon_name().unwrap_or("image-missing-symbolic")).upcast(),
    }
}
