//! Visual editor for the radial menu: the ring as it will look, and a
//! shelf of everything that can go in it. The ring holds 2–12 items and
//! re-spaces them evenly whenever one is added or removed.
//!
//! - Drag a shelf item onto a spot to replace it, or onto the ring to add it.
//! - Drag a spot onto another to swap them, or onto the shelf to remove it.
//! - With a pen: tap a spot to pick it, then tap an item to put it there;
//!   tapping an item with nothing picked adds it. "Remove" drops the picked spot.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::glyphs::radial_glyph;
use crate::settings::{PenSlot, RADIAL_MAX, RADIAL_MIN, RadialItem, default_radial};

const RING: i32 = 320;
const SPOT_RADIUS: f64 = 116.0;
const SPOT_MAX: f64 = 56.0;

/// Shelf entries: every pen slot, then the actions.
fn shelf_items(pens: &[PenSlot]) -> Vec<(RadialItem, String)> {
    let mut out = Vec::new();
    let (mut n_pen, mut n_hl) = (0, 0);
    for (i, p) in pens.iter().enumerate() {
        let label = match p.tool {
            crate::ink::Tool::Pen => {
                n_pen += 1;
                format!("Pen {n_pen}")
            }
            crate::ink::Tool::Highlighter => {
                n_hl += 1;
                format!("Highlighter {n_hl}")
            }
        };
        out.push((RadialItem::Slot(i), label));
    }
    for a in RadialItem::ACTIONS.into_iter().filter(|a| *a != RadialItem::None) {
        out.push((a, a.label()));
    }
    out
}

fn encode(kind: &str, i: usize) -> glib::Value {
    format!("{kind}:{i}").to_value()
}

fn decode(v: &glib::Value) -> Option<(String, usize)> {
    let s = v.get::<String>().ok()?;
    let (kind, i) = s.split_once(':')?;
    Some((kind.to_string(), i.parse().ok()?))
}

fn drag_source(widget: &impl IsA<gtk::Widget>, value: glib::Value) {
    let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    let w = widget.as_ref().clone();
    source.connect_prepare(move |src, _, _| {
        src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&w))), 20, 20);
        Some(gdk::ContentProvider::for_value(&value))
    });
    widget.add_controller(source);
}

type ChangedFn = Box<dyn Fn(&[RadialItem])>;

struct Editor {
    items: RefCell<Vec<RadialItem>>,
    pens: Vec<PenSlot>,
    shelf: Vec<(RadialItem, String)>,
    ring: gtk::Fixed,
    spots: RefCell<Vec<gtk::Button>>,
    remove: gtk::Button,
    count: gtk::Label,
    /// Spot picked by tapping, waiting for an item.
    picked: Cell<Option<usize>>,
    /// Spot to "pop" in on the next layout (just added or replaced).
    popping: Cell<Option<usize>>,
    changed: ChangedFn,
}

impl Editor {
    fn label(&self, item: RadialItem) -> String {
        self.shelf.iter().find(|(it, _)| *it == item).map_or_else(|| item.label(), |(_, l)| l.clone())
    }

    /// Rebuild the spots around the ring for the current items.
    fn layout(self: &Rc<Self>) {
        for spot in self.spots.take() {
            self.ring.remove(&spot);
        }
        let items = self.items.borrow().clone();
        let n = items.len().max(1);
        // Smaller spots when many share the ring.
        let size = SPOT_MAX.min(std::f64::consts::TAU * SPOT_RADIUS / n as f64 - 10.0);
        let mut spots = Vec::with_capacity(n);
        for (i, item) in items.iter().enumerate() {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let (x, y) = (
                RING as f64 / 2.0 + SPOT_RADIUS * a.sin() - size / 2.0,
                RING as f64 / 2.0 - SPOT_RADIUS * a.cos() - size / 2.0,
            );
            let spot = gtk::Button::builder()
                .width_request(size as i32)
                .height_request(size as i32)
                .child(&radial_glyph(*item, &self.pens))
                .tooltip_text(self.label(*item))
                .css_classes(["circular", "rt-spot"])
                .build();
            if self.picked.get() == Some(i) {
                spot.add_css_class("suggested-action");
            }
            if self.popping.get() == Some(i) {
                spot.add_css_class("rt-pop");
            }
            self.attach_spot(&spot, i);
            self.ring.put(&spot, x, y);
            spots.push(spot);
        }
        *self.spots.borrow_mut() = spots;
        self.popping.set(None);
        let len = items.len();
        self.count.set_text(&format!("{len} of {RADIAL_MAX} items"));
        self.remove.set_sensitive(self.picked.get().is_some() && len > RADIAL_MIN);
    }

