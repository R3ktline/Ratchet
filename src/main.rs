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
use gtk::{gio, glib};

const APP_ID: &str = "app.ratchet.notes";

fn main() -> glib::ExitCode {
    gio::resources_register_include!("ratchet.gresource").expect("register resources");
    let app = adw::Application::builder().application_id(APP_ID).build();
    // Styles and icons come from the resources (see data/ratchet.gresource.xml).
    app.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
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
