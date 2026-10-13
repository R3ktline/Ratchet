//! Export dialog: format and options, then a save dialog.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::platform::px;
use crate::settings::{ExportFormat, ExportOptions};

/// What is being exported, as shown at the top of the dialog.
pub struct Subject<'a> {
    pub name: &'a str,
    /// e.g. "Page" or "Section · 12 pages".
    pub detail: &'a str,
    /// Whether any exported page is infinite (only then does the
    /// infinite-page choice matter).
    pub has_infinite: bool,
}

/// A row of choices, each with an icon and a short label.
fn choices(items: &[(&str, &str, &str, &str)], active: &str) -> adw::ToggleGroup {
    let group = adw::ToggleGroup::builder().homogeneous(true).build();
    for (name, icon, label, tip) in items {
        let mut toggle = adw::Toggle::builder().name(*name).icon_name(*icon).label(*label);
        if !tip.is_empty() {
            toggle = toggle.tooltip(*tip);
        }
        group.add(toggle.build());
    }
    group.set_active_name(Some(active));
    group
}

/// Ask for options and a destination, then call `export` with them (the
/// options carry the chosen folder, to be remembered).
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    subject: Subject,
    file_name: &str,
    current: ExportOptions,
    export: impl Fn(ExportOptions, PathBuf) + 'static,
) {
    // What
    let what = adw::ActionRow::builder()
        .title(subject.name)
        .subtitle(subject.detail)
        .title_lines(1)
        .activatable(false)
        .build();
    what.add_prefix(&gtk::Image::from_icon_name("x-office-document-symbolic"));
    let what_group = adw::PreferencesGroup::new();
    what_group.add(&what);

    // Format
    let format = choices(
        &[
            ("pdf", "x-office-document-symbolic", "PDF", ""),
            ("png", "image-x-generic-symbolic", "Image", "PNG, 200 DPI"),
        ],
        if current.format == ExportFormat::Png { "png" } else { "pdf" },
    );
    let format_group = adw::PreferencesGroup::builder().title("Format").build();
    format_group.add(&format);

    // Infinite pages
    let split = choices(
        &[
            ("split", "view-paged-symbolic", "A4 Pages", "Split into A4 pages"),
            ("whole", "view-continuous-symbolic", "One Page", "One page, cropped to the content"),
        ],
        if current.split_a4 { "split" } else { "whole" },
    );
    let split_group = adw::PreferencesGroup::builder()
        .title("Infinite Pages")
        .visible(subject.has_infinite)
        .build();
    split_group.add(&split);

    // Options
    let background = adw::SwitchRow::builder()
        .title("Page Pattern")
        .tooltip_text("Lines, grid or dots")
        .active(current.background)
        .build();
    background.add_prefix(&gtk::Image::from_icon_name("view-grid-symbolic"));
    let open_after = adw::SwitchRow::builder().title("Open When Done").active(current.open_after).build();
    open_after.add_prefix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    let options = adw::PreferencesGroup::builder()
        .title("Options")
        .tooltip_text("Exports always use white paper")
        .build();
    options.add(&background);
    options.add(&open_after);

    let cancel = gtk::Button::with_label("Cancel");
    // css_classes replaces the "text-button" class a label adds; keep it.
    let go = gtk::Button::builder().label("Export").css_classes(["text-button", "suggested-action"]).build();
    let header = adw::HeaderBar::builder().show_start_title_buttons(false).show_end_title_buttons(false).build();
    header.pack_start(&cancel);
    header.pack_end(&go);

    let page = adw::PreferencesPage::new();
    for group in [&what_group, &format_group, &split_group, &options] {
        page.add(group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("Export")
        .content_width(px(560.0))
        .child(&view)
        .build();
    crate::ui::keep_icon_labels_aligned(&dialog);

    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });

    let export = Rc::new(export);
    let file_name = file_name.to_string();
    let d = dialog.clone();
    go.connect_clicked(move |_| {
        let opts = ExportOptions {
            format: if format.active_name().as_deref() == Some("png") { ExportFormat::Png } else { ExportFormat::Pdf },
            split_a4: split.active_name().as_deref() != Some("whole"),
            background: background.is_active(),
            open_after: open_after.is_active(),
            folder: current.folder.clone(),
        };
        let (ext, mime, label) = match opts.format {
            ExportFormat::Pdf => ("pdf", "application/pdf", "PDF documents"),
            ExportFormat::Png => ("png", "image/png", "PNG images"),
        };
        let filter = gtk::FileFilter::new();
        filter.add_mime_type(mime);
        filter.set_name(Some(label));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let save = gtk::FileDialog::builder()
            .title("Export To")
            .initial_name(format!("{file_name}.{ext}"))
            .filters(&filters)
            .default_filter(&filter)
            .build();
        if let Some(folder) = &opts.folder {
            save.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        let window = d.root().and_downcast::<gtk::Window>();
        let export = export.clone();
        d.close();
        save.save(window.as_ref(), gio::Cancellable::NONE, move |res| {
            let Some(mut path) = res.ok().and_then(|f| f.path()) else { return };
            if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(ext)) {
                path.set_extension(ext);
            }
            let mut opts = opts.clone();
            opts.folder = path.parent().map(|p| p.to_path_buf());
            export(opts, path);
        });
    });
    dialog.present(Some(parent));
}
