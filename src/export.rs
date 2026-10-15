//! Export pages to PDF (vector, with bookmarks) or PNG (200 DPI).
//!
//! Always rendered light: white paper, adaptive ink black. Pages become
//! output sheets:
//! - sheet pages: one output page per sheet that has ink (at least one);
//! - infinite pages: cut into A4 pages top to bottom, scaled down to fit
//!   the width if needed, or one page cropped to the ink.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use gtk::{gsk, pango};

use crate::canvas::{draw_stroke, rect, rgba};
use crate::ink::{self, Tool};
use crate::library::{Background, PageFile, PaperSize};
use crate::settings::ExportOptions;
use crate::text;

/// Must match the canvas' gap between sheets.
const SHEET_GAP: f32 = 24.0;
/// Margin around cropped / split infinite pages.
const MARGIN_MM: f32 = 10.0;
const PNG_DPI: f64 = 200.0;
/// Page units are CSS px (96 per inch); PDF uses points (72 per inch).
const PT_PER_PX: f64 = 72.0 / 96.0;

/// One output page, in page units.
pub struct Sheet {
    pub width: f32,
    pub height: f32,
    pub node: Option<gsk::RenderNode>,
}

/// A bookmark pointing at the first sheet of a page or section.
pub struct Bookmark {
    pub title: String,
    /// 0 = top level (section in a notebook export), 1 = nested.
    pub level: usize,
    pub sheet: usize,
}

#[derive(Default)]
pub struct Doc {
    pub title: String,
    pub sheets: Vec<Sheet>,
    pub bookmarks: Vec<Bookmark>,
}

/// Content extent: (min x, min y, max x, max y), or None if empty.
fn content_bounds(page: &PageFile, texts: &[text::TextBox]) -> Option<ink::Bounds> {
    let strokes = page.strokes.iter().map(|s| s.bounds);
    let texts = texts.iter().map(|t| t.bounds());
    strokes.chain(texts).reduce(|a, b| ink::Bounds {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    })
}

/// Background lines drawn individually (crisp vectors in the PDF).
fn draw_pattern(s: &gtk::Snapshot, bg: Background, area: gtk::graphene::Rect) {
    let spacing = match bg {
        Background::Blank => return,
        Background::Lined => 8.0 * ink::PX_PER_MM,
        Background::Grid | Background::Dotted => 5.0 * ink::PX_PER_MM,
    };
    let color = rgba((0.0, 0.0, 0.0), 0.13);
    let line = 0.75;
    let (x0, y0, x1, y1) = (area.x(), area.y(), area.x() + area.width(), area.y() + area.height());
    let first = |v: f32| (v / spacing).ceil() * spacing;
    match bg {
        Background::Lined => {
            let mut y = first(y0);
            while y < y1 {
                s.append_color(&color, &rect(x0, y, x1 - x0, line));
                y += spacing;
            }
        }
        Background::Grid => {
            let mut y = first(y0);
            while y < y1 {
                s.append_color(&color, &rect(x0, y, x1 - x0, line));
                y += spacing;
            }
            let mut x = first(x0);
            while x < x1 {
                s.append_color(&color, &rect(x, y0, line, y1 - y0));
                x += spacing;
            }
        }
        Background::Dotted => {
            let dot = rgba((0.0, 0.0, 0.0), 0.35);
            let mut y = first(y0);
            while y < y1 {
                let mut x = first(x0);
                while x < x1 {
                    s.append_color(&dot, &rect(x - 0.75, y - 0.75, 1.5, 1.5));
                    x += spacing;
                }
                y += spacing;
            }
        }
        Background::Blank => {}
    }
}

/// Ink and text in page coordinates, highlighter under the ink, text on top.
fn draw_content(s: &gtk::Snapshot, page: &PageFile, texts: &[text::TextBox], ctx: Option<&pango::Context>) {
    for tool in [Tool::Highlighter, Tool::Pen] {
        for stroke in page.strokes.iter().filter(|st| st.tool == tool) {
            draw_stroke(s, stroke, false);
        }
    }
    if let Some(ctx) = ctx {
        for t in texts {
            let laid = text::layout(t, ctx, false);
            text::draw(s, t, &laid, &rgba(t.color.resolve(false), 1.0));
        }
    }
}

