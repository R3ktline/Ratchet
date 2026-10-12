// Phase 1 spike: verify GTK4 delivers pen pressure, tilt and full-rate
// samples (via the event history "backlog") from a drawing tablet.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib};

const APP_ID: &str = "app.ratchet.notes";

struct Segment {
    from: (f64, f64),
    to: (f64, f64),
    width: f64,
}

#[derive(Default)]
struct PenState {
    segments: Vec<Segment>,
    last: Option<(f64, f64)>,
    samples_this_second: u32,
    peak_samples: u32,
    pressures_seen: HashSet<i32>,
    device: String,
    pressure: f64,
    tilt: (Option<f64>, Option<f64>),
}

impl PenState {
    fn add_sample(&mut self, x: f64, y: f64, pressure: f64) {
        self.samples_this_second += 1;
        self.pressures_seen.insert((pressure * 1000.0).round() as i32);
        self.pressure = pressure;
        if let Some(from) = self.last {
            self.segments.push(Segment { from, to: (x, y), width: 0.5 + pressure * 8.0 });
        }
        self.last = Some((x, y));
    }
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let state = Rc::new(RefCell::new(PenState::default()));

    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    {
        let state = state.clone();
        area.set_draw_func(move |_, cr, _, _| {
            cr.set_source_rgb(0.12, 0.12, 0.13);
            let _ = cr.paint();
            cr.set_source_rgb(0.9, 0.9, 0.9);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            for s in &state.borrow().segments {
                cr.set_line_width(s.width);
                cr.move_to(s.from.0, s.from.1);
                cr.line_to(s.to.0, s.to.1);
                let _ = cr.stroke();
            }
        });
    }

    let stylus = gtk::GestureStylus::new();
    {
        let state = state.clone();
        let area = area.clone();
        stylus.connect_down(move |g, x, y| {
            let mut s = state.borrow_mut();
            s.last = None;
            // On Wayland device() is the seat's logical pointer; the
            // physical pen is identified by the device tool.
            s.device = g
                .device_tool()
                .map(|t| format!("{:?} tool, serial {}", t.tool_type(), t.serial()))
                .unwrap_or_else(|| "no tool".into());
            s.add_sample(x, y, g.axis(gdk::AxisUse::Pressure).unwrap_or(0.0));
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        stylus.connect_motion(move |g, x, y| {
            let mut s = state.borrow_mut();
            // The backlog holds intermediate samples the compositor merged
            // into this frame; replaying them gives the tablet's full rate.
            // Backlog coordinates are relative to the widget, like x/y.
            if let Some(backlog) = g.backlog() {
                for entry in backlog.iter() {
                    let axes = entry.axes();
                    let flags = entry.flags();
                    let get = |flag: gdk::AxisFlags, axis: gdk::AxisUse| {
                        flags.contains(flag).then(|| axes[axis.into_glib() as usize])
                    };
                    let bx = get(gdk::AxisFlags::X, gdk::AxisUse::X);
                    let by = get(gdk::AxisFlags::Y, gdk::AxisUse::Y);
                    let bp = get(gdk::AxisFlags::PRESSURE, gdk::AxisUse::Pressure);
                    if let (Some(bx), Some(by)) = (bx, by) {
                        s.add_sample(bx, by, bp.unwrap_or(0.0));
                    }
                }
            }
            s.add_sample(x, y, g.axis(gdk::AxisUse::Pressure).unwrap_or(0.0));
            s.tilt = (g.axis(gdk::AxisUse::Xtilt), g.axis(gdk::AxisUse::Ytilt));
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        stylus.connect_up(move |_, _, _| state.borrow_mut().last = None);
    }
    area.add_controller(stylus);

    let readout = gtk::Label::builder()
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(12)
        .margin_end(12)
        .xalign(0.0)
        .css_classes(["osd", "monospace"])
        .build();
    {
        let state = state.clone();
        let readout = readout.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let mut s = state.borrow_mut();
            let fmt = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or("none".into());
            readout.set_text(&format!(
                " device: {}\n pressure: {:.3}\n distinct pressures: {}\n tilt x/y: {} / {}\n samples/s: {} (peak {}) ",
                if s.device.is_empty() { "-" } else { &s.device },
                s.pressure,
                s.pressures_seen.len(),
                fmt(s.tilt.0),
                fmt(s.tilt.1),
                s.samples_this_second,
                s.peak_samples,
            ));
            s.peak_samples = s.peak_samples.max(s.samples_this_second);
            s.samples_this_second = 0;
            glib::ControlFlow::Continue
        });
    }

    let clear = gtk::Button::with_label("Clear");
    {
        let state = state.clone();
        let area = area.clone();
        clear.connect_clicked(move |_| {
            let mut s = state.borrow_mut();
            s.segments.clear();
            s.pressures_seen.clear();
            s.peak_samples = 0;
            area.queue_draw();
        });
    }

    let header = adw::HeaderBar::new();
    header.pack_start(&clear);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&readout);

    let content = adw::ToolbarView::new();
    content.add_top_bar(&header);
    content.set_content(Some(&overlay));

    adw::ApplicationWindow::builder()
        .application(app)
        .title("Ratchet · pen test")
        .default_width(1280)
        .default_height(800)
        .content(&content)
        .build()
        .present();
}
