//! Preferences window.

use adw::prelude::*;

use crate::settings::{ButtonAction, Smoothing};

pub struct Prefs {
    pub smoothing: Smoothing,
    pub button_2: ButtonAction,
    pub button_3: ButtonAction,
    pub eraser: ButtonAction,
}

/// Show the preferences; `changed` runs with the new values on every edit.
pub fn show(parent: &impl IsA<gtk::Widget>, current: Prefs, changed: impl Fn(&Prefs) + 'static) {
    let smoothing = adw::ComboRow::builder()
        .title("Pen smoothing")
        .subtitle("Higher irons out more wobble; the line trails the pen a little more")
        .model(&gtk::StringList::new(&Smoothing::ALL.map(Smoothing::label)))
        .selected(Smoothing::ALL.iter().position(|s| *s == current.smoothing).unwrap_or(2) as u32)
        .build();
    let ink = adw::PreferencesGroup::builder().title("Ink").build();
    ink.add(&smoothing);

    let actions = gtk::StringList::new(&ButtonAction::ALL.map(ButtonAction::label));
    let pos = |a: ButtonAction| ButtonAction::ALL.iter().position(|x| *x == a).unwrap_or(0) as u32;
    let button_2 = adw::ComboRow::builder()
        .title("Side button (middle click)")
        .subtitle("Hold while touching the tablet")
        .model(&actions)
        .selected(pos(current.button_2))
        .build();
    let button_3 = adw::ComboRow::builder()
        .title("Side button (right click)")
        .subtitle("Hold while touching the tablet")
        .model(&actions)
        .selected(pos(current.button_3))
        .build();
    let eraser = adw::ComboRow::builder()
        .title("Eraser mode")
        .subtitle("Flipped pen, or a button your tablet driver maps to eraser")
        .model(&actions)
        .selected(pos(current.eraser))
        .build();
    let buttons = adw::PreferencesGroup::builder()
        .title("Pen Buttons")
        .description("Turn on Debug › Pen Input Overlay (RATCHET_DEBUG=1) to see which button is which.")
        .build();
    buttons.add(&button_2);
    buttons.add(&button_3);
    buttons.add(&eraser);

    let page = adw::PreferencesPage::builder().title("Pen").icon_name("document-edit-symbolic").build();
    page.add(&ink);
    page.add(&buttons);
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);

    let rows = (smoothing.clone(), button_2.clone(), button_3.clone(), eraser.clone());
    let changed = std::rc::Rc::new(changed);
    let emit = move || {
        let prefs = Prefs {
            smoothing: Smoothing::ALL[rows.0.selected() as usize],
            button_2: ButtonAction::ALL[rows.1.selected() as usize],
            button_3: ButtonAction::ALL[rows.2.selected() as usize],
            eraser: ButtonAction::ALL[rows.3.selected() as usize],
        };
        changed(&prefs);
    };
    let emit = std::rc::Rc::new(emit);
    for row in [&smoothing, &button_2, &button_3, &eraser] {
        let emit = emit.clone();
        row.connect_selected_notify(move |_| emit());
    }
    dialog.present(Some(parent));
}
