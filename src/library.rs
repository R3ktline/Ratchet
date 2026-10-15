//! Notebooks on disk: a folder of human-readable folders and files.
//!
//! ```text
//! <root>/
//!   .ratchet-library.json          notebook order
//!   Calculus II/
//!     .ratchet-notebook.json       name, color, section order
//!     Course/
//!       .ratchet-section.json      name, naming pattern, page order + titles
//!       Lecture 1.json             page (title, timestamps, strokes)
//! ```
//!
//! Folders and files the metadata doesn't know about (e.g. synced in from
//! another machine) are picked up on load; entries whose files vanished are
//! dropped. Display names live in the metadata, so names with characters
//! that are invalid in file names still round-trip.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ink::Stroke;
use crate::text::TextBox;
use crate::naming::{Pattern, Today};

const LIBRARY_META: &str = ".ratchet-library.json";
const NOTEBOOK_META: &str = ".ratchet-notebook.json";
const SECTION_META: &str = ".ratchet-section.json";
const PAGE_EXT: &str = "json";
const FORMAT_VERSION: u32 = 1;

/// Notebook colors, matching style.css `.nb-<name>` classes.
pub const PALETTE: [&str; 10] =
    ["blue", "green", "orange", "purple", "red", "teal", "yellow", "pink", "brown", "slate"];

/// Sections every new notebook starts with.
const PRESET_SECTIONS: [&str; 2] = ["Course", "Assignments"];

/// Page-name prefixes for well-known section names; others use the
/// section name itself.
const SECTION_PREFIXES: [(&str, &str); 3] =
    [("Course", "Lecture"), ("Assignments", "Assignment"), ("Laboratory", "Lab")];

