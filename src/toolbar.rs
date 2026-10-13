//! Drawing tools in the header bar: pen slots, eraser, undo/redo.
//!
//! Clicking a slot selects it; clicking the already selected slot opens its
//! editor (color, width, remove). Same for the eraser (eraser mode). With
//! the select tool active and strokes selected, clicking a slot restyles
//! the selection instead.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::canvas::{Action, TextCommand};
use crate::text::TextSize;
use crate::ink::{InkColor, Tool};
use crate::glyphs::{draw_color, eraser_glyph, hand_glyph, select_glyph, slot_glyph, text_glyph};
use crate::settings::{ActiveTool, EraserMode, PenSlot};

/// Colors offered in the slot editor.
const PALETTE: [InkColor; 12] = [
    InkColor::Ink,
    InkColor::Rgb(0x1c, 0x71, 0xd8),
    InkColor::Rgb(0xe0, 0x1b, 0x24),
    InkColor::Rgb(0x26, 0xa2, 0x69),
    InkColor::Rgb(0xe6, 0x61, 0x00),
    InkColor::Rgb(0x91, 0x41, 0xac),
    InkColor::Rgb(0x21, 0x90, 0xa4),
    InkColor::Rgb(0x86, 0x5e, 0x3c),
    InkColor::Rgb(0xd5, 0x61, 0x99),
    InkColor::Rgb(0xf6, 0xd3, 0x2d),
    InkColor::Rgb(0x8f, 0xf0, 0xa4),
    InkColor::Rgb(0x77, 0x76, 0x7b),
];

pub struct Callbacks {
    /// Tool or pen settings changed.
    pub on_change: Box<dyn Fn(&ToolState)>,
    /// A slot was picked while selecting/typing; return true if it was used
    /// to restyle (the current tool then stays active).
    pub on_pick: Box<dyn Fn(&PenSlot) -> bool>,
    pub on_text: Box<dyn Fn(TextCommand)>,
}

#[derive(Clone)]
pub struct ToolState {
    pub pens: Vec<PenSlot>,
    pub active: ActiveTool,
    pub eraser_mode: EraserMode,
    pub text_size: TextSize,
    pub text_font: Option<String>,
    pub show_text_tool: bool,
}

pub struct Toolbar {
    pub widget: gtk::Box,
    /// Undo and redo, placed by the window (on the far side of the bar).
    pub history: gtk::Box,
    /// Pen and highlighter slots, in the order they were added.
    slots: gtk::Box,
    eraser: gtk::ToggleButton,
    select: gtk::ToggleButton,
    text: gtk::ToggleButton,
    hand: gtk::ToggleButton,
    text_options: gtk::Revealer,
    size: gtk::DropDown,
    font: gtk::FontDialogButton,
    on_text: Box<dyn Fn(TextCommand)>,
    buttons: RefCell<Vec<gtk::ToggleButton>>,
    /// Set while re-activating the select tool after a restyle.
    restoring: Cell<bool>,
    /// Offered a slot picked while selecting; true if it was used to
    /// restyle the selection (so the select tool stays active).
    on_pick: Box<dyn Fn(&PenSlot) -> bool>,
    state: RefCell<ToolState>,
    /// Set by `toggled` so the following `clicked` knows the button wasn't
    /// already active (and shouldn't open the editor).
    just_activated: Cell<bool>,
    on_change: Box<dyn Fn(&ToolState)>,
}

