//! Preferences window.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

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

/// A switch row to turn a hold gesture on/off, and a slider row for its
/// duration (disabled while the switch is off).
fn hold_rows(title: &str, subtitle: &str, value_ms: Option<u32>, default_ms: u32) -> (adw::SwitchRow, adw::ActionRow, gtk::Scale) {
    let switch = adw::SwitchRow::builder().title(title).subtitle(subtitle).active(value_ms.is_some()).build();
    let secs = value_ms.unwrap_or(default_ms) as f64 / 1000.0;
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, HOLD_MIN, HOLD_MAX, 0.05);
    scale.set_value(secs);
    scale.set_hexpand(true);
    scale.set_width_request(220);
    scale.set_valign(gtk::Align::Center);
    let label = gtk::Label::builder().label(format!("{secs:.2} s")).width_chars(6).xalign(1.0).build();
    let l = label.clone();
    scale.connect_value_changed(move |s| l.set_text(&format!("{:.2} s", s.value())));
    let row = adw::ActionRow::builder().title("Hold time").build();
    row.add_suffix(&scale);
    row.add_suffix(&label);
    switch.bind_property("active", &row, "sensitive").sync_create().build();
    (switch, row, scale)
}

/// Show the preferences; `changed` runs with the new values on every edit.
pub fn show(parent: &impl IsA<gtk::Widget>, current: Prefs, pens: &[PenSlot], changed: impl Fn(&Prefs) + 'static) {
    let smoothing = adw::ComboRow::builder()
        .title("Pen smoothing")
        .subtitle("Higher irons out more wobble; the line trails the pen a little more")
        .model(&gtk::StringList::new(&Smoothing::ALL.map(Smoothing::label)))
        .selected(Smoothing::ALL.iter().position(|s| *s == current.smoothing).unwrap_or(2) as u32)
        .build();
    let (shapes, shape_time_row, shape_time) = hold_rows(
        "Hold still to make a shape",
        "At the end of a stroke: straight lines (snapping to 15°), arrows and circles",
        current.shape_hold_ms,
        500,
    );
    let ink = adw::PreferencesGroup::builder().title("Ink").build();
    ink.add(&smoothing);
    ink.add(&shapes);
    ink.add(&shape_time_row);

    let actions = gtk::StringList::new(&ButtonAction::ALL.map(ButtonAction::label));
    let pos = |a: ButtonAction| ButtonAction::ALL.iter().position(|x| *x == a).unwrap_or(0) as u32;
    let button_row = |title: &str, subtitle: &str, value: ButtonAction| {
        adw::ComboRow::builder().title(title).subtitle(subtitle).model(&actions).selected(pos(value)).build()
    };
    let button_2 = button_row("Side button (middle click)", "Press while hovering for the menu, or hold while touching", current.button_2);
    let button_3 = button_row("Side button (right click)", "Hold while touching the tablet", current.button_3);
    let eraser = button_row("Eraser mode", "Flipped pen, or a button your tablet driver maps to eraser", current.eraser);
    let buttons = adw::PreferencesGroup::builder()
        .title("Pen Buttons")
        .description("Turn on Debug › Pen Input Overlay (RATCHET_DEBUG=1) to see which button is which.")
        .build();
    buttons.add(&button_2);
    buttons.add(&button_3);
    buttons.add(&eraser);

    // Radial menu
    let (long_press, long_press_row, long_press_time) =
        hold_rows("Hold the pen still to open the menu", "Without moving, anywhere on the page", current.long_press_ms, 720);
    let customize = adw::ButtonRow::builder().title("Customize Menu…").end_icon_name("go-next-symbolic").build();
    let radial = adw::PreferencesGroup::builder()
        .title("Radial Menu")
        .description("Point at an item and release the menu button, or touch, slide toward it and lift. The center or Esc closes it.")
        .build();
    radial.add(&long_press);
    radial.add(&long_press_row);
    radial.add(&customize);

    let page = adw::PreferencesPage::builder().title("Pen").icon_name("document-edit-symbolic").build();
    page.add(&ink);
    page.add(&buttons);
    page.add(&radial);
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);

    let radial_items = Rc::new(RefCell::new(current.radial_items));
    let rows = (smoothing.clone(), button_2.clone(), button_3.clone(), eraser.clone());
    let holds = (shapes.clone(), shape_time.clone(), long_press.clone(), long_press_time.clone());
    let ms = |on: bool, scale: &gtk::Scale| on.then(|| (scale.value() * 1000.0).round() as u32);
    let changed = Rc::new(changed);
    let items = radial_items.clone();
    let emit = Rc::new(move || {
        let prefs = Prefs {
            smoothing: Smoothing::ALL[rows.0.selected() as usize],
            button_2: ButtonAction::ALL[rows.1.selected() as usize],
            button_3: ButtonAction::ALL[rows.2.selected() as usize],
            eraser: ButtonAction::ALL[rows.3.selected() as usize],
            shape_hold_ms: ms(holds.0.is_active(), &holds.1),
            long_press_ms: ms(holds.2.is_active(), &holds.3),
            radial_items: items.borrow().clone(),
        };
        changed(&prefs);
    });
    for row in [&smoothing, &button_2, &button_3, &eraser] {
        let emit = emit.clone();
        row.connect_selected_notify(move |_| emit());
    }
    for switch in [&shapes, &long_press] {
        let emit = emit.clone();
        switch.connect_active_notify(move |_| emit());
    }
    for scale in [&shape_time, &long_press_time] {
        let emit = emit.clone();
        scale.connect_value_changed(move |_| emit());
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
