//! Preferences window: everything on one wide screen in two columns, short
//! labels, explanations in tooltips.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::glyphs::{shapes_glyph, wave_glyph};
use crate::platform::px;
use crate::radial_editor;
use crate::settings::{ButtonAction, PenSlot, RadialItem, Smoothing};

pub struct Prefs {
    pub smoothing: Smoothing,
    pub button_2: ButtonAction,
    pub button_3: ButtonAction,
    pub eraser: ButtonAction,
    pub long_press_ms: Option<u32>,
    pub shape_hold_ms: Option<u32>,
    pub radial_items: Vec<RadialItem>,
}

/// Hold times can be set from 0.15 to 2 seconds.
const HOLD_MIN: f64 = 0.15;
const HOLD_MAX: f64 = 2.0;

/// A hold gesture: a row whose switch turns it on, opening to its duration.
fn hold_row(
    title: &str,
    subtitle: &str,
    tooltip: &str,
    icon: &impl IsA<gtk::Widget>,
    value_ms: Option<u32>,
    default_ms: u32,
) -> (adw::ExpanderRow, adw::SpinRow) {
    let row = adw::ExpanderRow::builder()
        .title(title)
        .subtitle(subtitle)
        .tooltip_text(tooltip)
        .show_enable_switch(true)
        .enable_expansion(value_ms.is_some())
        .build();
    row.add_prefix(icon);
    let secs = value_ms.unwrap_or(default_ms) as f64 / 1000.0;
    let time = adw::SpinRow::builder()
        .title("Hold For")
        .subtitle("Seconds")
        .adjustment(&gtk::Adjustment::new(secs, HOLD_MIN, HOLD_MAX, 0.05, 0.25, 0.0))
        .digits(2)
        .build();
    row.add_row(&time);
    (row, time)
}

/// A drawn glyph at the size of a row's icons, so labels line up.
fn row_glyph(glyph: gtk::DrawingArea) -> gtk::DrawingArea {
    glyph.set_content_width(px(16.0));
    glyph.set_content_height(px(16.0));
    glyph
}

fn icon_row<R: IsA<adw::ActionRow>>(row: R, icon: &str) -> R {
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row
}