/// Where a page area goes on an output sheet.
struct Placement {
    /// Paper size.
    size: (f32, f32),
    /// Page area shown (page units).
    src: gtk::graphene::Rect,
    offset: (f32, f32),
    scale: f32,
}

impl Placement {
    fn plain(size: (f32, f32), src: gtk::graphene::Rect) -> Self {
        Placement { size, src, offset: (0.0, 0.0), scale: 1.0 }
    }
}

/// Render one output sheet: white paper showing a page area.
fn sheet(
    page: &PageFile,
    texts: &[text::TextBox],
    ctx: Option<&pango::Context>,
    opts: &ExportOptions,
    at: Placement,
) -> Sheet {
    let Placement { size, src, offset, scale } = at;
    let s = gtk::Snapshot::new();
    s.append_color(&rgba((1.0, 1.0, 1.0), 1.0), &rect(0.0, 0.0, size.0, size.1));
    s.save();
    s.translate(&gtk::graphene::Point::new(offset.0, offset.1));
    s.scale(scale, scale);
    s.translate(&gtk::graphene::Point::new(-src.x(), -src.y()));
    s.push_clip(&src);
    if opts.background {
        draw_pattern(&s, page.layout.background, src);
    }
    draw_content(&s, page, texts, ctx);
    s.pop();
    s.restore();
    Sheet { width: size.0, height: size.1, node: s.to_node() }
}

/// Lay a page out as output sheets.
pub fn page_sheets(page: &PageFile, ctx: Option<&pango::Context>, opts: &ExportOptions) -> Vec<Sheet> {
    // Text boxes need their measured size for bounds.
    let texts: Vec<text::TextBox> = page
        .texts
        .iter()
        .map(|t| {
            let mut t = t.clone();
            if let Some(ctx) = ctx {
                t.measure(ctx);
            }
            t
        })
        .collect();
    let bounds = content_bounds(page, &texts);
    let margin = MARGIN_MM * ink::PX_PER_MM;
    let a4 = {
        let (w, h) = PaperSize::A4.mm();
        (w * ink::PX_PER_MM, h * ink::PX_PER_MM)
    };

    match page.layout.size.sheet_px() {
        Some((w, h)) => {
            let pitch = h + SHEET_GAP;
            let count = bounds.map_or(1, |b| (b.y1 / pitch).floor().max(0.0) as usize + 1);
            (0..count)
                .map(|i| {
                    let src = rect(0.0, i as f32 * pitch, w, h);
                    sheet(page, &texts, ctx, opts, Placement::plain((w, h), src))
                })
                .collect()
        }
        None => {
            let Some(b) = bounds else {
                // Empty page: one blank A4.
                return vec![sheet(page, &texts, ctx, opts, Placement::plain(a4, rect(0.0, 0.0, a4.0, a4.1)))];
            };
            let (cw, ch) = (b.x1 - b.x0, b.y1 - b.y0);
            if !opts.split_a4 {
                let size = (cw + 2.0 * margin, ch + 2.0 * margin);
                let src = rect(b.x0 - margin, b.y0 - margin, size.0, size.1);
                return vec![sheet(page, &texts, ctx, opts, Placement::plain(size, src))];
            }
            // Fit the width, then cut top to bottom. Each slice spans the
            // whole printable width, so the background fills the sheet
            // even when the content is narrower.
            let scale = ((a4.0 - 2.0 * margin) / cw).min(1.0);
            let (slice_w, slice) = ((a4.0 - 2.0 * margin) / scale, (a4.1 - 2.0 * margin) / scale);
            let count = (ch / slice).ceil().max(1.0) as usize;
            (0..count)
                .map(|k| {
                    let src = rect(b.x0, b.y0 + k as f32 * slice, slice_w, slice);
                    sheet(page, &texts, ctx, opts, Placement { size: a4, src, offset: (margin, margin), scale })
                })
                .collect()
        }
    }
}

