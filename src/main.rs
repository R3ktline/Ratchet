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
mod preferences;
mod radial_editor;
mod settings;
mod shapes;
mod text;
mod toolbar;
mod window;

use adw::prelude::*;
use gtk::{gdk, glib};

const APP_ID: &str = "app.ratchet.notes";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
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
