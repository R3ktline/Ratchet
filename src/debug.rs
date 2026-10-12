//! Debug tools, enabled with `RATCHET_DEBUG=1`.

use adw::prelude::*;
use gtk::glib;

pub fn enabled() -> bool {
    std::env::var("RATCHET_DEBUG").is_ok_and(|v| v == "1")
}

/// A small window with a scrolling event log and Copy / Clear buttons.
pub struct LogWindow {
    pub window: adw::Window,
    buffer: gtk::TextBuffer,
    view: gtk::TextView,
}

impl LogWindow {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let buffer = gtk::TextBuffer::new(None);
        let view = gtk::TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(8)
            .right_margin(8)
            .top_margin(8)
            .bottom_margin(8)
            .build();
        let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy All").build();
        let clear = gtk::Button::builder().icon_name("edit-clear-all-symbolic").tooltip_text("Clear").build();
        let header = adw::HeaderBar::new();
        header.pack_start(&copy);
        header.pack_start(&clear);
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&gtk::ScrolledWindow::builder().child(&view).build()));
        let window = adw::Window::builder()
            .title("Ratchet Event Log")
            .default_width(520)
            .default_height(420)
            .transient_for(parent)
            .hide_on_close(true)
            .content(&content)
            .build();

        let b = buffer.clone();
        copy.connect_clicked(move |button| {
            let text = b.text(&b.start_iter(), &b.end_iter(), false);
            button.clipboard().set_text(&text);
        });
        let b = buffer.clone();
        clear.connect_clicked(move |_| b.set_text(""));

        LogWindow { window, buffer, view }
    }

    pub fn append(&self, line: &str) {
        let time = glib::DateTime::now_local()
            .ok()
            .and_then(|t| t.format("%H:%M:%S.%f").ok())
            .map(|s| s[..s.len().min(12)].to_string())
            .unwrap_or_default();
        let mut end = self.buffer.end_iter();
        self.buffer.insert(&mut end, &format!("{time} {line}\n"));
        // Keep the log bounded.
        let lines = self.buffer.line_count();
        if lines > 2000 {
            let mut start = self.buffer.start_iter();
            let mut cut = self.buffer.iter_at_line(lines - 2000).unwrap_or(self.buffer.start_iter());
            self.buffer.delete(&mut start, &mut cut);
        }
        let mark = self.buffer.create_mark(None, &self.buffer.end_iter(), false);
        self.view.scroll_mark_onscreen(&mark);
        self.buffer.delete_mark(&mark);
    }
}
