//! Page settings popover: size (presets or custom), orientation,
//! background, and making it the section default.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

use crate::library::{Background, PageLayout, PageSize, PaperSize};
use crate::settings::LengthUnit;

pub struct Callbacks {
    /// Every edit of the page layout.
    pub changed: Box<dyn Fn(PageLayout)>,
    /// "Use for New Pages in This Section".
    pub make_default: Box<dyn Fn(PageLayout)>,
    /// The unit picked for custom sizes (remembered app-wide).
    pub unit_changed: Box<dyn Fn(LengthUnit)>,
}

/// Size choices in the dropdown: Infinite, the presets, then Custom.
fn size_index(size: PageSize) -> u32 {
    match size {
        PageSize::Infinite => 0,
        PageSize::Sheets { paper: PaperSize::Custom { .. }, .. } => PaperSize::PRESETS.len() as u32 + 1,
        PageSize::Sheets { paper, .. } => {
            PaperSize::PRESETS.iter().position(|p| *p == paper).map_or(0, |i| i as u32 + 1)
        }
    }
}

pub fn build(current: PageLayout, section_name: &str, unit: LengthUnit, cb: Callbacks) -> gtk::Popover {
    let layout = Rc::new(Cell::new(current));
    let cb = Rc::new(cb);
    let emit = {
        let (layout, cb) = (layout.clone(), cb.clone());
        Rc::new(move |l: PageLayout| {
            layout.set(l);
            (cb.changed)(l);
        })
    };

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .width_request(280)
        .build();
    let heading = |text: &str| gtk::Label::builder().label(text).xalign(0.0).css_classes(["heading"]).build();
    let row = |label: &str, widget: &gtk::Widget| {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        b.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
        b.append(widget);
        b
    };

    // Size
    content.append(&heading("Size"));
    let mut names = vec!["Infinite".to_string()];
    names.extend(PaperSize::PRESETS.iter().map(|p| p.label()));
    names.push("Custom".into());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let size = gtk::DropDown::from_strings(&names);
    size.set_selected(size_index(current.size));
    content.append(&size);

    let landscape = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let landscape_row = row("Landscape", landscape.upcast_ref());
    content.append(&landscape_row);

    // Custom size: width × height in the chosen unit.
    let unit = Rc::new(Cell::new(unit));
    let adj = || gtk::Adjustment::new(0.0, 0.0, 10_000.0, 1.0, 10.0, 0.0);
    let width = gtk::SpinButton::new(Some(&adj()), 1.0, 1);
    let height = gtk::SpinButton::new(Some(&adj()), 1.0, 1);
    let units = gtk::DropDown::from_strings(&LengthUnit::ALL.map(LengthUnit::label));
    units.set_selected(LengthUnit::ALL.iter().position(|u| *u == unit.get()).unwrap_or(0) as u32);
    let custom_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    custom_row.append(&width);
    custom_row.append(&gtk::Label::new(Some("×")));
    custom_row.append(&height);
    custom_row.append(&units);
    content.append(&custom_row);

    // Show a custom size's numbers in the current unit.
    let show_custom = {
        let (width, height, unit) = (width.clone(), height.clone(), unit.clone());
        Rc::new(move |paper: PaperSize| {
            let (w, h) = paper.mm();
            let k = unit.get().mm_per_unit();
            let digits = if unit.get() == LengthUnit::Mm { 0 } else { 2 };
            for (spin, v) in [(&width, w), (&height, h)] {
                spin.set_digits(digits);
                let a = spin.adjustment();
                a.set_lower(10.0 / k);
                a.set_upper(5000.0 / k);
                a.set_step_increment(if digits == 0 { 1.0 } else { 0.1 });
                spin.set_value(v as f64 / k);
            }
        })
    };

    // Reflect the layout in the controls (without emitting changes).
    let updating = Rc::new(Cell::new(false));
    let sync = {
        let (landscape, landscape_row, custom_row, show_custom, updating) =
            (landscape.clone(), landscape_row.clone(), custom_row.clone(), show_custom.clone(), updating.clone());
        Rc::new(move |l: PageLayout| {
            updating.set(true);
            match l.size {
                PageSize::Infinite => {
                    landscape_row.set_visible(false);
                    custom_row.set_visible(false);
                }
                PageSize::Sheets { paper, landscape: is_landscape } => {
                    landscape_row.set_visible(true);
                    landscape.set_active(is_landscape);
                    let custom = matches!(paper, PaperSize::Custom { .. });
                    custom_row.set_visible(custom);
                    if custom {
                        show_custom(paper);
                    }
                }
            }
            updating.set(false);
        })
    };
    sync(current);

    {
        let (layout, emit, sync, landscape, updating) =
            (layout.clone(), emit.clone(), sync.clone(), landscape.clone(), updating.clone());
        size.connect_selected_notify(move |d| {
            if updating.get() {
                return;
            }
            let i = d.selected() as usize;
            let mut l = layout.get();
            let keep_landscape = match l.size {
                PageSize::Sheets { landscape, .. } => landscape,
                PageSize::Infinite => landscape.is_active(),
            };
            l.size = if i == 0 {
                PageSize::Infinite
            } else if let Some(paper) = PaperSize::PRESETS.get(i - 1) {
                PageSize::Sheets { paper: *paper, landscape: keep_landscape }
            } else {
                // Start a custom size from the previous sheet size.
                let (w, h) = match l.size {
                    PageSize::Sheets { paper, .. } => paper.mm(),
                    PageSize::Infinite => PaperSize::A4.mm(),
                };
                PageSize::Sheets { paper: PaperSize::Custom { width_mm: w, height_mm: h }, landscape: keep_landscape }
            };
            emit(l);
            sync(l);
        });
    }
    {
        let (layout, emit, updating) = (layout.clone(), emit.clone(), updating.clone());
        landscape.connect_active_notify(move |s| {
            if updating.get() {
                return;
            }
            let mut l = layout.get();
            if let PageSize::Sheets { paper, .. } = l.size {
                l.size = PageSize::Sheets { paper, landscape: s.is_active() };
                emit(l);
            }
        });
    }
    for spin in [&width, &height] {
        let (layout, emit, updating, unit, width, height) =
            (layout.clone(), emit.clone(), updating.clone(), unit.clone(), width.clone(), height.clone());
        spin.connect_value_changed(move |_| {
            if updating.get() {
                return;
            }
            let mut l = layout.get();
            if let PageSize::Sheets { landscape, .. } = l.size {
                let k = unit.get().mm_per_unit();
                let paper = PaperSize::Custom {
                    width_mm: (width.value() * k) as f32,
                    height_mm: (height.value() * k) as f32,
                };
                l.size = PageSize::Sheets { paper, landscape };
                emit(l);
            }
        });
    }
    {
        let (layout, cb, updating) = (layout.clone(), cb.clone(), updating.clone());
        units.connect_selected_notify(move |d| {
            unit.set(LengthUnit::ALL[d.selected() as usize]);
            (cb.unit_changed)(unit.get());
            if let PageSize::Sheets { paper, .. } = layout.get().size {
                updating.set(true);
                show_custom(paper);
                updating.set(false);
            }
        });
    }

    // Background
    content.append(&heading("Background"));
    let backgrounds = gtk::Box::builder().css_classes(["linked"]).homogeneous(true).build();
    let mut group: Option<gtk::ToggleButton> = None;
    for (label, bg) in [
        ("Blank", Background::Blank),
        ("Lined", Background::Lined),
        ("Grid", Background::Grid),
        ("Dotted", Background::Dotted),
    ] {
        let b = gtk::ToggleButton::builder().label(label).active(current.background == bg).build();
        if let Some(g) = &group {
            b.set_group(Some(g));
        } else {
            group = Some(b.clone());
        }
        let (layout, emit) = (layout.clone(), emit.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                emit(PageLayout { background: bg, ..layout.get() });
            }
        });
        backgrounds.append(&b);
    }
    content.append(&backgrounds);

    let popover = gtk::Popover::new();
    let default_button = gtk::Button::builder()
        .label(format!("Use for New Pages in “{section_name}”"))
        .margin_top(6)
        .build();
    {
        let popover = popover.clone();
        default_button.connect_clicked(move |_| {
            (cb.make_default)(layout.get());
            popover.popdown();
        });
    }
    content.append(&default_button);
    let note = gtk::Label::builder()
        .label("Paper sizes turn to match the screen when new pages are created.")
        .wrap(true)
        .max_width_chars(36)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    content.append(&note);

    popover.set_child(Some(&content));
    popover
}
