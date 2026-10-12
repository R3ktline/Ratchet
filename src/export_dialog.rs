//! Export dialog: format and options, then a save dialog.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use crate::settings::{ExportFormat, ExportOptions};

/// Ask for options and a destination, then call `export` with them (the
/// options carry the chosen folder, to be remembered).
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    file_name: &str,
    current: ExportOptions,
    export: impl Fn(ExportOptions, PathBuf) + 'static,
) {
    let format = adw::ComboRow::builder()
        .title("Format")
        .model(&gtk::StringList::new(&["PDF", "PNG image (200 DPI)"]))
        .selected(if current.format == ExportFormat::Png { 1 } else { 0 })
        .build();
    let split = adw::ComboRow::builder()
        .title("Infinite pages")
        .subtitle("Pages with sheets always export one page per sheet")
        .model(&gtk::StringList::new(&["Split into A4 pages", "One page, cropped to the content"]))
        .selected(if current.split_a4 { 0 } else { 1 })
        .build();
    let background = adw::SwitchRow::builder()
        .title("Background pattern")
        .subtitle("Lines, grid or dots")
        .active(current.background)
        .build();
    let open_after = adw::SwitchRow::builder().title("Open after export").active(current.open_after).build();

    let group = adw::PreferencesGroup::builder().description("Exports always use a light (white paper) look.").build();
    group.add(&format);
    group.add(&split);
    group.add(&background);
    group.add(&open_after);

    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::builder().label("Export…").css_classes(["suggested-action"]).build();
    let header = adw::HeaderBar::builder().show_start_title_buttons(false).show_end_title_buttons(false).build();
    header.pack_start(&cancel);
    header.pack_end(&go);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder().title(title).content_width(440).child(&view).build();

    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });

    let export = Rc::new(export);
    let file_name = file_name.to_string();
    let d = dialog.clone();
    go.connect_clicked(move |_| {
        let opts = ExportOptions {
            format: if format.selected() == 1 { ExportFormat::Png } else { ExportFormat::Pdf },
            split_a4: split.selected() == 0,
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