    fn attach_spot(self: &Rc<Self>, spot: &gtk::Button, i: usize) {
        // Dragging a spot off the ring (dropped anywhere that isn't a
        // spot) removes it.
        let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        let w = spot.clone();
        source.connect_prepare(move |src, _, _| {
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&w))), 20, 20);
            Some(gdk::ContentProvider::for_value(&encode("spot", i)))
        });
        let ed = Rc::downgrade(self);
        source.connect_drag_cancel(move |_, _, reason| {
            let Some(ed) = ed.upgrade() else { return false };
            if reason == gdk::DragCancelReason::NoTarget {
                ed.remove_at(i);
                // Handled: no snap-back animation.
                return true;
            }
            false
        });
        spot.add_controller(source);
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        let ed = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(ed), Some((kind, k))) = (ed.upgrade(), decode(value)) else { return false };
            match kind.as_str() {
                "item" => ed.replace(i, ed.shelf[k].0),
                "spot" if k != i => ed.set(|items| items.swap(i, k)),
                _ => return false,
            }
            true
        });
        spot.add_controller(drop);
        let ed = Rc::downgrade(self);
        spot.connect_clicked(move |_| {
            let Some(ed) = ed.upgrade() else { return };
            ed.picked.set(if ed.picked.get() == Some(i) { None } else { Some(i) });
            ed.layout();
        });
    }

    /// Change the items, re-lay out the ring and report.
    fn set(self: &Rc<Self>, f: impl FnOnce(&mut Vec<RadialItem>)) {
        f(&mut self.items.borrow_mut());
        self.picked.set(None);
        self.layout();
        (self.changed)(&self.items.borrow());
    }

    fn add(self: &Rc<Self>, item: RadialItem) {
        let len = self.items.borrow().len();
        if len < RADIAL_MAX {
            self.popping.set(Some(len));
            self.set(|items| items.push(item));
        }
    }

    fn replace(self: &Rc<Self>, i: usize, item: RadialItem) {
        self.popping.set(Some(i));
        self.set(|items| items[i] = item);
    }

    fn remove_at(self: &Rc<Self>, i: usize) {
        if self.items.borrow().len() > RADIAL_MIN {
            self.set(|items| {
                items.remove(i);
            });
        }
    }
}