/// Show the preferences; `changed` runs with the new values on every edit.
pub fn show(parent: &impl IsA<gtk::Widget>, current: Prefs, pens: &[PenSlot], changed: impl Fn(&Prefs) + 'static) {
    // Ink
    let smoothing = adw::ComboRow::builder()
        .title("Smoothing")
        .tooltip_text("Higher smooths out wobble; the line trails the pen a little more")
        .model(&gtk::StringList::new(&Smoothing::ALL.map(Smoothing::label)))
        .selected(Smoothing::ALL.iter().position(|s| *s == current.smoothing).unwrap_or(2) as u32)
        .build();
    smoothing.add_prefix(&row_glyph(wave_glyph()));
    let (shapes, shape_time) = hold_row(
        "Snap to Shapes",
        "Hold still at the end of a stroke",
        "Straight lines (snapping to 15°), arrows and circles",
        &row_glyph(shapes_glyph()),
        current.shape_hold_ms,
        500,
    );
    let ink = adw::PreferencesGroup::builder().title("Ink").build();
    ink.add(&smoothing);
    ink.add(&shapes);

    // Pen buttons
    let actions = gtk::StringList::new(&ButtonAction::ALL.map(ButtonAction::label));
    let pos = |a: ButtonAction| ButtonAction::ALL.iter().position(|x| *x == a).unwrap_or(0) as u32;
    let button_row = |title: &str, tooltip: &str, icon: &str, value: ButtonAction| {
        icon_row(
            adw::ComboRow::builder().title(title).tooltip_text(tooltip).model(&actions).selected(pos(value)).build(),
            icon,
        )
    };
    let button_2 = button_row(
        "Lower Side Button",
        "Middle click: press while hovering, or hold while touching",
        "input-mouse-symbolic",
        current.button_2,
    );
    let button_3 = button_row("Upper Side Button", "Right click: hold while touching", "input-mouse-symbolic", current.button_3);
    let eraser = button_row(
        "Eraser End",
        "The flipped pen, or a button your tablet driver maps to eraser",
        "edit-clear-symbolic",
        current.eraser,
    );
    let info = gtk::Image::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("Which button is which? Debug › Pen Input Overlay (RATCHET_DEBUG=1)")
        .css_classes(["dim-label"])
        .build();
    let buttons = adw::PreferencesGroup::builder().title("Pen Buttons").header_suffix(&info).build();
    buttons.add(&button_2);
    buttons.add(&button_3);
    buttons.add(&eraser);

    // Radial menu
    let (long_press, long_press_time) = hold_row(
        "Long Press",
        "Hold the pen still to open it",
        "Without moving, anywhere on the page",
        &gtk::Image::from_icon_name("preferences-system-time-symbolic"),
        current.long_press_ms,
        720,
    );
    let customize = adw::ButtonRow::builder().title("Customize Items…").start_icon_name("view-app-grid-symbolic").build();
    let radial = adw::PreferencesGroup::builder()
        .title("Radial Menu")
        .tooltip_text("Point at an item and release, or touch, slide toward it and lift. The center or Esc closes it.")
        .build();
    radial.add(&long_press);
    radial.add(&customize);

    // Everything on one screen, in two columns: drawing on the left, the
    // pen's buttons on the right.
    let column = |groups: &[&adw::PreferencesGroup]| {
        let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(px(24.0)).valign(gtk::Align::Start).build();
        for g in groups {
            b.append(*g);
        }
        b
    };
    let columns = gtk::Box::builder()
        .homogeneous(true)
        .spacing(px(28.0))
        // 28px matches the header's inset, so its edges line up.
        .margin_start(28)
        .margin_end(28)
        .margin_top(px(8.0))
        .margin_bottom(28)
        .build();
    columns.append(&column(&[&ink, &radial]));
    columns.append(&column(&[&buttons]));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&columns));
    let dialog = adw::Dialog::builder()
        .title("Preferences")
        .content_width(px(900.0))
        .child(&view)
        .build();
    crate::ui::keep_icon_labels_aligned(&dialog);

    let radial_items = Rc::new(RefCell::new(current.radial_items));
    let rows = (smoothing.clone(), button_2.clone(), button_3.clone(), eraser.clone());
    let holds = (shapes.clone(), shape_time.clone(), long_press.clone(), long_press_time.clone());
    let ms = |on: bool, time: &adw::SpinRow| on.then(|| (time.value() * 1000.0).round() as u32);
    let changed = Rc::new(changed);
    let items = radial_items.clone();
    let emit = Rc::new(move || {
        let prefs = Prefs {
            smoothing: Smoothing::ALL[rows.0.selected() as usize],
            button_2: ButtonAction::ALL[rows.1.selected() as usize],
            button_3: ButtonAction::ALL[rows.2.selected() as usize],
            eraser: ButtonAction::ALL[rows.3.selected() as usize],
            shape_hold_ms: ms(holds.0.enables_expansion(), &holds.1),
            long_press_ms: ms(holds.2.enables_expansion(), &holds.3),
            radial_items: items.borrow().clone(),
        };
        changed(&prefs);
    });
    for row in [&smoothing, &button_2, &button_3, &eraser] {
        let emit = emit.clone();
        row.connect_selected_notify(move |_| emit());
    }
    for row in [&shapes, &long_press] {
        let emit = emit.clone();
        row.connect_enable_expansion_notify(move |_| emit());
    }
    for time in [&shape_time, &long_press_time] {
        let emit = emit.clone();
        time.connect_value_notify(move |_| emit());
    }

    let pens = pens.to_vec();
    let d = dialog.clone();
    customize.connect_activated(move |_| {
        let (items, emit) = (radial_items.clone(), emit.clone());
        radial_editor::show(&d, radial_items.borrow().clone(), pens.clone(), move |new| {
            *items.borrow_mut() = new.to_vec();
            emit();
        });
    });
    dialog.present(Some(parent));
}