#[derive(Serialize, Deserialize)]
struct LibraryMeta {
    version: u32,
    notebooks: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct NotebookMeta {
    version: u32,
    name: String,
    color: String,
    sections: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct SectionMeta {
    version: u32,
    name: String,
    pattern: Pattern,
    #[serde(default)]
    page_layout: PageLayout,
    pages: Vec<PageRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageRef {
    pub file: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaperSize {
    A3,
    #[default]
    A4,
    A5,
    Letter,
    Legal,
    Tabloid,
    #[serde(rename = "slide-16x9")]
    Slide16x9,
    #[serde(rename = "slide-4x3")]
    Slide4x3,
    /// Portrait width and height in mm.
    Custom { width_mm: f32, height_mm: f32 },
}

impl PaperSize {
    pub const PRESETS: [PaperSize; 8] = [
        PaperSize::A3,
        PaperSize::A4,
        PaperSize::A5,
        PaperSize::Letter,
        PaperSize::Legal,
        PaperSize::Tabloid,
        PaperSize::Slide16x9,
        PaperSize::Slide4x3,
    ];

    /// Width and height in mm before applying landscape.
    pub fn mm(self) -> (f32, f32) {
        match self {
            PaperSize::A3 => (297.0, 420.0),
            PaperSize::A4 => (210.0, 297.0),
            PaperSize::A5 => (148.0, 210.0),
            PaperSize::Letter => (215.9, 279.4),
            PaperSize::Legal => (215.9, 355.6),
            PaperSize::Tabloid => (279.4, 431.8),
            // Slides are screen-shaped, so their natural shape is wide.
            PaperSize::Slide16x9 => (338.7, 190.5),
            PaperSize::Slide4x3 => (254.0, 190.5),
            PaperSize::Custom { width_mm, height_mm } => (width_mm, height_mm),
        }
    }

    pub fn label(self) -> String {
        match self {
            PaperSize::A3 => "A3".into(),
            PaperSize::A4 => "A4".into(),
            PaperSize::A5 => "A5".into(),
            PaperSize::Letter => "US Letter".into(),
            PaperSize::Legal => "US Legal".into(),
            PaperSize::Tabloid => "Tabloid".into(),
            PaperSize::Slide16x9 => "Slide 16:9".into(),
            PaperSize::Slide4x3 => "Slide 4:3".into(),
            PaperSize::Custom { .. } => "Custom".into(),
        }
    }

    /// Paper sizes turn to match the screen when a page is created;
    /// slides and custom sizes keep the shape they were given.
    pub fn follows_orientation(self) -> bool {
        !matches!(self, PaperSize::Slide16x9 | PaperSize::Slide4x3 | PaperSize::Custom { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum PageSize {
    /// Grows right and down as you write.
    #[default]
    Infinite,
    /// Stacked sheets of paper; a blank one is always kept at the end.
    Sheets { paper: PaperSize, landscape: bool },
}

impl PageSize {
    /// Sheet width and height in page units, if sheets.
    pub fn sheet_px(self) -> Option<(f32, f32)> {
        match self {
            PageSize::Infinite => None,
            PageSize::Sheets { paper, landscape } => {
                let (w, h) = paper.mm();
                let (w, h) = (w * crate::ink::PX_PER_MM, h * crate::ink::PX_PER_MM);
                Some(if landscape { (h, w) } else { (w, h) })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    #[default]
    Blank,
    Lined,
    Grid,
    Dotted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageLayout {
    #[serde(default)]
    pub size: PageSize,
    #[serde(default)]
    pub background: Background,
}

/// Contents of a page file.
#[derive(Serialize, Deserialize)]
pub struct PageFile {
    pub version: u32,
    pub title: String,
    pub created: String,
    pub modified: String,
    #[serde(default)]
    pub layout: PageLayout,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<TextBox>,
    /// Fields from newer versions, preserved when rewriting the file.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PageFile {
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut page: PageFile = serde_json::from_slice(&fs::read(path)?)?;
        for s in &mut page.strokes {
            s.update_bounds();
        }
        Ok(page)
    }

    pub fn save(&mut self, path: &Path) -> io::Result<()> {
        self.modified = now_iso();
        write_json(path, self)
    }
}

#[derive(Debug)]
pub struct Section {
    pub dir: String,
    pub name: String,
    pub pattern: Pattern,
    /// Layout for new pages in this section.
    pub page_layout: PageLayout,
    pub pages: Vec<PageRef>,
}

#[derive(Debug)]
pub struct Notebook {
    pub dir: String,
    pub name: String,
    pub color: String,
    pub sections: Vec<Section>,
}

#[derive(Debug)]
pub struct Library {
    pub root: PathBuf,
    pub notebooks: Vec<Notebook>,
}

/// Something removed from the library but not yet trashed, so it can be
/// put back by "Undo".
pub enum Detached {
    Notebook { index: usize, item: Notebook },
    Section { nb: usize, index: usize, item: Section },
    Page { nb: usize, sec: usize, index: usize, item: PageRef },
}

fn now_iso() -> String {
    gtk::glib::DateTime::now_local()
        .and_then(|d| d.format_iso8601())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Write via a synced temp file and a rename, so a crash or power loss
/// leaves either the old file or the new one, never a truncated one.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    io::Write::write_all(&mut file, &serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Turn a display name into a portable file name (Linux, Windows, macOS).
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"/\:*?"<>|"#.contains(c) { '-' } else { c })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).trim_start_matches('.').to_string();
    if s.is_empty() {
        s = "Untitled".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.push('_');
    }
    s
}

/// `base` + `ext` in `dir`, adding " (2)", " (3)"… if taken. `keep` is a name
/// the caller already owns (renaming a file to itself).
fn unique_name(dir: &Path, base: &str, ext: Option<&str>, keep: Option<&str>) -> String {
    let make = |n: u32| {
        let stem = if n == 1 { base.to_string() } else { format!("{base} ({n})") };
        match ext {
            Some(e) => format!("{stem}.{e}"),
            None => stem,
        }
    };
    (1..)
        .map(make)
        .find(|name| Some(name.as_str()) == keep || !dir.join(name).exists())
        .expect("unbounded")
}

fn sorted_entries(dir: &Path, want_dirs: bool) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir() == want_dirs).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .filter(|n| want_dirs || n.ends_with(&format!(".{PAGE_EXT}")))
        .collect();
    names.sort();
    names
}

/// Keep `order` entries that exist on disk, then append unknown ones.
fn reconcile(order: Vec<String>, on_disk: &[String]) -> Vec<String> {
    let mut out: Vec<String> = order.into_iter().filter(|n| on_disk.contains(n)).collect();
    for n in on_disk {
        if !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}

fn default_pattern(section_name: &str) -> Pattern {
    let prefix = SECTION_PREFIXES
        .iter()
        .find(|(name, _)| *name == section_name)
        .map_or(section_name, |(_, prefix)| prefix);
    Pattern::numbered(prefix)
}

impl Library {
    pub fn load(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let meta: Option<LibraryMeta> = read_json(&root.join(LIBRARY_META));
        let order = reconcile(
            meta.map(|m| m.notebooks).unwrap_or_default(),
            &sorted_entries(root, true),
        );
        let notebooks = order
            .into_iter()
            .map(|dir| Self::load_notebook(&root.join(&dir), dir))
            .collect();
        let lib = Library { root: root.to_path_buf(), notebooks };
        lib.save_library_meta()?;
        Ok(lib)
    }

    fn load_notebook(path: &Path, dir: String) -> Notebook {
        let meta: Option<NotebookMeta> = read_json(&path.join(NOTEBOOK_META));
        let (name, color, order) = match meta {
            Some(m) => (m.name, m.color, m.sections),
            None => (dir.clone(), PALETTE[0].to_string(), Vec::new()),
        };
        let sections = reconcile(order, &sorted_entries(path, true))
            .into_iter()
            .map(|sdir| Self::load_section(&path.join(&sdir), sdir))
            .collect();
        Notebook { dir, name, color, sections }
    }

    fn load_section(path: &Path, dir: String) -> Section {
        let meta: Option<SectionMeta> = read_json(&path.join(SECTION_META));
        let (name, pattern, page_layout, refs) = match meta {
            Some(m) => (m.name, m.pattern, m.page_layout, m.pages),
            None => (dir.clone(), default_pattern(&dir), PageLayout::default(), Vec::new()),
        };
        let files = sorted_entries(path, false);
        let order = reconcile(refs.iter().map(|r| r.file.clone()).collect(), &files);
        let pages = order
            .into_iter()
            .map(|file| match refs.iter().find(|r| r.file == file) {
                Some(r) => r.clone(),
                None => {
                    let title = read_json::<PageFile>(&path.join(&file))
                        .map(|p| p.title)
                        .unwrap_or_else(|| {
                            file.trim_end_matches(&format!(".{PAGE_EXT}")).to_string()
                        });
                    PageRef { file, title }
                }
            })
            .collect();
        Section { dir, name, pattern, page_layout, pages }
    }

    // ---- paths -------------------------------------------------------------

    pub fn notebook_path(&self, nb: usize) -> PathBuf {
        self.root.join(&self.notebooks[nb].dir)
    }

    pub fn section_path(&self, nb: usize, sec: usize) -> PathBuf {
        self.notebook_path(nb).join(&self.notebooks[nb].sections[sec].dir)
    }

    pub fn page_path(&self, nb: usize, sec: usize, page: usize) -> PathBuf {
        self.section_path(nb, sec).join(&self.notebooks[nb].sections[sec].pages[page].file)
    }

    // ---- metadata ------------------------------------------------------------

    fn save_library_meta(&self) -> io::Result<()> {
        let meta = LibraryMeta {
            version: FORMAT_VERSION,
            notebooks: self.notebooks.iter().map(|n| n.dir.clone()).collect(),
        };
        write_json(&self.root.join(LIBRARY_META), &meta)
    }

    fn save_notebook_meta(&self, nb: usize) -> io::Result<()> {
        let n = &self.notebooks[nb];
        let meta = NotebookMeta {
            version: FORMAT_VERSION,
            name: n.name.clone(),
            color: n.color.clone(),
            sections: n.sections.iter().map(|s| s.dir.clone()).collect(),
        };
        write_json(&self.notebook_path(nb).join(NOTEBOOK_META), &meta)
    }

    fn save_section_meta(&self, nb: usize, sec: usize) -> io::Result<()> {
        let s = &self.notebooks[nb].sections[sec];
        let meta = SectionMeta {
            version: FORMAT_VERSION,
            name: s.name.clone(),
            pattern: s.pattern.clone(),
            page_layout: s.page_layout,
            pages: s.pages.clone(),
        };
        write_json(&self.section_path(nb, sec).join(SECTION_META), &meta)
    }

    // ---- create ----------------------------------------------------------------

    /// The palette color used by the fewest notebooks (first one on ties).
    fn next_color(&self) -> String {
        PALETTE
            .iter()
            .min_by_key(|c| self.notebooks.iter().filter(|n| n.color == **c).count())
            .unwrap()
            .to_string()
    }

    pub fn create_notebook(&mut self, name: &str) -> io::Result<usize> {
        let dir = unique_name(&self.root, &sanitize(name), None, None);
        fs::create_dir(self.root.join(&dir))?;
        let color = self.next_color();
        self.notebooks.push(Notebook { dir, name: name.to_string(), color, sections: Vec::new() });
        let nb = self.notebooks.len() - 1;
        self.save_notebook_meta(nb)?;
        self.save_library_meta()?;
        for section in PRESET_SECTIONS {
            self.create_section(nb, section)?;
        }
        Ok(nb)
    }

    pub fn create_section(&mut self, nb: usize, name: &str) -> io::Result<usize> {
        let nb_path = self.notebook_path(nb);
        let dir = unique_name(&nb_path, &sanitize(name), None, None);
        fs::create_dir(nb_path.join(&dir))?;
        let section = Section {
            dir,
            name: name.to_string(),
            pattern: default_pattern(name),
            page_layout: PageLayout::default(),
            pages: Vec::new(),
        };
        self.notebooks[nb].sections.push(section);
        let sec = self.notebooks[nb].sections.len() - 1;
        self.save_section_meta(nb, sec)?;
        self.save_notebook_meta(nb)?;
        Ok(sec)
    }

    /// Add a page using the section's default layout. `landscape` is the
    /// screen orientation, which paper sizes follow.
    pub fn create_page(&mut self, nb: usize, sec: usize, landscape: bool) -> io::Result<usize> {
        let section = &self.notebooks[nb].sections[sec];
        let title =
            section.pattern.next_title(section.pages.iter().map(|p| p.title.as_str()), Today::now());
        let sec_path = self.section_path(nb, sec);
        let file = unique_name(&sec_path, &sanitize(&title), Some(PAGE_EXT), None);
        let now = now_iso();
        let page = PageFile {
            version: FORMAT_VERSION,
            title: title.clone(),
            created: now.clone(),
            modified: now,
            layout: {
                let mut layout = self.notebooks[nb].sections[sec].page_layout;
                if let PageSize::Sheets { paper, .. } = layout.size
                    && paper.follows_orientation()
                {
                    layout.size = PageSize::Sheets { paper, landscape };
                }
                layout
            },
            strokes: Vec::new(),
            texts: Vec::new(),
            extra: Default::default(),
        };
        write_json(&sec_path.join(&file), &page)?;
        self.notebooks[nb].sections[sec].pages.push(PageRef { file, title });
        self.save_section_meta(nb, sec)?;
        Ok(self.notebooks[nb].sections[sec].pages.len() - 1)
    }

    pub fn set_section_layout(&mut self, nb: usize, sec: usize, layout: PageLayout) -> io::Result<()> {
        self.notebooks[nb].sections[sec].page_layout = layout;
        self.save_section_meta(nb, sec)
    }

    // ---- rename ----------------------------------------------------------------

    pub fn rename_notebook(&mut self, nb: usize, name: &str) -> io::Result<()> {
        let old = self.notebooks[nb].dir.clone();
        let dir = unique_name(&self.root, &sanitize(name), None, Some(&old));
        if dir != old {
            fs::rename(self.root.join(&old), self.root.join(&dir))?;
        }
        let n = &mut self.notebooks[nb];
        n.dir = dir;
        n.name = name.to_string();
        self.save_notebook_meta(nb)?;
        self.save_library_meta()
    }

    pub fn rename_section(&mut self, nb: usize, sec: usize, name: &str) -> io::Result<()> {
        let nb_path = self.notebook_path(nb);
        let old = self.notebooks[nb].sections[sec].dir.clone();
        let dir = unique_name(&nb_path, &sanitize(name), None, Some(&old));
        if dir != old {
            fs::rename(nb_path.join(&old), nb_path.join(&dir))?;
        }
        let s = &mut self.notebooks[nb].sections[sec];
        // Until a pattern has been learned from a page rename, page names
        // follow the section name ("New Section" -> "Exercises 1").
        if s.pattern == default_pattern(&s.name) {
            s.pattern = default_pattern(name);
        }
        s.dir = dir;
        s.name = name.to_string();
        self.save_section_meta(nb, sec)?;
        self.save_notebook_meta(nb)
    }

    /// Rename a page; a title with a number and/or date also becomes the
    /// section's naming pattern for new pages.
    pub fn rename_page(&mut self, nb: usize, sec: usize, page: usize, title: &str) -> io::Result<()> {
        let sec_path = self.section_path(nb, sec);
        let old = self.notebooks[nb].sections[sec].pages[page].file.clone();
        let file = unique_name(&sec_path, &sanitize(title), Some(PAGE_EXT), Some(&old));
        if file != old {
            fs::rename(sec_path.join(&old), sec_path.join(&file))?;
        }
        let path = sec_path.join(&file);
        if let Some(mut data) = read_json::<PageFile>(&path) {
            data.title = title.to_string();
            data.modified = now_iso();
            write_json(&path, &data)?;
        }
        let section = &mut self.notebooks[nb].sections[sec];
        section.pages[page] = PageRef { file, title: title.to_string() };
        if let Some(pattern) = Pattern::learn(title) {
            section.pattern = pattern;
        }
        self.save_section_meta(nb, sec)
    }

    // ---- delete (two-step, for undo) -------------------------------------------

    /// Remove an item from the library without touching its files.
    /// Returns its path so the caller can trash it once undo has expired.
    pub fn detach(&mut self, target: Target) -> io::Result<(Detached, PathBuf)> {
        Ok(match target {
            Target::Notebook(nb) => {
                let path = self.notebook_path(nb);
                let item = self.notebooks.remove(nb);
                self.save_library_meta()?;
                (Detached::Notebook { index: nb, item }, path)
            }
            Target::Section(nb, sec) => {
                let path = self.section_path(nb, sec);
                let item = self.notebooks[nb].sections.remove(sec);
                self.save_notebook_meta(nb)?;
                (Detached::Section { nb, index: sec, item }, path)
            }
            Target::Page(nb, sec, page) => {
                let path = self.page_path(nb, sec, page);
                let item = self.notebooks[nb].sections[sec].pages.remove(page);
                self.save_section_meta(nb, sec)?;
                (Detached::Page { nb, sec, index: page, item }, path)
            }
        })
    }

    /// Undo a [`Library::detach`]. Assumes the library wasn't restructured in
    /// between (the UI only allows undo of the most recent delete).
    pub fn reattach(&mut self, d: Detached) -> io::Result<()> {
        match d {
            Detached::Notebook { index, item } => {
                let index = index.min(self.notebooks.len());
                self.notebooks.insert(index, item);
                self.save_library_meta()
            }
            Detached::Section { nb, index, item } => {
                let sections = &mut self.notebooks[nb].sections;
                sections.insert(index.min(sections.len()), item);
                self.save_notebook_meta(nb)
            }
            Detached::Page { nb, sec, index, item } => {
                let pages = &mut self.notebooks[nb].sections[sec].pages;
                pages.insert(index.min(pages.len()), item);
                self.save_section_meta(nb, sec)
            }
        }
    }

    // ---- move ------------------------------------------------------------------

    pub fn move_notebook(&mut self, from: usize, to: usize) -> io::Result<()> {
        let item = self.notebooks.remove(from);
        let to = if to > from { to - 1 } else { to };
        self.notebooks.insert(to.min(self.notebooks.len()), item);
        self.save_library_meta()
    }

    /// Move section `(nb, sec)` to position `to` in notebook `to_nb`.
    /// Returns its new `(nb, sec)`.
    pub fn move_section(
        &mut self,
        (nb, sec): (usize, usize),
        to_nb: usize,
        to: usize,
    ) -> io::Result<(usize, usize)> {
        let to = if to_nb == nb && to > sec { to - 1 } else { to };
        if to_nb != nb {
            let old_path = self.section_path(nb, sec);
            let target_dir = self.notebook_path(to_nb);
            let dir = unique_name(&target_dir, &self.notebooks[nb].sections[sec].dir, None, None);
            fs::rename(&old_path, target_dir.join(&dir))?;
            self.notebooks[nb].sections[sec].dir = dir;
        }
        let item = self.notebooks[nb].sections.remove(sec);
        let sections = &mut self.notebooks[to_nb].sections;
        let to = to.min(sections.len());
        sections.insert(to, item);
        self.save_notebook_meta(nb)?;
        if to_nb != nb {
            self.save_notebook_meta(to_nb)?;
        }
        Ok((to_nb, to))
    }

    /// Move page `(nb, sec, page)` to position `to` in section `(to_nb, to_sec)`.
    /// Returns its new `(nb, sec, page)`.
    pub fn move_page(
        &mut self,
        (nb, sec, page): (usize, usize, usize),
        (to_nb, to_sec): (usize, usize),
        to: usize,
    ) -> io::Result<(usize, usize, usize)> {
        let same = (nb, sec) == (to_nb, to_sec);
        let to = if same && to > page { to - 1 } else { to };
        if !same {
            let old_path = self.page_path(nb, sec, page);
            let target_dir = self.section_path(to_nb, to_sec);
            let stem = self.notebooks[nb].sections[sec].pages[page]
                .file
                .trim_end_matches(&format!(".{PAGE_EXT}"))
                .to_string();
            let file = unique_name(&target_dir, &stem, Some(PAGE_EXT), None);
            fs::rename(&old_path, target_dir.join(&file))?;
            self.notebooks[nb].sections[sec].pages[page].file = file;
        }
        let item = self.notebooks[nb].sections[sec].pages.remove(page);
        let pages = &mut self.notebooks[to_nb].sections[to_sec].pages;
        let to = to.min(pages.len());
        pages.insert(to, item);
        self.save_section_meta(nb, sec)?;
        if !same {
            self.save_section_meta(to_nb, to_sec)?;
        }
        Ok((to_nb, to_sec, to))
    }
}

/// Addresses an item in the library by indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Notebook(usize),
    Section(usize, usize),
    Page(usize, usize, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ratchet-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn new_pages_follow_orientation() {
        let root = temp_root("orient");
        let mut lib = Library::load(&root).unwrap();
        let nb = lib.create_notebook("N").unwrap();
        let a4 = PageLayout { size: PageSize::Sheets { paper: PaperSize::A4, landscape: false }, ..Default::default() };
        lib.set_section_layout(nb, 0, a4).unwrap();
        let p = lib.create_page(nb, 0, true).unwrap();
        let page = PageFile::load(&lib.page_path(nb, 0, p)).unwrap();
        assert_eq!(page.layout.size, PageSize::Sheets { paper: PaperSize::A4, landscape: true });
        // Custom sizes keep their shape; layouts round-trip through JSON.
        let custom = PageSize::Sheets { paper: PaperSize::Custom { width_mm: 100.0, height_mm: 50.0 }, landscape: false };
        lib.set_section_layout(nb, 0, PageLayout { size: custom, ..Default::default() }).unwrap();
        let p = lib.create_page(nb, 0, false).unwrap();
        assert_eq!(PageFile::load(&lib.page_path(nb, 0, p)).unwrap().layout.size, custom);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("Calc: II / Analysis?"), "Calc- II - Analysis-");
        assert_eq!(sanitize("  ..hidden.  "), "hidden");
        assert_eq!(sanitize("con"), "con_");
        assert_eq!(sanitize(""), "Untitled");
    }

    #[test]
    fn create_rename_move_reload() {
        let root = temp_root("crud");
        let mut lib = Library::load(&root).unwrap();
        let calc = lib.create_notebook("Calculus II").unwrap();
        let phys = lib.create_notebook("Physics I").unwrap();
        assert_ne!(lib.notebooks[calc].color, lib.notebooks[phys].color);
        assert_eq!(lib.notebooks[calc].sections.len(), 2);
        lib.create_section(calc, "Laboratory").unwrap();
        lib.create_section(phys, "Laboratory").unwrap();

        lib.create_page(calc, 0, false).unwrap();
        lib.create_page(calc, 0, false).unwrap();
        let titles: Vec<_> = lib.notebooks[calc].sections[0].pages.iter().map(|p| &p.title).collect();
        assert_eq!(titles, ["Lecture 1", "Lecture 2"]);
        lib.create_page(calc, 2, false).unwrap();
        assert_eq!(lib.notebooks[calc].sections[2].pages[0].title, "Lab 1");

        lib.rename_page(calc, 0, 1, "Week 9: limits").unwrap();
        lib.create_page(calc, 0, false).unwrap();
        assert_eq!(lib.notebooks[calc].sections[0].pages[2].title, "Week 10: limits");
        assert!(lib.page_path(calc, 0, 1).ends_with("Week 9- limits.json"));

        // Reorder within a section, then move a page to another notebook.
        lib.move_page((calc, 0, 2), (calc, 0), 0).unwrap();
        assert_eq!(lib.notebooks[calc].sections[0].pages[0].title, "Week 10: limits");
        lib.move_page((calc, 0, 0), (phys, 1), 0).unwrap();
        assert!(lib.page_path(phys, 1, 0).exists());
        lib.move_section((calc, 2), phys, 0).unwrap();
        assert_eq!(lib.notebooks[phys].sections[0].name, "Laboratory");
        lib.move_notebook(phys, 0).unwrap();

        let reloaded = Library::load(&root).unwrap();
        assert_eq!(reloaded.notebooks[0].name, "Physics I");
        assert_eq!(reloaded.notebooks[0].sections.len(), 4);
        assert_eq!(reloaded.notebooks[0].sections[2].pages[0].title, "Week 10: limits");
        assert_eq!(reloaded.notebooks[1].sections[0].pages.len(), 2);

        // Detach + reattach round-trips; unknown files get adopted on load.
        let (d, path) = lib.detach(Target::Notebook(1)).unwrap();
        assert!(path.exists());
        lib.reattach(d).unwrap();
        fs::write(lib.section_path(0, 1).join("Synced.json"), b"{}").unwrap();
        let reloaded = Library::load(&root).unwrap();
        assert_eq!(reloaded.notebooks.len(), 2);
        assert_eq!(reloaded.notebooks[0].sections[1].pages[0].title, "Synced");

        fs::remove_dir_all(root).unwrap();
    }
}