/// One page to export, with its place in the bookmark tree.
pub struct Entry {
    pub page: PageFile,
    /// Section name when exporting a notebook (starts a new top bookmark).
    pub section: Option<String>,
    pub title: String,
}

/// Build the whole document.
pub fn build(title: &str, entries: &[Entry], ctx: Option<&pango::Context>, opts: &ExportOptions) -> Doc {
    let mut doc = Doc { title: title.to_string(), ..Default::default() };
    let nested = entries.iter().any(|e| e.section.is_some());
    let mut current_section: Option<&str> = None;
    for e in entries {
        if let Some(section) = e.section.as_deref()
            && current_section != Some(section)
        {
            current_section = Some(section);
            doc.bookmarks.push(Bookmark { title: section.to_string(), level: 0, sheet: doc.sheets.len() });
        }
        doc.bookmarks.push(Bookmark {
            title: e.title.clone(),
            level: usize::from(nested),
            sheet: doc.sheets.len(),
        });
        doc.sheets.extend(page_sheets(&e.page, ctx, opts));
    }
    doc
}

pub fn write_pdf(doc: &Doc, path: &Path) -> Result<(), String> {
    let err = |e: gtk::cairo::Error| e.to_string();
    let first = doc.sheets.first().ok_or("nothing to export")?;
    let surface = gtk::cairo::PdfSurface::new(
        first.width as f64 * PT_PER_PX,
        first.height as f64 * PT_PER_PX,
        path,
    )
    .map_err(err)?;
    let _ = surface.set_metadata(gtk::cairo::PdfMetadata::Title, &doc.title);
    let _ = surface.set_metadata(gtk::cairo::PdfMetadata::Creator, "Ratchet");
    let cr = gtk::cairo::Context::new(&surface).map_err(err)?;
    for s in &doc.sheets {
        surface.set_size(s.width as f64 * PT_PER_PX, s.height as f64 * PT_PER_PX).map_err(err)?;
        cr.save().map_err(err)?;
        cr.scale(PT_PER_PX, PT_PER_PX);
        if let Some(node) = &s.node {
            node.draw(&cr);
        }
        cr.restore().map_err(err)?;
        cr.show_page().map_err(err)?;
    }
    // Bookmarks: sections at the top level, pages under them.
    let mut parent = 0;
    for b in &doc.bookmarks {
        let link = format!("page={}", b.sheet + 1);
        let under = if b.level == 0 { 0 } else { parent };
        let id = surface.add_outline(under, &b.title, &link, gtk::cairo::PdfOutline::empty()).map_err(err)?;
        if b.level == 0 {
            parent = id;
        }
    }
    surface.finish();
    Ok(())
}

