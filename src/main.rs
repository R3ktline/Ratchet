// No console window next to the app in Windows release builds; debug builds
// keep it so stderr output stays visible.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod canvas;
mod debug;
mod export;
mod export_dialog;
mod glyphs;
mod ink;
mod library;
mod math;
mod naming;
mod page_menu;
mod platform;
mod preferences;
mod radial_editor;
mod settings;
mod shapes;
mod text;
mod title_layout;
mod toolbar;
mod ui;
mod window;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "app.ratchet.notes";

fn main() -> glib::ExitCode {
    platform::early_setup();
    gio::resources_register_include!("ratchet.gresource").expect("register resources");
    let app = adw::Application::builder().application_id(APP_ID).build();
    // Styles and icons come from the resources (see data/ratchet.gresource.xml).
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        platform::setup();
        load_sizes();
    });
    app.connect_activate(|app| {
        // Single instance: re-activating focuses the existing window.
        if let Some(win) = app.active_window() {
            win.present();
            return;
        }
        window::Window::new(app).present();
    });
    app.run()
}

/// App-wide sizes, in a stylesheet of their own so they follow the UI
/// scale (style.css has the rest). See the UI rules in CLAUDE.md: these
/// apply to every header bar, dialog and menu, not just the main window.
fn load_sizes() {
    let s = platform::ui_scale();
    let px = |v: f64| format!("{}px", (v * s).round());
    let css = format!(
        "\
headerbar, .rt-bar {{ min-height: {header}; }}
/* 22px + the header's own 6px inner inset = the 28px a dialog page's
   content sits in from its edge, so header buttons line up with it. */
headerbar {{ padding: {gap} 22px; }}
.rt-bar {{ min-height: 0; padding: {gap} {pad_bar}; }}
.rt-bar-side {{ border-spacing: {gap}; }}
headerbar box.start, headerbar box.end {{ border-spacing: {gap}; }}
headerbar box.start > button, headerbar box.end > button,
headerbar box.start > menubutton > button, headerbar box.end > menubutton > button,
.rt-bar-side > button, .rt-bar-side > menubutton > button, .rt-tool-group button {{
  min-height: {button}; min-width: {button}; padding: 0; border-radius: {radius};
}}
headerbar box.start > button.text-button, headerbar box.end > button.text-button,
headerbar box.start > button.image-text-button, headerbar box.end > button.image-text-button,
.rt-bar button.text-button, .rt-bar button.image-text-button {{ padding: 0 {text_pad}; }}preferencespage > scrolledwindow > viewport > clamp {{ margin: 0; padding: 0; }}
preferencespage > scrolledwindow > viewport > clamp > box {{ margin-left: 0; margin-right: 0; }}
.rt-toolbar {{ border-spacing: {group_gap}; }}
.rt-tool-group {{ padding: {inset}; border-spacing: {inner}; border-radius: {group_radius}; }}
.rt-slots {{ border-spacing: {inner}; }}
.rt-tool-group button {{ border-radius: {radius}; }}
/* Export stands beside the clusters, so it takes their height and corners. */
button.rt-export {{ min-height: {group_h}; border-radius: {group_radius}; }}
.nb-dot {{ min-width: {dot}; min-height: {dot}; }}
.rt-popover {{ padding: {pad_x}; }}
popover.menu > contents {{ padding: {pad_y_menu}; }}
popover.menu modelbutton {{ min-height: {menu_row}; padding: 0 {menu_pad}; border-radius: {radius}; }}
popover.menu modelbutton > check, popover.menu modelbutton > radio {{ margin-right: {menu_pad}; }}
popover.menu modelbutton accelerator {{ margin-left: {menu_gap}; }}
popover.menu separator {{ margin: {pad_y_menu} 0; }}
popover.menu .title {{ padding: {pad_y_menu} {menu_pad} {pad_y}; }}
",
        header = px(46.0),
        gap = px(8.0),
        button = px(30.0),
        pad_y = px(3.0),
        pad_x = px(7.0),
        radius = px(7.0),
        group_gap = px(16.0),
        inset = px(3.0),
        inner = px(4.0),
        group_radius = px(9.0),
        // A cluster is a button plus its inset on both sides.
        group_h = px(30.0 + 2.0 * 3.0),
        dot = px(10.0),
        pad_bar = px(12.0),
        pad_y_menu = px(6.0),
        menu_row = px(34.0),
        menu_pad = px(12.0),
        menu_gap = px(24.0),
        text_pad = px(16.0),
    );
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
    }
}