fn swatch(color: InkColor, selected: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(24).content_height(24).build();
    area.set_draw_func(move |area, cr, w, h| {
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let (r, g, b) = draw_color(color);
        cr.set_source_rgb(r, g, b);
        cr.arc(cx, cy, 8.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        // Outline so colors matching the background stay visible.
        let fg = area.color();
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.3);
        cr.set_line_width(1.0);
        cr.arc(cx, cy, 8.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        if selected {
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
            cr.set_line_width(2.0);
            cr.arc(cx, cy, 11.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
        }
    });
    area
}

impl Toolbar {
    pub fn new(state: ToolState, callbacks: Callbacks) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let slots = gtk::Box::builder().css_classes(["rt-slots"]).build();
        let eraser = gtk::ToggleButton::builder()
            .child(&eraser_glyph())
            .tooltip_text("Eraser (click again for options)")
            .css_classes(["flat"])
            .build();
        let select = gtk::ToggleButton::builder()
            .child(&select_glyph())
            .tooltip_text("Select strokes — draw a loop or tap a stroke")
            .css_classes(["flat"])
            .group(&eraser)
            .build();

        // Adds pens and highlighters (they tile after the first pen) and
        // turns on optional tools.
        let add_menu = gio::Menu::new();
        let add_slots = gio::Menu::new();
        add_slots.append(Some("Pen"), Some("tools.add::pen"));
        add_slots.append(Some("Highlighter"), Some("tools.add::highlighter"));
        add_menu.append_section(None, &add_slots);
        let extra = gio::Menu::new();
        extra.append(Some("Text Tool"), Some("tools.show-text"));
        add_menu.append_section(None, &extra);
        let add = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Tool")
            .menu_model(&add_menu)
            .css_classes(["flat", "rt-add"])
            .build();

        let text = gtk::ToggleButton::builder()
            .child(&text_glyph())
            .tooltip_text("Text (T) — click to type, drag to set a width; $…$ for math")
            .css_classes(["flat"])
            .group(&eraser)
            .build();

        let hand = gtk::ToggleButton::builder()
            .child(&hand_glyph())
            .tooltip_text("Pan — drag the page")
            .css_classes(["flat"])
            .group(&eraser)
            .build();

        // Text options, shown while the text tool is active.
        let size = gtk::DropDown::from_strings(&TextSize::ALL.map(TextSize::label));
        size.set_selected(TextSize::ALL.iter().position(|s| *s == state.text_size).unwrap_or(1) as u32);
        size.set_tooltip_text(Some("Text Size"));
        size.set_focus_on_click(false);
        let font = gtk::FontDialogButton::builder()
            .dialog(&gtk::FontDialog::builder().title("Text Font").build())
            .level(gtk::FontLevel::Family)
            .use_font(true)
            .tooltip_text("Text Font")
            .focus_on_click(false)
            .build();
        if let Some(f) = &state.text_font {
            font.set_font_desc(&gtk::pango::FontDescription::from_string(f));
        }
        let text_box = gtk::Box::builder().css_classes(["rt-tool-group"]).valign(gtk::Align::Center).build();
        text_box.append(&size);
        text_box.append(&font);
        let style_buttons: Vec<(gtk::Button, TextCommand)> = [
            ("format-text-bold-symbolic", "Bold (Ctrl+B)", TextCommand::Bold),
            ("format-text-italic-symbolic", "Italic (Ctrl+I)", TextCommand::Italic),
            ("format-text-underline-symbolic", "Underline (Ctrl+U)", TextCommand::Underline),
        ]
        .into_iter()
        .map(|(icon, tip, cmd)| {
            let b = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .css_classes(["flat"])
                .focus_on_click(false)
                .build();
            text_box.append(&b);
            (b, cmd)
        })
        .collect();
        let text_options = gtk::Revealer::builder()
            .child(&text_box)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .reveal_child(state.active == ActiveTool::Text)
            .visible(state.active == ActiveTool::Text)
            .build();

        let undo = gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text("Undo (Ctrl+Z)")
            .action_name("win.undo")
            .css_classes(["flat"])
            .build();
        let redo = gtk::Button::builder()
            .icon_name("edit-redo-symbolic")
            .tooltip_text("Redo (Ctrl+Shift+Z)")
            .action_name("win.redo")
            .css_classes(["flat"])
            .build();

        // One cluster: the pens, then eraser, select and pan (and text when
        // turned on), then "+"; text options follow while typing. History
        // goes wherever the window puts it.
        let group = |children: &[&gtk::Widget]| {
            let b = gtk::Box::builder().css_classes(["rt-tool-group"]).valign(gtk::Align::Center).build();
            for c in children {
                b.append(*c);
            }
            b
        };
        let divider = gtk::Separator::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(crate::platform::px(6.0))
            .margin_bottom(crate::platform::px(6.0))
            .build();
        text.set_visible(state.show_text_tool);
        widget.add_css_class("rt-toolbar");
        widget.append(&group(&[
            slots.upcast_ref(),
            divider.upcast_ref(),
            eraser.upcast_ref(),
            select.upcast_ref(),
            hand.upcast_ref(),
            text.upcast_ref(),
            add.upcast_ref(),
        ]));
        widget.append(&text_options);
        let history = group(&[undo.upcast_ref(), redo.upcast_ref()]);

        let this = Rc::new(Toolbar {
            widget,
            history,
            slots,
            eraser,
            select,
            text,
            hand,
            text_options,
            size,
            font,
            on_text: callbacks.on_text,
            buttons: RefCell::default(),
            restoring: Cell::new(false),
            on_pick: callbacks.on_pick,
            state: RefCell::new(state),
            just_activated: Cell::new(false),
            on_change: callbacks.on_change,
        });

        let group = gio::SimpleActionGroup::new();
        let add_action = gio::SimpleAction::new("add", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(&this);
        add_action.connect_activate(move |_, arg| {
            let Some(this) = weak.upgrade() else { return };
            let tool = match arg.and_then(|v| v.str()) {
                Some("highlighter") => Tool::Highlighter,
                _ => Tool::Pen,
            };
            let color = match tool {
                Tool::Pen => InkColor::Ink,
                Tool::Highlighter => PALETTE[9],
            };
            let index = {
                let mut st = this.state.borrow_mut();
                st.pens.push(PenSlot::new(tool, color));
                st.active = ActiveTool::Slot(st.pens.len() - 1);
                st.pens.len() - 1
            };
            this.rebuild();
            this.changed();
            // Open the new pen's editor right away to pick its color.
            this.edit_slot(index);
        });
        group.add_action(&add_action);
        let show_text = gio::SimpleAction::new_stateful("show-text", None, &this.state.borrow().show_text_tool.to_variant());
        let weak = Rc::downgrade(&this);
        show_text.connect_activate(move |action, _| {
            let Some(this) = weak.upgrade() else { return };
            let show = !action.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            action.set_state(&show.to_variant());
            this.text.set_visible(show);
            this.state.borrow_mut().show_text_tool = show;
            if !show && this.text.is_active() {
                this.select_tool(ActiveTool::Slot(0));
            }
            this.changed();
        });
        group.add_action(&show_text);
        this.widget.insert_action_group("tools", Some(&group));

        let weak = Rc::downgrade(&this);
        this.eraser.connect_toggled(move |b| {
            let Some(this) = weak.upgrade() else { return };
            if b.is_active() {
                this.just_activated.set(true);
                this.state.borrow_mut().active = ActiveTool::Eraser;
                this.changed();
            }
        });
        let weak = Rc::downgrade(&this);
        this.eraser.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if !this.just_activated.replace(false) {
                this.edit_eraser();
            }
        });

        let weak = Rc::downgrade(&this);
        this.select.connect_toggled(move |b| {
            let Some(this) = weak.upgrade() else { return };
            if b.is_active() {
                if !this.restoring.replace(false) {
                    this.just_activated.set(true);
                }
                this.state.borrow_mut().active = ActiveTool::Select;
                this.changed();
            }
        });
        let weak = Rc::downgrade(&this);
        this.select.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.just_activated.set(false);
            }
        });

        let weak = Rc::downgrade(&this);
        this.hand.connect_toggled(move |b| {
            let Some(this) = weak.upgrade() else { return };
            if b.is_active() {
                this.restoring.set(false);
                this.state.borrow_mut().active = ActiveTool::Hand;
                this.changed();
            }
        });
        let weak = Rc::downgrade(&this);
        this.text.connect_toggled(move |b| {
            let Some(this) = weak.upgrade() else { return };
            if b.is_active() {
                this.restoring.set(false);
                this.state.borrow_mut().active = ActiveTool::Text;
                this.changed();
            }
            this.show_text_options(b.is_active());
        });
        for (b, cmd) in style_buttons {
            let weak = Rc::downgrade(&this);
            b.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    (this.on_text)(cmd.clone());
                }
            });
        }
        let weak = Rc::downgrade(&this);
        this.size.connect_selected_notify(move |d| {
            let Some(this) = weak.upgrade() else { return };
            let size = TextSize::ALL[d.selected() as usize];
            this.state.borrow_mut().text_size = size;
            this.changed();
            (this.on_text)(TextCommand::Size(size));
        });
        let weak = Rc::downgrade(&this);
        this.font.connect_font_desc_notify(move |b| {
            let Some(this) = weak.upgrade() else { return };
            let font = b.font_desc().and_then(|d| d.family()).map(|f| f.to_string());
            this.state.borrow_mut().text_font = font.clone();
            this.changed();
            (this.on_text)(TextCommand::Font(font));
        });

        // Redraw glyphs when the theme flips (adaptive ink color).
        let weak = Rc::downgrade(&this);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.rebuild();
            }
        });

        this.rebuild();
        this
    }

    /// Switch tools as if the button had been clicked (radial menu).
    pub fn select_tool(&self, tool: ActiveTool) {
        match tool {
            ActiveTool::Slot(i) => {
                if let Some(b) = self.buttons.borrow().get(i) {
                    b.set_active(true);
                }
            }
            ActiveTool::Eraser => self.eraser.set_active(true),
            ActiveTool::Select => self.select.set_active(true),
            ActiveTool::Text => {
                // Picking text (T key, radial menu) shows its button too.
                if !self.text.is_visible() {
                    let _ = self.widget.activate_action("tools.show-text", None);
                }
                self.text.set_active(true);
            }
            ActiveTool::Hand => self.hand.set_active(true),
        }
        // Not a click on an already active button: never open an editor.
        self.just_activated.set(false);
    }

    fn show_text_options(&self, show: bool) {
        if show {
            self.text_options.set_visible(true);
        }
        self.text_options.set_reveal_child(show);
        if !show {
            let r = self.text_options.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                if !r.reveals_child() {
                    r.set_visible(false);
                }
            });
        }
    }

    /// Highlight the tool a pen input is temporarily using.
    pub fn set_temp_tool(&self, action: Option<Action>) {
        for (button, a) in [(&self.eraser, Action::Erase), (&self.select, Action::Select)] {
            if action == Some(a) {
                button.add_css_class("temp-tool");
            } else {
                button.remove_css_class("temp-tool");
            }
        }
    }

    fn changed(&self) {
        let st = self.state.borrow().clone();
        (self.on_change)(&st);
    }

    fn rebuild(self: &Rc<Self>) {
        while let Some(child) = self.slots.first_child() {
            self.slots.remove(&child);
        }
        let st = self.state.borrow().clone();
        let mut buttons = Vec::new();
        for (i, slot) in st.pens.iter().enumerate() {
            let kind = match slot.tool {
                Tool::Pen => "Pen",
                Tool::Highlighter => "Highlighter",
            };
            let button = gtk::ToggleButton::builder()
                .child(&slot_glyph(slot))
                .tooltip_text(format!("{kind} {:.2} mm (click again to edit)", slot.width_mm))
                .css_classes(["flat"])
                .group(&self.eraser)
                .active(st.active == ActiveTool::Slot(i))
                .build();
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |b| {
                let Some(this) = weak.upgrade() else { return };
                if b.is_active() {
                    this.just_activated.set(true);
                    let (previous, slot) = {
                        let st = this.state.borrow();
                        (st.active, st.pens[i].clone())
                    };
                    let keep = match previous {
                        ActiveTool::Select => Some(this.select.clone()),
                        ActiveTool::Text => Some(this.text.clone()),
                        _ => None,
                    };
                    if let Some(keep) = keep.filter(|_| (this.on_pick)(&slot)) {
                        // Restyled the selection / text: stay on that tool.
                        let select = keep;
                        let weak = Rc::downgrade(&this);
                        gtk::glib::idle_add_local_once(move || {
                            if let Some(this) = weak.upgrade() {
                                this.restoring.set(true);
                                select.set_active(true);
                            }
                        });
                        return;
                    }
                    this.state.borrow_mut().active = ActiveTool::Slot(i);
                    this.changed();
                }
            });
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                let Some(this) = weak.upgrade() else { return };
                if !this.just_activated.replace(false) {
                    this.edit_slot(i);
                }
            });
            self.slots.append(&button);
            buttons.push(button);
        }
        match st.active {
            ActiveTool::Eraser => self.eraser.set_active(true),
            ActiveTool::Select => self.select.set_active(true),
            ActiveTool::Text => self.text.set_active(true),
            ActiveTool::Hand => self.hand.set_active(true),
            ActiveTool::Slot(_) => {}
        }
        self.just_activated.set(false);
        *self.buttons.borrow_mut() = buttons;
    }

    fn update_slot(self: &Rc<Self>, index: usize, f: impl FnOnce(&mut PenSlot)) {
        {
            let mut st = self.state.borrow_mut();
            let Some(slot) = st.pens.get_mut(index) else { return };
            f(slot);
        }
        // Swap the glyph in place: rebuilding would close the open editor.
        let slot = self.state.borrow().pens[index].clone();
        if let Some(button) = self.buttons.borrow().get(index) {
            button.set_child(Some(&slot_glyph(&slot)));
            let kind = match slot.tool {
                Tool::Pen => "Pen",
                Tool::Highlighter => "Highlighter",
            };
            button.set_tooltip_text(Some(&format!("{kind} {:.2} mm (click again to edit)", slot.width_mm)));
        }
        self.changed();
    }

    fn edit_slot(self: &Rc<Self>, index: usize) {
        let Some(button) = self.buttons.borrow().get(index).cloned() else { return };
        let slot = self.state.borrow().pens[index].clone();
        let popover = gtk::Popover::new();
        popover.set_parent(&button);
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(crate::platform::px(12.0))
            .css_classes(["rt-popover"])
            .build();

        // Colors, then the custom color picker as the last swatch.
        let colors = gtk::FlowBox::builder()
            .max_children_per_line(7)
            .min_children_per_line(7)
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .build();
        let swatch_buttons: Rc<RefCell<Vec<(InkColor, gtk::Button)>>> = Rc::default();
        let refresh_swatches = {
            let swatch_buttons = swatch_buttons.clone();
            Rc::new(move |current: InkColor| {
                for (c, b) in swatch_buttons.borrow().iter() {
                    b.set_child(Some(&swatch(*c, *c == current)));
                }
            })
        };
        for color in PALETTE {
            let b = gtk::Button::builder()
                .child(&swatch(color, color == slot.color))
                .css_classes(["flat", "circular"])
                .tooltip_text(if color == InkColor::Ink { "Ink (black, white in dark mode)" } else { "" })
                .build();
            let weak = Rc::downgrade(self);
            let refresh = refresh_swatches.clone();
            b.connect_clicked(move |_| {
                let Some(this) = weak.upgrade() else { return };
                this.update_slot(index, |s| s.color = color);
                refresh(color);
            });
            swatch_buttons.borrow_mut().push((color, b.clone()));
            colors.append(&b);
        }
        let custom = gtk::Button::builder()
            .icon_name("color-select-symbolic")
            .tooltip_text("Custom Color")
            .css_classes(["flat", "circular"])
            .build();
        colors.append(&custom);
        content.append(&colors);

        let weak = Rc::downgrade(self);
        let refresh = refresh_swatches.clone();
        let pop = popover.clone();
        custom.connect_clicked(move |b| {
            let Some(this) = weak.upgrade() else { return };
            let current = this.state.borrow().pens[index].color;
            let (r, g, bl) = current.resolve(false);
            let initial = gdk::RGBA::new(r, g, bl, 1.0);
            let window = b.root().and_downcast::<gtk::Window>();
            let dialog = gtk::ColorDialog::builder().with_alpha(false).build();
            let weak = Rc::downgrade(&this);
            let refresh = refresh.clone();
            pop.popdown();
            dialog.choose_rgba(window.as_ref(), Some(&initial), gio::Cancellable::NONE, move |res| {
                let (Some(this), Ok(c)) = (weak.upgrade(), res) else { return };
                let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let color = InkColor::Rgb(to8(c.red()), to8(c.green()), to8(c.blue()));
                this.update_slot(index, |s| s.color = color);
                refresh(color);
            });
        });

        // Width: presets + slider
        let presets = PenSlot::presets(slot.tool);
        let (lo, hi) = PenSlot::range(slot.tool);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, 0.05);
        scale.set_value(slot.width_mm as f64);
        scale.set_hexpand(true);
        scale.set_width_request(crate::platform::px(180.0));
        let value_label = gtk::Label::new(Some(&format!("{:.2} mm", slot.width_mm)));
        value_label.set_width_chars(8);

        // Presets aren't a toggle group: between presets none is active.
        let preset_box = gtk::Box::builder().css_classes(["linked"]).homogeneous(true).build();
        let preset_buttons: Rc<RefCell<Vec<(f32, gtk::ToggleButton)>>> = Rc::default();
        for (name, mm) in ["Fine", "Medium", "Thick"].into_iter().zip(presets) {
            let b = gtk::ToggleButton::builder()
                .label(name)
                .active((slot.width_mm - mm).abs() < 0.01)
                .build();
            let s = scale.clone();
            b.connect_clicked(move |b| {
                b.set_active(true);
                s.set_value(mm as f64);
            });
            preset_buttons.borrow_mut().push((mm, b.clone()));
            preset_box.append(&b);
        }
        content.append(&preset_box);

        let weak = Rc::downgrade(self);
        let label = value_label.clone();
        scale.connect_value_changed(move |s| {
            let Some(this) = weak.upgrade() else { return };
            let mm = s.value() as f32;
            label.set_text(&format!("{mm:.2} mm"));
            for (preset, b) in preset_buttons.borrow().iter() {
                b.set_active((preset - mm).abs() < 0.01);
            }
            this.update_slot(index, |slot| slot.width_mm = mm);
        });
        // Width slider, then remove at the end of the row.
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove")
            .css_classes(["flat"])
            .sensitive(self.state.borrow().pens.len() > 1)
            .build();
        let width_row = gtk::Box::new(gtk::Orientation::Horizontal, crate::platform::px(6.0));
        width_row.append(&scale);
        width_row.append(&value_label);
        width_row.append(&remove);
        content.append(&width_row);

        let weak: Weak<Self> = Rc::downgrade(self);
        let pop = popover.clone();
        remove.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            pop.popdown();
            {
                let mut st = this.state.borrow_mut();
                st.pens.remove(index);
                st.active = match st.active {
                    ActiveTool::Slot(i) if i == index => ActiveTool::Slot(0),
                    ActiveTool::Slot(i) if i > index => ActiveTool::Slot(i - 1),
                    other => other,
                };
            }
            this.rebuild();
            this.changed();
        });

        popover.set_child(Some(&content));
        popover.popup();
    }

    fn edit_eraser(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        popover.set_parent(&self.eraser);
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        let mode = self.state.borrow().eraser_mode;
        let modes = adw::ToggleGroup::builder().homogeneous(true).css_classes(["rt-popover"]).build();
        modes.add(adw::Toggle::builder().name("stroke").label("Whole Stroke").tooltip("Removes the strokes you touch").build());
        modes.add(adw::Toggle::builder().name("partial").label("Partial").tooltip("Erases only where it passes").build());
        modes.set_active_name(Some(if mode == EraserMode::Partial { "partial" } else { "stroke" }));
        let weak = Rc::downgrade(self);
        modes.connect_active_name_notify(move |g| {
            let Some(this) = weak.upgrade() else { return };
            this.state.borrow_mut().eraser_mode =
                if g.active_name().as_deref() == Some("partial") { EraserMode::Partial } else { EraserMode::Stroke };
            this.changed();
        });
        popover.set_child(Some(&modes));
        popover.popup();
    }
}