/// PNG files for the sheets: `path` for one sheet, else `name-01.png`, …
/// (`name 2-01.png`, … when any of those already exist: the file dialog
/// only asked about overwriting `path`).
pub fn write_png(doc: &Doc, path: &Path) -> Result<Vec<PathBuf>, String> {
    let err = |e: gtk::cairo::Error| e.to_string();
    let scale = PNG_DPI / 96.0;
    let base = path.file_stem().and_then(|s| s.to_str()).unwrap_or("export").to_string();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let many = doc.sheets.len() > 1;
    let sheet_file = |stem: &str, i: usize| dir.join(format!("{stem}-{:02}.png", i + 1));
    let mut stem = base.clone();
    for n in 2.. {
        if !many || (0..doc.sheets.len()).all(|i| !sheet_file(&stem, i).exists()) {
            break;
        }
        stem = format!("{base} {n}");
    }
    let mut written = Vec::new();
    for (i, s) in doc.sheets.iter().enumerate() {
        let (w, h) = ((s.width as f64 * scale).ceil() as i32, (s.height as f64 * scale).ceil() as i32);
        let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).map_err(err)?;
        let cr = gtk::cairo::Context::new(&surface).map_err(err)?;
        cr.scale(scale, scale);
        if let Some(node) = &s.node {
            node.draw(&cr);
        }
        drop(cr);
        let file = if many { sheet_file(&stem, i) } else { path.to_path_buf() };
        let mut out = std::fs::File::create(&file).map_err(|e| e.to_string())?;
        surface.write_to_png(&mut out).map_err(|e| e.to_string())?;
        written.push(file);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::{InkColor, Point, Stroke};
    use crate::library::{PageLayout, PageSize};

    fn page(layout: PageLayout, ys: &[f32]) -> PageFile {
        let strokes = ys
            .iter()
            .map(|&y| {
                let mut s = Stroke::new(Tool::Pen, InkColor::Ink, 2.0);
                s.points = vec![Point { x: 50.0, y, p: 0.5 }, Point { x: 300.0, y, p: 0.5 }];
                s.update_bounds();
                s
            })
            .collect();
        PageFile {
            version: 1,
            title: "T".into(),
            created: String::new(),
            modified: String::new(),
            layout,
            strokes,
            texts: Vec::new(),
            extra: Default::default(),
        }
    }

    /// GTK can only be initialized from one thread per process, and each
    /// test runs on its own thread, so all GTK-using checks share one test.
    /// Skipped where there's no display to initialize GTK with.
    #[test]
    fn export_layout_and_files() {
        if gtk::init().is_err() {
            return;
        }
        sheet_counts();
        writes_pdf_with_pages_and_png();
    }

    fn sheet_counts() {
        let opts = ExportOptions { background: true, ..Default::default() };
        let a4 = PageLayout { size: PageSize::Sheets { paper: PaperSize::A4, landscape: false }, background: Background::Grid };
        // Ink on sheets 1 and 3 -> 3 sheets (trailing blank one dropped).
        let h = a4.size.sheet_px().unwrap().1 + SHEET_GAP;
        assert_eq!(page_sheets(&page(a4, &[100.0, 2.0 * h + 100.0]), None, &opts).len(), 3);
        // Infinite page 3 A4-heights tall -> split into several A4 pages.
        let inf = PageLayout::default();
        let tall = page(inf, &[0.0, 3000.0]);
        let split = page_sheets(&tall, None, &opts);
        assert!(split.len() >= 3);
        let cropped = page_sheets(&tall, None, &ExportOptions { split_a4: false, ..opts.clone() });
        assert_eq!(cropped.len(), 1);
        assert!(cropped[0].height > 3000.0);
    }

    fn writes_pdf_with_pages_and_png() {
        let dir = std::env::temp_dir().join(format!("ratchet-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = ExportOptions::default();
        let entries = vec![
            Entry { page: page(PageLayout::default(), &[0.0, 3000.0]), section: Some("Course".into()), title: "Lecture 1".into() },
            Entry { page: page(PageLayout::default(), &[10.0]), section: Some("Lab".into()), title: "Lab 1".into() },
        ];
        let doc = build("Calculus", &entries, None, &opts);
        assert_eq!(doc.bookmarks.len(), 4);
        let pdf = dir.join("out.pdf");
        write_pdf(&doc, &pdf).unwrap();
        let bytes = std::fs::read(&pdf).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        // Bookmarks and pages live in compressed object streams, so they
        // can't be checked as plain text here; keep the file to inspect it
        // (e.g. with pdfinfo) by setting RATCHET_KEEP_EXPORT=<path>.
        assert!(bytes.len() > 1000);
        if let Ok(keep) = std::env::var("RATCHET_KEEP_EXPORT") {
            std::fs::copy(&pdf, keep).unwrap();
        }
        let pngs = write_png(&doc, &dir.join("out.png")).unwrap();
        assert_eq!(pngs.len(), doc.sheets.len());
        assert!(std::fs::read(&pngs[0]).unwrap().starts_with(b"\x89PNG"));
        // A second multi-sheet export doesn't overwrite the first.
        let again = write_png(&doc, &dir.join("out.png")).unwrap();
        assert!(again[0].ends_with("out 2-01.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
