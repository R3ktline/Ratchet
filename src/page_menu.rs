//! Page settings popover: size (presets or custom), orientation,
//! background, and making it the section default.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

use crate::library::{Background, PageLayout, PageSize, PaperSize};
use crate::platform::px;
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

fn background_name(bg: Background) -> &'static str {
    match bg {
        Background::Blank => "blank",
        Background::Lined => "lined",
        Background::Grid => "grid",
        Background::Dotted => "dotted",
    }
}

/// A small sheet of paper showing a background pattern.
fn background_preview(bg: Background) -> gtk::DrawingArea {
    // A small sheet with room around it, and a sparse pattern that reads at
    // a glance.
    let area = gtk::DrawingArea::builder()
        .content_width(px(26.0))
        .content_height(px(20.0))
        .margin_top(px(4.0))
        .margin_bottom(px(4.0))
        .halign(gtk::Align::Center)
        .build();
    area.set_draw_func(move |area, cr, w, h| {
        let fg = area.color();
        let (r, g, b) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
        let s = crate::platform::ui_scale();
        let (x0, y0, x1, y1) = (1.0 * s, 1.0 * s, w as f64 - 1.0 * s, h as f64 - 1.0 * s);
        cr.set_line_width(1.0 * s);
        cr.set_source_rgba(r, g, b, 0.7);
        cr.rectangle(x0, y0, x1 - x0, y1 - y0);
        let _ = cr.stroke();
        let step = 5.0 * s;
        cr.set_source_rgba(r, g, b, 0.45);
        cr.set_line_width(0.8 * s);
        match bg {
            Background::Blank => {}
            Background::Lined => {
                let mut y = y0 + step;
                while y < y1 - 1.0 {
                    cr.move_to(x0 + 2.0 * s, y);
                    cr.line_to(x1 - 2.0 * s, y);
                    y += step;
                }
                let _ = cr.stroke();
            }
            Background::Grid => {
                let mut y = y0 + step;
                while y < y1 - 1.0 {
                    cr.move_to(x0, y);
                    cr.line_to(x1, y);
                    y += step;
                }
                let mut x = x0 + step;
                while x < x1 - 1.0 {
                    cr.move_to(x, y0);
                    cr.line_to(x, y1);
                    x += step;
                }
                let _ = cr.stroke();
            }
            Background::Dotted => {
                cr.set_source_rgba(r, g, b, 0.7);
                let mut y = y0 + step;
                while y < y1 - 1.0 {
                    let mut x = x0 + step;
                    while x < x1 - 1.0 {
                        cr.arc(x, y, 0.8 * s, 0.0, std::f64::consts::TAU);
                        let _ = cr.fill();
                        x += step;
                    }
                    y += step;
                }
            }
        }
    });
    area
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
        .spacing(px(12.0))
        .width_request(px(300.0))
        .css_classes(["rt-popover"])
        .build();

    // Size
    let mut names = vec!["Infinite".to_string()];
    names.extend(PaperSize::PRESETS.iter().map(|p| p.label()));
    names.push("Custom".into());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let size = adw::ComboRow::builder()
        .title("Paper")
        .model(&gtk::StringList::new(&names))
        .selected(size_index(current.size))
        .tooltip_text("Sheets turn to match the screen on new pages")
        .build();
    size.add_prefix(&gtk::Image::from_icon_name("document-page-setup-symbolic"));

    let landscape = adw::SwitchRow::builder().title("Landscape").build();
    landscape.add_prefix(&gtk::Image::from_icon_name("object-rotate-right-symbolic"));

    // Custom size: width and height in the chosen unit.
    let unit = Rc::new(Cell::new(unit));
    let spin = |title: &str| {
        let adj = gtk::Adjustment::new(0.0, 0.0, 10_000.0, 1.0, 10.0, 0.0);
        adw::SpinRow::builder().title(title).adjustment(&adj).digits(1).build()
    };
    let width = spin("Width");
    let height = spin("Height");
    let units = adw::ComboRow::builder()
        .title("Unit")
        .model(&gtk::StringList::new(&LengthUnit::ALL.map(LengthUnit::label)))
        .selected(LengthUnit::ALL.iter().position(|u| *u == unit.get()).unwrap_or(0) as u32)
        .build();

    let size_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for row in [size.upcast_ref::<gtk::Widget>(), landscape.upcast_ref(), width.upcast_ref(), height.upcast_ref(), units.upcast_ref()] {
        size_list.append(row);
    }
    content.append(&size_list);

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
        let (landscape, width, height, units, show_custom, updating) =
            (landscape.clone(), width.clone(), height.clone(), units.clone(), show_custom.clone(), updating.clone());
        Rc::new(move |l: PageLayout| {
            updating.set(true);
            let (sheets, custom) = match l.size {
                PageSize::Infinite => (false, false),
                PageSize::Sheets { paper, landscape: is_landscape } => {
                    landscape.set_active(is_landscape);
                    let custom = matches!(paper, PaperSize::Custom { .. });
                    if custom {
                        show_custom(paper);
                    }
                    (true, custom)
                }
            };
            landscape.set_visible(sheets);
            for row in [width.upcast_ref::<gtk::Widget>(), height.upcast_ref(), units.upcast_ref()] {
                row.set_visible(custom);
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
        spin.connect_value_notify(move |_| {
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

    // Background: pictures of the patterns, names in tooltips.
    const BACKGROUNDS: [(Background, &str); 4] = [
        (Background::Blank, "Blank"),
        (Background::Lined, "Lines"),
        (Background::Grid, "Grid"),
        (Background::Dotted, "Dots"),
    ];
    let backgrounds = adw::ToggleGroup::builder().homogeneous(true).build();
    for (bg, tip) in BACKGROUNDS {
        backgrounds.add(adw::Toggle::builder().name(background_name(bg)).child(&background_preview(bg)).tooltip(tip).build());
    }
    backgrounds.set_active_name(Some(background_name(current.background)));
    {
        let (layout, emit) = (layout.clone(), emit.clone());
        backgrounds.connect_active_name_notify(move |g| {
            let name = g.active_name();
            let Some((bg, _)) = BACKGROUNDS.iter().find(|(bg, _)| Some(background_name(*bg)) == name.as_deref()) else {
                return;
            };
            emit(PageLayout { background: *bg, ..layout.get() });
        });
    }
    content.append(&backgrounds);

    let popover = gtk::Popover::new();
    let default_button = gtk::Button::builder()
        .child(&adw::ButtonContent::builder().icon_name("starred-symbolic").label("Set as Default").build())
        .tooltip_text(format!("Use for new pages in “{section_name}”"))
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();
    {
        let popover = popover.clone();
        default_button.connect_clicked(move |_| {
            (cb.make_default)(layout.get());
            popover.popdown();
        });
    }
    content.append(&default_button);

    popover.set_child(Some(&content));
    crate::ui::keep_icon_labels_aligned(&popover);
    popover
}