pub fn show(
    parent: &impl IsA<gtk::Widget>,
    items: Vec<RadialItem>,
    pens: Vec<PenSlot>,
    changed: impl Fn(&[RadialItem]) + 'static,
) {
    // The ring: a drawn disc; spots are placed on it by `layout`.
    let ring = gtk::Fixed::builder().width_request(RING).height_request(RING).halign(gtk::Align::Center).build();
    let disc = gtk::DrawingArea::builder().content_width(RING).content_height(RING).build();
    disc.set_draw_func(|area, cr, w, h| {
        let fg = area.color();
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.06);
        cr.arc(cx, cy, cx - 4.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
        cr.arc(cx, cy, 30.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        // The center closes the menu.
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.5);
        cr.set_line_width(1.6);
        for (a, b) in [((-6.0, -6.0), (6.0, 6.0)), ((6.0, -6.0), (-6.0, 6.0))] {
            cr.move_to(cx + a.0, cy + a.1);
            cr.line_to(cx + b.0, cy + b.1);
        }
        let _ = cr.stroke();
    });
    ring.put(&disc, 0.0, 0.0);

    let remove = gtk::Button::builder().label("Remove").css_classes(["text-button", "destructive-action"]).sensitive(false).build();
    let count = gtk::Label::builder().css_classes(["dim-label", "caption"]).build();

    let editor = Rc::new(Editor {
        items: RefCell::new(items),
        shelf: shelf_items(&pens),
        pens,
        ring: ring.clone(),
        spots: RefCell::default(),
        remove: remove.clone(),
        count: count.clone(),
        picked: Cell::new(None),
        popping: Cell::new(None),
        changed: Box::new(changed),
    });

    // Dropping a shelf item on the ring (not on a spot) adds it.
    let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
    let ed = Rc::downgrade(&editor);
    drop.connect_drop(move |_, value, _, _| {
        let (Some(ed), Some((kind, k))) = (ed.upgrade(), decode(value)) else { return false };
        if kind != "item" {
            return false;
        }
        ed.add(ed.shelf[k].0);
        true
    });
    ring.add_controller(drop);

    let ed = Rc::downgrade(&editor);
    remove.connect_clicked(move |_| {
        let Some(ed) = ed.upgrade() else { return };
        if let Some(i) = ed.picked.get() {
            ed.remove_at(i);
        }
    });

    // Shelf
    let shelf_box = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(4)
        .min_children_per_line(2)
        .homogeneous(true)
        .column_spacing(6)
        .row_spacing(6)
        .build();
    for (k, (item, label)) in editor.shelf.iter().enumerate() {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&radial_glyph(*item, &editor.pens));
        content.append(&gtk::Label::builder().label(label).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
        let chip = gtk::Button::builder().child(&content).build();
        drag_source(&chip, encode("item", k));
        let ed = Rc::downgrade(&editor);
        chip.connect_clicked(move |_| {
            let Some(ed) = ed.upgrade() else { return };
            let item = ed.shelf[k].0;
            match ed.picked.get() {
                Some(t) => ed.replace(t, item),
                None => ed.add(item),
            }
        });
        shelf_box.append(&chip);
    }
    // Dropping a spot back on the shelf removes it.
    let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
    let ed = Rc::downgrade(&editor);
    drop.connect_drop(move |_, value, _, _| {
        let (Some(ed), Some((kind, k))) = (ed.upgrade(), decode(value)) else { return false };
        if kind != "spot" {
            return false;
        }
        ed.remove_at(k);
        true
    });
    shelf_box.add_controller(drop);

    let hint = gtk::Label::builder()
        .label("Drag items onto the ring to add them, onto a spot to replace it. Drag a spot onto another to swap, or drag it off the ring to remove it. Or tap a spot, then an item.")
        .wrap(true)
        .max_width_chars(52)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    let reset = gtk::Button::builder().label("Reset to Defaults").build();
    let ed = Rc::downgrade(&editor);
    reset.connect_clicked(move |_| {
        if let Some(ed) = ed.upgrade() {
            let defaults = default_radial(&ed.pens);
            ed.set(|items| *items = defaults);
        }
    });
    let actions = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).build();
    actions.append(&remove);
    actions.append(&reset);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&hint);
    content.append(&ring);
    content.append(&count);
    content.append(&actions);
    content.append(
        &gtk::ScrolledWindow::builder()
            .child(&shelf_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(crate::platform::px(240.0))
            .build(),
    );

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title("Radial Menu")
        .content_width(crate::platform::px(540.0))
        .child(&view)
        .build();
    editor.layout();
    // The dialog's widgets only hold weak references; keep the editor
    // alive until it closes.
    let keep = RefCell::new(Some(editor));
    dialog.connect_closed(move |_| {
        keep.take();
    });
    dialog.present(Some(parent));
}
