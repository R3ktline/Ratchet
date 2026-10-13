//! The page canvas: renders ink with GSK and turns pen/mouse input into
//! strokes. Owns the open page's strokes, layout, view (zoom/scroll), undo
//! history and autosave.
//!
//! Page coordinates are page units (96 per inch at 100% zoom). The view maps
//! them to the widget: `screen = (page - scroll) * zoom`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use crate::ink::{self, InkColor, Point, Smoother, Stroke, Strokes, Tool};
use crate::library::{Background, PageFile, PageLayout, PageSize};
use crate::shapes::{self, ShapeKind};
use crate::text::{self, Run, Style, TextBox, TextSize};

/// Text boxes on a page, shared cheaply between undo snapshots.
pub type Texts = Vec<Rc<TextBox>>;

/// Everything undo restores.
type Snap = (Strokes, Texts);

/// Text formatting commands from the toolbar.
#[derive(Clone, Debug)]
pub enum TextCommand {
    Size(TextSize),
    Font(Option<String>),
    Bold,
    Italic,
    Underline,
}

/// An open text editor on the canvas.
struct Editor {
    view: gtk::TextView,
    /// The box being edited (its position, size, font, color).
    draft: TextBox,
    /// The saved box this edit replaces, if any.
    original: Option<Rc<TextBox>>,
    /// Style for newly typed text.
    pending: Rc<Cell<Style>>,
}
use crate::settings::{ButtonAction, EraserMode, RadialItem};

/// Eraser radii in screen pixels (the same size at any zoom).
const STROKE_ERASER_RADIUS: f32 = 4.0;
const PARTIAL_ERASER_RADIUS: f32 = 8.0;
const HIGHLIGHTER_ALPHA: f32 = 0.4;
const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 4.0;
/// Space around and between sheets, in page units.
const SHEET_GAP: f32 = 24.0;
/// Pixels scrolled per mouse wheel notch.
const WHEEL_STEP: f32 = 48.0;

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub tool: Tool,
    pub color: InkColor,
    /// Width in page units.
    pub width: f32,
}

/// What the main tool (pen tip / left mouse button) does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasTool {
    Draw,
    Erase,
    Select,
    Text,
    Hand,
}

/// What a single pointer-down gesture is doing. Also reported while a
/// mapped pen input temporarily overrides the selected tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Draw,
    Erase,
    Select,
    Text,
    /// Picking from the radial menu.
    Menu,
    Pan,
    Ignore,
}

/// How a radial menu entry looks.
#[derive(Clone, Debug)]
pub enum EntryLook {
    Pen(InkColor),
    Highlighter(InkColor),
    Eraser,
    Select,
    Hand,
    Icon(&'static str),
}

#[derive(Clone, Debug)]
pub struct RadialEntry {
    pub item: RadialItem,
    pub look: EntryLook,
}

struct Radial {
    center: (f32, f32),
    hover: Option<usize>,
}

struct LongPress {
    start: (f32, f32),
    began: Instant,
    timer: Option<glib::SourceId>,
    tick: Option<gtk::TickCallbackId>,
}

const RADIAL_ITEM: f32 = 23.0;

/// Ring geometry for `n` items: (outer radius, radius of the item circle).
/// The ring grows when items wouldn't fit around the default size.
fn radial_geometry(n: usize) -> (f32, f32) {
    let spacing = 2.0 * RADIAL_ITEM + 8.0;
    let items = (72.0f32).max(n as f32 * spacing / std::f32::consts::TAU);
    (items + RADIAL_ITEM + 13.0, items)
}
const RADIAL_CENTER: f32 = 28.0;
/// Pointer movement (screen px) that cancels a long press.
const LONG_PRESS_SLOP: f32 = 6.0;
/// The long-press ring only shows after this, so quick strokes don't flash it.
const LONG_PRESS_RING_DELAY: Duration = Duration::from_millis(250);

/// Pen-pan movement (screen px) that breaks the vertical rail.
const RAIL_BREAK: f32 = 113.0;
/// Handle size for resizing a selection, in screen px.
const HANDLE_RADIUS: f32 = 6.0;
const ZOOM_ANIMATION_MS: u32 = 160;
const FADE_MS: u32 = 160;

#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    zoom: f32,
    /// Page coordinates of the widget's top-left corner.
    sx: f32,
    sy: f32,
}

impl Default for View {
    fn default() -> Self {
        View { zoom: 1.0, sx: 0.0, sy: 0.0 }
    }
}

#[derive(Default)]
struct History {
    undo: Vec<Snap>,
    redo: Vec<Snap>,
}

struct Live {
    stroke: Stroke,
    smoother: Smoother,
    last_raw: Point,
    raw: Vec<Point>,
    /// Set once the stroke snapped to a shape (pen held still at the end).
    shape: Option<ShapeKind>,
    /// Screen position where the pen last started holding still.
    hold_from: (f32, f32),
}

/// Pen movement (screen px) still counted as "holding still" for shapes;
/// generous, since hands tremble.
const SHAPE_HOLD_SLOP: f32 = 9.0;

struct Erasing {
    before: Strokes,
    last: Option<(f32, f32)>,
    changed: bool,
}

struct Panning {
    start: (f32, f32),
    view: View,
    /// Pen pans are locked vertical until clearly moved sideways.
    rail: bool,
    decided: bool,
}

/// An in-progress selection gesture.
enum SelOp {
    Lasso { points: Vec<(f32, f32)>, add: bool },
    Move { start: (f32, f32), delta: (f32, f32) },
    /// Scale around `anchor` (the opposite corner).
    Resize { anchor: (f32, f32), corner: (f32, f32), scale: (f32, f32), free: bool },
}

impl SelOp {
    /// Map a page point through the live move/resize transform.
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        match self {
            SelOp::Move { delta, .. } => (x + delta.0, y + delta.1),
            SelOp::Resize { anchor, scale, .. } => {
                (anchor.0 + (x - anchor.0) * scale.0, anchor.1 + (y - anchor.1) * scale.1)
            }
            SelOp::Lasso { .. } => (x, y),
        }
    }

    fn width_factor(&self) -> f32 {
        match self {
            SelOp::Resize { scale, .. } => (scale.0.abs() * scale.1.abs()).sqrt(),
            _ => 1.0,
        }
    }

    fn transforms(&self) -> bool {
        !matches!(self, SelOp::Lasso { .. })
    }
}

fn transformed(stroke: &Stroke, op: &SelOp) -> Stroke {
    let mut s = stroke.clone();
    for p in &mut s.points {
        (p.x, p.y) = op.apply(p.x, p.y);
    }
    s.width *= op.width_factor();
    s.update_bounds();
    s
}

/// Move/resize a text box: its corner follows the transform, a fixed wrap
/// width scales; the font size stays.
fn transformed_text(t: &TextBox, op: &SelOp, ctx: &gtk::pango::Context) -> TextBox {
    let mut n = t.clone();
    (n.x, n.y) = op.apply(t.x, t.y);
    if let (Some(w), SelOp::Resize { scale, .. }) = (t.width, op) {
        n.width = Some((w * scale.0.abs()).max(20.0));
    }
    n.measure(ctx);
    n
}

fn text_bounds(t: &TextBox) -> ink::Bounds {
    t.bounds()
}

fn merge_bounds(a: ink::Bounds, b: ink::Bounds) -> ink::Bounds {
    ink::Bounds { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }
}

fn union_bounds<'a>(strokes: impl IntoIterator<Item = &'a Rc<Stroke>>) -> Option<ink::Bounds> {
    strokes.into_iter().map(|s| s.bounds).reduce(merge_bounds)
}

/// Add `picked` to `sel` (replacing it unless `add`); with `add`, tapping
/// something already selected unselects it.
fn merge_selection<T>(sel: &mut Vec<Rc<T>>, picked: Vec<Rc<T>>, add: bool, tap: bool) {
    if !add {
        sel.clear();
    }
    for item in picked {
        match sel.iter().position(|x| Rc::ptr_eq(x, &item)) {
            Some(i) if add && tap => {
                sel.remove(i);
            }
            Some(_) => {}
            None => sel.push(item),
        }
    }
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            c = !c;
        }
    }
    c
}

pub struct Config {
    pub tool: CanvasTool,
    /// Pen used for drawing (also when a pen input is mapped to Draw).
    pub brush: Brush,
    pub eraser_mode: EraserMode,
    /// Stabilizer string length in mm of screen movement.
    pub stabilizer_mm: f32,
    /// Curve-fitting passes on pen-up.
    pub smooth_passes: usize,
    pub button_2: ButtonAction,
    pub button_3: ButtonAction,
    pub eraser_end: ButtonAction,
    pub text_size: TextSize,
    pub text_font: Option<String>,
    pub radial: Vec<RadialEntry>,
    pub long_press: Option<Duration>,
    /// Holding the pen still this long at the end of a stroke snaps it to
    /// a shape (None = off).
    pub shape_hold: Option<Duration>,
}

/// Debug tools, only reachable with `RATCHET_DEBUG=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebugFlags {
    pub input: bool,
    pub points: bool,
    pub perf: bool,
    pub log: bool,
}

#[derive(Default)]
struct DebugState {
    flags: DebugFlags,
    tool: String,
    pressure: f64,
    tilt: (Option<f64>, Option<f64>),
    buttons: String,
    last_button: Option<u32>,
    action: String,
    samples: u32,
    samples_per_sec: u32,
    backlog: usize,
    frames: u32,
    fps: u32,
    frame_us: u128,
    cache_us: u128,
    save_ms: f64,
    second_start: Option<Instant>,
    /// Raw input of strokes drawn this session, keyed by stroke address.
    raw: Vec<(*const Stroke, Vec<Point>)>,
}

type HistoryFn = Box<dyn Fn(bool, bool)>;
type ErrorFn = Box<dyn Fn(String)>;
type ZoomFn = Box<dyn Fn(f32)>;
type LogFn = Box<dyn Fn(String)>;
type TempToolFn = Box<dyn Fn(Option<Action>)>;
type MenuFn = Box<dyn Fn(RadialItem)>;

mod imp {
    use super::*;

    pub struct Canvas {
        pub(super) path: RefCell<Option<PathBuf>>,
        pub(super) layout: Cell<PageLayout>,
        pub(super) strokes: RefCell<Strokes>,
        pub(super) texts: RefCell<Texts>,
        pub(super) sel_texts: RefCell<Texts>,
        pub(super) clipboard_texts: RefCell<Vec<TextBox>>,
        pub(super) editor: RefCell<Option<Editor>>,
        pub(super) editor_css: gtk::CssProvider,
        /// Text tool press position (screen), to tell a click from a drag.
        pub(super) text_press: Cell<Option<(f32, f32)>>,
        /// Right/bottom edge of all ink, in page units.
        pub(super) extent: Cell<(f32, f32)>,
        pub(super) history: RefCell<History>,
        pub(super) view: Cell<View>,
        /// Undo history and view of pages opened earlier in this session.
        pub(super) stashed: RefCell<HashMap<PathBuf, (History, View)>>,
        pub(super) dirty: Cell<bool>,
        pub(super) save_timer: RefCell<Option<glib::SourceId>>,
        pub(super) live: RefCell<Option<Live>>,
        pub(super) erasing: RefCell<Option<Erasing>>,
        pub(super) panning: RefCell<Option<Panning>>,
        pub(super) eraser_at: Cell<Option<(f32, f32)>>,
        pub(super) pointer: Cell<(f32, f32)>,
        pub(super) action: Cell<Action>,
        /// The current gesture's action came from a held pen button.
        pub(super) from_button: Cell<bool>,
        /// Committed strokes rendered once (in page coordinates); rebuilt
        /// when they or the theme change.
        /// Keyed by (dark theme, selection hidden for a live move/resize).
        pub(super) cache: RefCell<Option<(bool, bool, Option<gsk::RenderNode>)>>,
        pub(super) config: RefCell<Config>,
        pub(super) debug: RefCell<DebugState>,
        pub(super) on_history: RefCell<Option<HistoryFn>>,
        pub(super) on_error: RefCell<Option<ErrorFn>>,
        pub(super) on_zoom: RefCell<Option<ZoomFn>>,
        pub(super) on_log: RefCell<Option<LogFn>>,
        pub(super) on_temp_tool: RefCell<Option<TempToolFn>>,
        pub(super) on_menu: RefCell<Option<MenuFn>>,
        pub(super) radial: RefCell<Option<Radial>>,
        pub(super) radial_open: Cell<f64>,
        pub(super) radial_anim: RefCell<Option<adw::Animation>>,
        pub(super) long_press: RefCell<Option<LongPress>>,
        pub(super) shape_timer: RefCell<Option<glib::SourceId>>,
        pub(super) selection: RefCell<Strokes>,
        pub(super) sel_op: RefCell<Option<SelOp>>,
        pub(super) clipboard: RefCell<Vec<Stroke>>,
        /// Modifier keys at the start of the current gesture.
        pub(super) mods: Cell<gdk::ModifierType>,
        /// Pen side buttons (BUTTON2/3 masks) the raw event handler has seen held.
        pub(super) pen_buttons: Cell<gdk::ModifierType>,
        pub(super) zoom_anim: RefCell<Option<(adw::TimedAnimation, f32)>>,
        pub(super) fade: Cell<f64>,
        pub(super) fade_anim: RefCell<Option<adw::Animation>>,
    }

    impl Default for Canvas {
        fn default() -> Self {
            Canvas {
                path: RefCell::default(),
                layout: Cell::default(),
                strokes: RefCell::default(),
                texts: RefCell::default(),
                sel_texts: RefCell::default(),
                clipboard_texts: RefCell::default(),
                editor: RefCell::default(),
                editor_css: gtk::CssProvider::new(),
                text_press: Cell::new(None),
                extent: Cell::new((0.0, 0.0)),
                history: RefCell::default(),
                view: Cell::default(),
                stashed: RefCell::default(),
                dirty: Cell::new(false),
                save_timer: RefCell::default(),
                live: RefCell::default(),
                erasing: RefCell::default(),
                panning: RefCell::default(),
                eraser_at: Cell::new(None),
                pointer: Cell::new((0.0, 0.0)),
                action: Cell::new(Action::Ignore),
                from_button: Cell::new(false),
                cache: RefCell::default(),
                config: RefCell::new(Config {
                    tool: CanvasTool::Draw,
                    brush: Brush { tool: Tool::Pen, color: InkColor::Ink, width: 0.7 * ink::PX_PER_MM },
                    eraser_mode: EraserMode::Stroke,
                    stabilizer_mm: 1.0,
                    smooth_passes: 2,
                    button_2: ButtonAction::Pan,
                    button_3: ButtonAction::Click,
                    eraser_end: ButtonAction::Eraser,
                    text_size: TextSize::default(),
                    text_font: None,
                    radial: Vec::new(),
                    long_press: Some(Duration::from_millis(720)),
                    shape_hold: Some(Duration::from_millis(500)),
                }),
                debug: RefCell::default(),
                on_history: RefCell::default(),
                on_error: RefCell::default(),
                on_zoom: RefCell::default(),
                on_log: RefCell::default(),
                on_temp_tool: RefCell::default(),
                on_menu: RefCell::default(),
                radial: RefCell::default(),
                radial_open: Cell::new(0.0),
                radial_anim: RefCell::default(),
                long_press: RefCell::default(),
                shape_timer: RefCell::default(),
                selection: RefCell::default(),
                sel_op: RefCell::default(),
                clipboard: RefCell::default(),
                mods: Cell::new(gdk::ModifierType::empty()),
                pen_buttons: Cell::new(gdk::ModifierType::empty()),
                zoom_anim: RefCell::default(),
                fade: Cell::new(1.0),
                fade_anim: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "RatchetCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {
        fn dispose(&self) {
            if let Some(e) = self.editor.take() {
                e.view.unparent();
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_focusable(true);
            obj.set_cursor_from_name(Some("crosshair"));
            obj.setup_input();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &self.editor_css,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }

            let weak = obj.downgrade();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(canvas) = weak.upgrade() {
                    canvas.invalidate();
                }
            });
        }
    }

    impl WidgetImpl for Canvas {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            // Keep the view valid (e.g. sheets centered) when resized.
            let obj = self.obj();
            obj.set_view(self.view.get());
            obj.allocate_editor();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let started = Instant::now();
            self.obj().draw(snapshot);
            if let Some(e) = self.editor.borrow().as_ref() {
                self.obj().snapshot_child(&e.view, snapshot);
            }
            let mut d = self.debug.borrow_mut();
            if d.flags.perf {
                d.frames += 1;
                d.frame_us = started.elapsed().as_micros();
            }
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The held-button mask of a pen side button.
fn button_mask(button: u32) -> Option<gdk::ModifierType> {
    match button {
        2 => Some(gdk::ModifierType::BUTTON2_MASK),
        3 => Some(gdk::ModifierType::BUTTON3_MASK),
        _ => None,
    }
}

pub(crate) fn rgba((r, g, b): (f32, f32, f32), a: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, a)
}

pub(crate) fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

pub(crate) fn draw_stroke(snapshot: &gtk::Snapshot, stroke: &Stroke, dark: bool) {
    let outline = stroke.outline();
    let n = outline.len();
    if n < 3 {
        return;
    }
    // Smooth the polygon: quadratic curves through edge midpoints.
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let builder = gsk::PathBuilder::new();
    let start = mid(outline[0], outline[1]);
    builder.move_to(start.0, start.1);
    for i in 1..=n {
        let p = outline[i % n];
        let m = mid(p, outline[(i + 1) % n]);
        builder.quad_to(p.0, p.1, m.0, m.1);
    }
    builder.close();
    let alpha = match stroke.tool {
        Tool::Pen => 1.0,
        Tool::Highlighter => HIGHLIGHTER_ALPHA,
    };
    snapshot.append_fill(&builder.to_path(), gsk::FillRule::Winding, &rgba(stroke.color.resolve(dark), alpha));
}

/// Fill `area` (page units) with the background pattern, tiled from the
/// coordinate origin. `zoom` keeps lines one screen pixel thin.
fn draw_pattern(snapshot: &gtk::Snapshot, bg: Background, area: graphene::Rect, zoom: f32, dark: bool) {
    let spacing = match bg {
        Background::Blank => return,
        Background::Lined => 8.0 * ink::PX_PER_MM,
        Background::Grid | Background::Dotted => 5.0 * ink::PX_PER_MM,
    };
    // Too dense to be useful when zoomed far out.
    if spacing * zoom < 5.0 {
        return;
    }
    let color = if dark { rgba((1.0, 1.0, 1.0), 0.13) } else { rgba((0.0, 0.0, 0.0), 0.13) };
    let line = 1.0 / zoom;
    match bg {
        Background::Lined => {
            snapshot.push_repeat(&area, Some(&rect(0.0, 0.0, 1.0, spacing)));
            snapshot.append_color(&color, &rect(0.0, 0.0, 1.0, line));
        }
        Background::Grid => {
            snapshot.push_repeat(&area, Some(&rect(0.0, 0.0, spacing, spacing)));
            snapshot.append_color(&color, &rect(0.0, 0.0, spacing, line));
            snapshot.append_color(&color, &rect(0.0, 0.0, line, spacing));
        }
        Background::Dotted => {
            snapshot.push_repeat(&area, Some(&rect(0.0, 0.0, spacing, spacing)));
            let d = 2.0 / zoom;
            let dot = gsk::RoundedRect::from_rect(rect(0.0, 0.0, d, d), d / 2.0);
            snapshot.push_rounded_clip(&dot);
            let mut c = color;
            c.set_alpha(0.35);
            snapshot.append_color(&c, &rect(0.0, 0.0, d, d));
            snapshot.pop();
        }
        Background::Blank => return,
    }
    snapshot.pop();
}

impl Default for Canvas {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- rendering -------------------------------------------------------------

    fn page_color(dark: bool) -> (f32, f32, f32) {
        if dark { (0.12, 0.12, 0.13) } else { (1.0, 1.0, 1.0) }
    }

    /// Sheet rectangles (page units) for a sheets layout.
    fn sheets(&self) -> Vec<graphene::Rect> {
        let imp = self.imp();
        let Some((w, h)) = imp.layout.get().size.sheet_px() else { return Vec::new() };
        let (_, bottom) = imp.extent.get();
        let pitch = h + SHEET_GAP;
        // Always one blank sheet after the last one with ink.
        let empty = imp.strokes.borrow().is_empty() && imp.texts.borrow().is_empty();
        let count = if empty { 1 } else { (bottom / pitch).floor().max(0.0) as usize + 2 };
        (0..count).map(|i| rect(0.0, i as f32 * pitch, w, h)).collect()
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let fade = self.imp().fade.get();
        if fade < 1.0 {
            snapshot.push_opacity(fade);
        }
        self.draw_page(snapshot);
        if fade < 1.0 {
            snapshot.pop();
        }
    }

    fn draw_page(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let dark = adw::StyleManager::default().is_dark();
        let (w, h) = (self.width() as f32, self.height() as f32);
        let view = imp.view.get();
        let layout = imp.layout.get();
        let page = rgba(Self::page_color(dark), 1.0);

        let sheets = self.sheets();
        if sheets.is_empty() {
            snapshot.append_color(&page, &rect(0.0, 0.0, w, h));
        } else {
            let gutter = if dark { (0.2, 0.2, 0.21) } else { (0.87, 0.87, 0.88) };
            snapshot.append_color(&rgba(gutter, 1.0), &rect(0.0, 0.0, w, h));
        }

        snapshot.save();
        snapshot.scale(view.zoom, view.zoom);
        snapshot.translate(&graphene::Point::new(-view.sx, -view.sy));
        let visible = rect(view.sx, view.sy, w / view.zoom, h / view.zoom);

        if sheets.is_empty() {
            // Infinite pages start at the origin and grow right/down.
            let area = rect(view.sx.max(0.0), view.sy.max(0.0), visible.width(), visible.height());
            draw_pattern(snapshot, layout.background, area, view.zoom, dark);
        } else {
            for sheet in sheets.iter().filter(|s| s.intersection(&visible).is_some()) {
                let shadow = gsk::RoundedRect::from_rect(*sheet, 0.0);
                snapshot.append_outset_shadow(&shadow, &rgba((0.0, 0.0, 0.0), 0.25), 0.0, 1.0, 0.0, 4.0);
                snapshot.append_color(&page, sheet);
                if let Some(area) = sheet.intersection(&visible) {
                    // Tile from the sheet's corner, not the page origin.
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(sheet.x(), sheet.y()));
                    let local = rect(area.x() - sheet.x(), area.y() - sheet.y(), area.width(), area.height());
                    draw_pattern(snapshot, layout.background, local, view.zoom, dark);
                    snapshot.restore();
                }
            }
        }

        let moving = imp.sel_op.borrow().as_ref().is_some_and(SelOp::transforms);
        {
            let mut cache = imp.cache.borrow_mut();
            if cache.as_ref().is_none_or(|(d, h, _)| (*d, *h) != (dark, moving)) {
                let started = Instant::now();
                let s = gtk::Snapshot::new();
                let strokes = imp.strokes.borrow();
                let selection = imp.selection.borrow();
                let hidden = |st: &Rc<Stroke>| moving && selection.iter().any(|x| Rc::ptr_eq(x, st));
                // Highlighter goes under the ink.
                for tool in [Tool::Highlighter, Tool::Pen] {
                    for stroke in strokes.iter().filter(|s| s.tool == tool && !hidden(s)) {
                        draw_stroke(&s, stroke, dark);
                    }
                }
                let editing = imp.editor.borrow().as_ref().and_then(|e| e.original.clone());
                let sel_texts = imp.sel_texts.borrow();
                let ctx = self.pango_context();
                for t in imp.texts.borrow().iter() {
                    let skip = editing.as_ref().is_some_and(|o| Rc::ptr_eq(o, t))
                        || (moving && sel_texts.iter().any(|x| Rc::ptr_eq(x, t)));
                    if !skip {
                        let laid = text::layout(t, &ctx, dark);
                        text::draw(&s, t, &laid, &rgba(t.color.resolve(dark), 1.0));
                    }
                }
                *cache = Some((dark, moving, s.to_node()));
                imp.debug.borrow_mut().cache_us = started.elapsed().as_micros();
            }
            if let Some((_, _, Some(node))) = cache.as_ref() {
                snapshot.append_node(node);
            }
        }
        if moving && let Some(op) = imp.sel_op.borrow().as_ref() {
            for stroke in imp.selection.borrow().iter() {
                draw_stroke(snapshot, &transformed(stroke, op), dark);
            }
            let ctx = self.pango_context();
            for t in imp.sel_texts.borrow().iter() {
                let moved = transformed_text(t, op, &ctx);
                let laid = text::layout(&moved, &ctx, dark);
                text::draw(snapshot, &moved, &laid, &rgba(moved.color.resolve(dark), 1.0));
            }
        }
        self.draw_selection(snapshot, view.zoom);
        if let Some(live) = imp.live.borrow().as_ref() {
            draw_stroke(snapshot, &live.stroke, dark);
        }
        if imp.debug.borrow().flags.points {
            self.draw_debug_points(snapshot, view.zoom);
        }
        snapshot.restore();

        if let Some((x, y)) = imp.eraser_at.get() {
            let builder = gsk::PathBuilder::new();
            builder.add_circle(&graphene::Point::new(x, y), self.eraser_radius_screen());
            let grey = if dark { (0.8, 0.8, 0.8) } else { (0.35, 0.35, 0.35) };
            snapshot.append_stroke(&builder.to_path(), &gsk::Stroke::new(1.0), &rgba(grey, 0.9));
        }
        self.draw_long_press(snapshot);
        self.draw_radial(snapshot);
        self.draw_debug_overlay(snapshot);
    }

    /// Selection box with corner handles, and the lasso while drawing it.
    fn draw_selection(&self, snapshot: &gtk::Snapshot, zoom: f32) {
        let imp = self.imp();
        let accent = adw::StyleManager::default().accent_color_rgba();
        let line = gsk::Stroke::new(1.5 / zoom);
        line.set_dash(&[4.0 / zoom, 3.0 / zoom]);
        let op = imp.sel_op.borrow();
        if let Some(SelOp::Lasso { points, .. }) = op.as_ref() {
            if points.len() > 1 {
                let b = gsk::PathBuilder::new();
                b.move_to(points[0].0, points[0].1);
                for p in &points[1..] {
                    b.line_to(p.0, p.1);
                }
                snapshot.append_stroke(&b.to_path(), &line, &accent);
            }
            return;
        }
        let Some(bounds) = self.selection_bounds() else { return };
        let (x0, y0) = op.as_ref().map_or((bounds.x0, bounds.y0), |o| o.apply(bounds.x0, bounds.y0));
        let (x1, y1) = op.as_ref().map_or((bounds.x1, bounds.y1), |o| o.apply(bounds.x1, bounds.y1));
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        let b = gsk::PathBuilder::new();
        b.add_rect(&rect(x0, y0, x1 - x0, y1 - y0));
        snapshot.append_stroke(&b.to_path(), &line, &accent);
        let r = HANDLE_RADIUS / zoom;
        let handles = gsk::PathBuilder::new();
        for (hx, hy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            handles.add_circle(&graphene::Point::new(hx, hy), r);
        }
        let handles = handles.to_path();
        let dark = adw::StyleManager::default().is_dark();
        snapshot.append_fill(&handles, gsk::FillRule::Winding, &rgba(Self::page_color(dark), 1.0));
        snapshot.append_stroke(&handles, &gsk::Stroke::new(1.5 / zoom), &accent);
    }

    fn draw_debug_points(&self, snapshot: &gtk::Snapshot, zoom: f32) {
        let imp = self.imp();
        let d = 3.0 / zoom;
        let dot = |snapshot: &gtk::Snapshot, p: &Point, color: (f32, f32, f32)| {
            snapshot.append_color(&rgba(color, 0.8), &rect(p.x - d / 2.0, p.y - d / 2.0, d, d));
        };
        let debug = imp.debug.borrow();
        for stroke in imp.strokes.borrow().iter() {
            let key = Rc::as_ptr(stroke);
            if let Some((_, raw)) = debug.raw.iter().find(|(k, _)| *k == key) {
                raw.iter().for_each(|p| dot(snapshot, p, (0.9, 0.1, 0.1)));
            }
            stroke.points.iter().for_each(|p| dot(snapshot, p, (0.1, 0.4, 0.95)));
        }
        if let Some(live) = imp.live.borrow().as_ref() {
            live.raw.iter().for_each(|p| dot(snapshot, p, (0.9, 0.1, 0.1)));
            live.stroke.points.iter().for_each(|p| dot(snapshot, p, (0.1, 0.4, 0.95)));
        }
    }

    fn draw_debug_overlay(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let mut d = imp.debug.borrow_mut();
        if !d.flags.input && !d.flags.perf {
            return;
        }
        let now = Instant::now();
        if d.second_start.is_none_or(|s| now - s >= Duration::from_secs(1)) {
            d.second_start = Some(now);
            d.fps = d.frames;
            d.frames = 0;
            d.samples_per_sec = d.samples;
            d.samples = 0;
        }
        let mut text = String::new();
        if d.flags.input {
            let fmt = |v: Option<f64>| v.map_or("none".into(), |v| format!("{v:.2}"));
            text += &format!(
                "INPUT\ntool: {}\npressure: {:.3}\ntilt: {} / {}\nheld buttons: {}\nlast button press: {}\naction: {}\nsamples/s: {}\nbacklog: {}\n",
                if d.tool.is_empty() { "-" } else { &d.tool },
                d.pressure,
                fmt(d.tilt.0),
                fmt(d.tilt.1),
                if d.buttons.is_empty() { "none" } else { &d.buttons },
                d.last_button.map_or("-".into(), |b| b.to_string()),
                if d.action.is_empty() { "-" } else { &d.action },
                d.samples_per_sec,
                d.backlog,
            );
        }
        if d.flags.perf {
            let strokes = imp.strokes.borrow();
            let points: usize = strokes.iter().map(|s| s.points.len()).sum();
            let view = imp.view.get();
            text += &format!(
                "PERF\nframes/s: {}\nframe: {:.2} ms\nink cache rebuild: {:.2} ms\nstrokes: {}  points: {}\nlast save: {:.1} ms\nzoom: {:.0}%  scroll: {:.0}, {:.0}\n",
                d.fps,
                d.frame_us as f64 / 1000.0,
                d.cache_us as f64 / 1000.0,
                strokes.len(),
                points,
                d.save_ms,
                view.zoom * 100.0,
                view.sx,
                view.sy,
            );
        }
        drop(d);
        let layout = self.create_pango_layout(Some(text.trim_end()));
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Monospace 9")));
        let (tw, th) = layout.pixel_size();
        snapshot.append_color(&rgba((0.0, 0.0, 0.0), 0.7), &rect(8.0, 8.0, tw as f32 + 16.0, th as f32 + 12.0));
        snapshot.save();
        snapshot.translate(&graphene::Point::new(16.0, 14.0));
        snapshot.append_layout(&layout, &rgba((1.0, 1.0, 1.0), 1.0));
        snapshot.restore();
    }

    // ---- page lifecycle ------------------------------------------------------

    /// Show the page at `path` (or nothing), saving the current one first.
    pub fn load(&self, path: Option<&Path>) {
        let imp = self.imp();
        if imp.path.borrow().as_deref() == path {
            return;
        }
        self.commit_edit();
        self.flush();
        self.cancel_input();
        imp.selection.borrow_mut().clear();
        imp.sel_texts.borrow_mut().clear();
        if let Some(old) = imp.path.take() {
            let history = imp.history.take();
            imp.stashed.borrow_mut().insert(old, (history, imp.view.get()));
        }
        let ctx = self.pango_context();
        let (layout, strokes, texts) = match path.map(PageFile::load) {
            Some(Ok(page)) => (
                page.layout,
                page.strokes.into_iter().map(Rc::new).collect(),
                page.texts
                    .into_iter()
                    .map(|mut t| {
                        t.measure(&ctx);
                        Rc::new(t)
                    })
                    .collect(),
            ),
            Some(Err(e)) => {
                self.report(format!("Couldn't open page: {e}"));
                (PageLayout::default(), Vec::new(), Vec::new())
            }
            None => (PageLayout::default(), Vec::new(), Vec::new()),
        };
        imp.layout.set(layout);
        *imp.strokes.borrow_mut() = strokes;
        *imp.texts.borrow_mut() = texts;
        self.update_extent();
        let (history, view) = path
            .and_then(|p| imp.stashed.borrow_mut().remove(p))
            .unwrap_or_else(|| (History::default(), self.initial_view()));
        *imp.history.borrow_mut() = history;
        *imp.path.borrow_mut() = path.map(Path::to_path_buf);
        self.set_view(view);
        self.notify_history();
        self.invalidate();
        self.log(format!("load {}", path.map_or("-".into(), |p| p.display().to_string())));
        if path.is_some() {
            self.fade_in();
        }
    }

    fn initial_view(&self) -> View {
        // Sheets: start a gap above the first sheet (x gets centered).
        match self.imp().layout.get().size {
            PageSize::Infinite => View::default(),
            PageSize::Sheets { .. } => View { zoom: 1.0, sx: -SHEET_GAP, sy: -SHEET_GAP },
        }
    }

    /// The open page's file moved or was renamed; keep everything else.
    pub fn retarget(&self, path: &Path) {
        *self.imp().path.borrow_mut() = Some(path.to_path_buf());
    }

    pub fn layout(&self) -> PageLayout {
        self.imp().layout.get()
    }

    /// Change the page's size/background (saved with the page).
    pub fn set_layout(&self, layout: PageLayout) {
        let imp = self.imp();
        if imp.path.borrow().is_none() || imp.layout.get() == layout {
            return;
        }
        let size_changed = imp.layout.get().size != layout.size;
        imp.layout.set(layout);
        if size_changed {
            self.set_view(self.initial_view());
        }
        self.schedule_save();
        self.queue_draw();
        self.log(format!("layout {layout:?}"));
    }

    /// Write pending changes to disk now.
    pub fn flush(&self) {
        let imp = self.imp();
        if let Some(id) = imp.save_timer.take() {
            id.remove();
        }
        if !imp.dirty.replace(false) {
            return;
        }
        let Some(path) = imp.path.borrow().clone() else { return };
        let started = Instant::now();
        let strokes = imp.strokes.borrow().iter().map(|s| (**s).clone()).collect();
        let layout = imp.layout.get();
        let texts = imp.texts.borrow().iter().map(|t| (**t).clone()).collect();
        let result = PageFile::load(&path).and_then(|mut page| {
            page.strokes = strokes;
            page.texts = texts;
            page.layout = layout;
            page.save(&path)
        });
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        imp.debug.borrow_mut().save_ms = ms;
        self.log(format!("save {ms:.1} ms"));
        if let Err(e) = result {
            self.report(format!("Couldn't save page: {e}"));
        }
    }

    fn schedule_save(&self) {
        let imp = self.imp();
        imp.dirty.set(true);
        if let Some(id) = imp.save_timer.take() {
            id.remove();
        }
        let weak = self.downgrade();
        let id = glib::timeout_add_local_once(AUTOSAVE_DELAY, move || {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().save_timer.take();
                canvas.flush();
            }
        });
        *imp.save_timer.borrow_mut() = Some(id);
    }

    fn report(&self, msg: String) {
        if let Some(f) = self.imp().on_error.borrow().as_ref() {
            f(msg);
        }
    }

    pub fn connect_error(&self, f: impl Fn(String) + 'static) {
        *self.imp().on_error.borrow_mut() = Some(Box::new(f));
    }

    /// Called with (can undo, can redo) whenever the history changes.
    pub fn connect_history(&self, f: impl Fn(bool, bool) + 'static) {
        *self.imp().on_history.borrow_mut() = Some(Box::new(f));
        self.notify_history();
    }

    /// Called with the zoom factor whenever it changes.
    pub fn connect_zoom(&self, f: impl Fn(f32) + 'static) {
        *self.imp().on_zoom.borrow_mut() = Some(Box::new(f));
    }

    fn notify_history(&self) {
        let imp = self.imp();
        let (u, r) = {
            let h = imp.history.borrow();
            (!h.undo.is_empty(), !h.redo.is_empty())
        };
        if let Some(f) = imp.on_history.borrow().as_ref() {
            f(u, r);
        }
    }

    fn update_extent(&self) {
        let imp = self.imp();
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for s in imp.strokes.borrow().iter() {
            x = x.max(s.bounds.x1);
            y = y.max(s.bounds.y1);
        }
        for t in imp.texts.borrow().iter() {
            let b = t.bounds();
            x = x.max(b.x1);
            y = y.max(b.y1);
        }
        imp.extent.set((x, y));
    }

    fn invalidate(&self) {
        self.imp().cache.take();
        self.queue_draw();
    }

    // ---- view ----------------------------------------------------------------------

    /// Clamp and apply a view: infinite pages scroll right/down only (up to
    /// a screen past the ink); sheets stay centered when narrower than
    /// the window.
    fn set_view(&self, view: View) {
        let imp = self.imp();
        let zoom = view.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let (vw, vh) = (self.width() as f32 / zoom, self.height() as f32 / zoom);
        let (ex, ey) = imp.extent.get();
        let (mut sx, mut sy) = (view.sx, view.sy);
        match imp.layout.get().size.sheet_px() {
            None => {
                // One full screen past the ink; at least two screens in all.
                sx = sx.clamp(0.0, ex.max(vw));
                sy = sy.clamp(0.0, ey.max(vh));
            }
            Some((w, _)) => {
                let bottom = self.sheets().last().map_or(0.0, |s| s.y() + s.height());
                if vw >= w + 2.0 * SHEET_GAP {
                    sx = (w - vw) / 2.0;
                } else {
                    sx = sx.clamp(-SHEET_GAP, w + SHEET_GAP - vw);
                }
                sy = sy.clamp(-SHEET_GAP, (bottom + SHEET_GAP - vh).max(-SHEET_GAP));
            }
        }
        let new = View { zoom, sx, sy };
        let old = imp.view.replace(new);
        if old != new {
            self.queue_draw();
            if self.is_editing() {
                if old.zoom != new.zoom {
                    self.update_editor_css();
                }
                self.queue_allocate();
            }
            if old.zoom != new.zoom
                && let Some(f) = imp.on_zoom.borrow().as_ref() {
                    f(zoom);
                }
        }
    }

    pub fn zoom(&self) -> f32 {
        self.imp().view.get().zoom
    }

    /// Zoom to `zoom`, keeping the page point under screen (x, y) in place.
    pub fn zoom_at(&self, zoom: f32, x: f32, y: f32) {
        let v = self.imp().view.get();
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let (px, py) = (x / v.zoom + v.sx, y / v.zoom + v.sy);
        self.set_view(View { zoom, sx: px - x / zoom, sy: py - y / zoom });
    }

    /// Smoothly zoom to `zoom` around screen (x, y). Repeated calls while
    /// animating continue from the pending target.
    pub fn zoom_animated(&self, zoom: f32, x: f32, y: f32) {
        let imp = self.imp();
        let target = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if let Some((anim, _)) = imp.zoom_anim.take() {
            anim.pause();
        }
        let from = self.zoom();
        let v = imp.view.get();
        let anchor = (x / v.zoom + v.sx, y / v.zoom + v.sy);
        let weak = self.downgrade();
        let target_fn = adw::CallbackAnimationTarget::new(move |t| {
            let Some(canvas) = weak.upgrade() else { return };
            let z = from + (target - from) * t as f32;
            canvas.set_view(View { zoom: z, sx: anchor.0 - x / z, sy: anchor.1 - y / z });
        });
        let anim = adw::TimedAnimation::builder()
            .widget(self)
            .value_from(0.0)
            .value_to(1.0)
            .duration(ZOOM_ANIMATION_MS)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target_fn)
            .build();
        anim.play();
        *imp.zoom_anim.borrow_mut() = Some((anim, target));
    }

    /// Zoom target, including one still being animated to.
    fn zoom_target(&self) -> f32 {
        let imp = self.imp();
        match imp.zoom_anim.borrow().as_ref() {
            Some((anim, target)) if anim.state() == adw::AnimationState::Playing => *target,
            _ => self.zoom(),
        }
    }

    /// Zoom by a factor around the middle of the canvas (keyboard/header).
    pub fn zoom_step(&self, factor: f32) {
        self.zoom_centered(self.zoom_target() * factor);
    }

    pub fn zoom_centered(&self, zoom: f32) {
        self.zoom_animated(zoom, self.width() as f32 / 2.0, self.height() as f32 / 2.0);
        self.log(format!("zoom {:.0}%", zoom.clamp(MIN_ZOOM, MAX_ZOOM) * 100.0));
    }

    /// Fade the page in (on page switch).
    fn fade_in(&self) {
        let imp = self.imp();
        let weak = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().fade.set(v);
                canvas.queue_draw();
            }
        });
        let anim = adw::TimedAnimation::new(self, 0.0, 1.0, FADE_MS, target);
        anim.set_easing(adw::Easing::EaseOutQuad);
        anim.play();
        *imp.fade_anim.borrow_mut() = Some(anim.upcast());
    }

    /// Scroll by a screen-pixel delta.
    fn scroll_by(&self, dx: f32, dy: f32) {
        let v = self.imp().view.get();
        self.set_view(View { sx: v.sx + dx / v.zoom, sy: v.sy + dy / v.zoom, ..v });
    }

    fn to_page(&self, x: f32, y: f32) -> (f32, f32) {
        let v = self.imp().view.get();
        (x / v.zoom + v.sx, y / v.zoom + v.sy)
    }

    // ---- config ------------------------------------------------------------------

    pub fn configure(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.imp().config.borrow_mut());
        if self.imp().config.borrow().tool != CanvasTool::Text {
            self.commit_edit();
        }
        self.reset_cursor();
    }

    fn reset_cursor(&self) {
        let name = match self.imp().config.borrow().tool {
            CanvasTool::Text => "text",
            CanvasTool::Hand => "grab",
            _ => "crosshair",
        };
        self.set_cursor_from_name(Some(name));
    }

    pub fn set_debug(&self, flags: DebugFlags) {
        let imp = self.imp();
        {
            let mut d = imp.debug.borrow_mut();
            d.flags = flags;
            if !flags.points {
                d.raw.clear();
            }
        }
        self.queue_draw();
    }

    pub fn connect_log(&self, f: impl Fn(String) + 'static) {
        *self.imp().on_log.borrow_mut() = Some(Box::new(f));
    }

    fn log(&self, msg: String) {
        let imp = self.imp();
        if !imp.debug.borrow().flags.log {
            return;
        }
        if let Some(f) = imp.on_log.borrow().as_ref() {
            f(msg);
        }
    }

    fn eraser_radius_screen(&self) -> f32 {
        match self.imp().config.borrow().eraser_mode {
            EraserMode::Stroke => STROKE_ERASER_RADIUS,
            EraserMode::Partial => PARTIAL_ERASER_RADIUS,
        }
    }

    // ---- undo --------------------------------------------------------------------

    /// Record the current strokes as an undo step, then replace them.
    fn commit(&self, before: Strokes, after: Strokes) {
        let texts = self.imp().texts.borrow().clone();
        self.commit_all((before, texts.clone()), (after, texts));
    }

    /// Record `before` as an undo step and switch to `after`.
    fn commit_all(&self, before: Snap, after: Snap) {
        let imp = self.imp();
        {
            let mut h = imp.history.borrow_mut();
            h.undo.push(before);
            h.redo.clear();
        }
        *imp.strokes.borrow_mut() = after.0;
        *imp.texts.borrow_mut() = after.1;
        self.after_change();
    }

    fn snap(&self) -> Snap {
        let imp = self.imp();
        (imp.strokes.borrow().clone(), imp.texts.borrow().clone())
    }

    fn restore(&self, snap: Snap) -> Snap {
        let imp = self.imp();
        let strokes = std::mem::replace(&mut *imp.strokes.borrow_mut(), snap.0);
        let texts = std::mem::replace(&mut *imp.texts.borrow_mut(), snap.1);
        (strokes, texts)
    }

    fn after_change(&self) {
        self.update_extent();
        self.invalidate();
        self.schedule_save();
        self.notify_history();
    }

    pub fn undo(&self) {
        let imp = self.imp();
        self.commit_edit();
        let Some(prev) = imp.history.borrow_mut().undo.pop() else { return };
        let current = self.restore(prev);
        imp.history.borrow_mut().redo.push(current);
        self.prune_selection();
        self.after_change();
        self.log("undo".into());
    }

    pub fn redo(&self) {
        let imp = self.imp();
        self.commit_edit();
        let Some(next) = imp.history.borrow_mut().redo.pop() else { return };
        let current = self.restore(next);
        imp.history.borrow_mut().undo.push(current);
        self.prune_selection();
        self.after_change();
        self.log("redo".into());
    }

    // ---- input -------------------------------------------------------------------

    fn setup_input(&self) {
        // Pen state GTK doesn't report on some platforms (see platform::pen_buttons).
        self.connect_realize(|canvas| {
            if let Some(surface) = canvas.native().and_then(|n| n.surface()) {
                crate::platform::hook_pen(&surface);
            }
        });

        // Only the tip touching the tablet starts a gesture. Side buttons
        // pressed while hovering also arrive as button presses (2/3); they
        // only matter as held modifiers when the tip comes down.
        let stylus = gtk::GestureStylus::new();
        stylus.set_button(gdk::BUTTON_PRIMARY);
        let weak = self.downgrade();
        stylus.connect_down(move |g, x, y| {
            let Some(canvas) = weak.upgrade() else { return };
            if g.current_button() != gdk::BUTTON_PRIMARY {
                canvas.log(format!("pen button {} while hovering: ignored", g.current_button()));
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            // Keep the mouse gesture from also handling the pen.
            g.set_state(gtk::EventSequenceState::Claimed);
            canvas.debug_stylus(g);
            let action = canvas.stylus_action(g);
            let p = g.axis(gdk::AxisUse::Pressure).unwrap_or(0.5) as f32;
            let buttons = canvas.imp().debug.borrow().buttons.clone();
            canvas.log(format!("pen down at {x:.0},{y:.0} p={p:.3} held=[{buttons}] -> {action:?}"));
            canvas.imp().mods.set(g.current_event_state());
            canvas.begin(action, x as f32, y as f32, p, true);
            canvas.imp().from_button.set(canvas.stylus_mapping(g).1);
        });
        let weak = self.downgrade();
        stylus.connect_motion(move |g, x, y| {
            let Some(canvas) = weak.upgrade() else { return };
            canvas.imp().pointer.set((x as f32, y as f32));
            canvas.debug_stylus(g);
            // Samples merged into this frame, so fast strokes stay detailed.
            let mut count = 0;
            if let Some(backlog) = g.backlog() {
                for entry in backlog.iter() {
                    let (axes, flags) = (entry.axes(), entry.flags());
                    if flags.contains(gdk::AxisFlags::X | gdk::AxisFlags::Y) {
                        let p = if flags.contains(gdk::AxisFlags::PRESSURE) { axes[5] } else { 0.5 };
                        canvas.extend(axes[1] as f32, axes[2] as f32, p as f32);
                        count += 1;
                    }
                }
            }
            let p = g.axis(gdk::AxisUse::Pressure).unwrap_or(0.5) as f32;
            // Older backlog samples belong to the old action; switch now.
            canvas.switch_gesture_on_buttons(g, x as f32, y as f32, p);
            canvas.extend(x as f32, y as f32, p);
            {
                let mut d = canvas.imp().debug.borrow_mut();
                d.backlog = count;
                d.samples += count as u32 + 1;
            }
            canvas.queue_debug_draw();
        });
        let weak = self.downgrade();
        stylus.connect_up(move |_, x, y| {
            if let Some(canvas) = weak.upgrade() {
                canvas.end(x as f32, y as f32);
            }
        });
        self.add_controller(stylus);

        // Mouse (and anything else without a tablet tool) draws at constant width.
        let drag = gtk::GestureDrag::builder().button(gdk::BUTTON_PRIMARY).build();
        let weak = self.downgrade();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(canvas) = weak.upgrade() else { return };
            if g.current_event().and_then(|e| e.device_tool()).is_some() {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            let action = canvas.main_action();
            canvas.log(format!("mouse down at {x:.0},{y:.0} -> {action:?}"));
            canvas.imp().mods.set(g.current_event_state());
            canvas.begin(action, x as f32, y as f32, 0.5, false);
        });
        let weak = self.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(canvas) = weak.upgrade() else { return };
            let (x0, y0) = g.start_point().unwrap_or((0.0, 0.0));
            canvas.extend((x0 + dx) as f32, (y0 + dy) as f32, 0.5);
        });
        let weak = self.downgrade();
        drag.connect_drag_end(move |g, dx, dy| {
            let Some(canvas) = weak.upgrade() else { return };
            let (x0, y0) = g.start_point().unwrap_or((0.0, 0.0));
            canvas.end((x0 + dx) as f32, (y0 + dy) as f32);
        });
        self.add_controller(drag);

        // Middle mouse drag pans.
        let pan = gtk::GestureDrag::builder().button(gdk::BUTTON_MIDDLE).build();
        let weak = self.downgrade();
        pan.connect_drag_begin(move |g, x, y| {
            let Some(canvas) = weak.upgrade() else { return };
            if g.current_event().and_then(|e| e.device_tool()).is_some() {
                // A pen side button press while hovering isn't a pan.
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            canvas.begin(Action::Pan, x as f32, y as f32, 0.5, false);
        });
        let weak = self.downgrade();
        pan.connect_drag_update(move |g, dx, dy| {
            let Some(canvas) = weak.upgrade() else { return };
            let (x0, y0) = g.start_point().unwrap_or((0.0, 0.0));
            canvas.extend((x0 + dx) as f32, (y0 + dy) as f32, 0.5);
        });
        let weak = self.downgrade();
        pan.connect_drag_end(move |g, dx, dy| {
            let Some(canvas) = weak.upgrade() else { return };
            let (x0, y0) = g.start_point().unwrap_or((0.0, 0.0));
            canvas.end((x0 + dx) as f32, (y0 + dy) as f32);
        });
        self.add_controller(pan);

        // Wheel / touchpad: scroll, Shift = horizontal, Ctrl = zoom.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let weak = self.downgrade();
        scroll.connect_scroll(move |c, dx, dy| {
            let Some(canvas) = weak.upgrade() else { return glib::Propagation::Proceed };
            let wheel = c.unit() == gdk::ScrollUnit::Wheel;
            let (mut dx, mut dy) = (dx as f32, dy as f32);
            let state = c.current_event_state();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                let (px, py) = canvas.imp().pointer.get();
                if wheel {
                    canvas.zoom_animated(canvas.zoom_target() * 1.15f32.powf(-dy), px, py);
                } else {
                    // Touchpads send many small steps; follow them directly.
                    canvas.zoom_at(canvas.zoom() * (1.0 - dy * 0.01).max(0.1), px, py);
                }
                return glib::Propagation::Stop;
            }
            if wheel {
                dx *= WHEEL_STEP;
                dy *= WHEEL_STEP;
            }
            if state.contains(gdk::ModifierType::SHIFT_MASK) && dx == 0.0 {
                (dx, dy) = (dy, 0.0);
            }
            canvas.scroll_by(dx, dy);
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        // Touchpad / touchscreen pinch.
        let pinch = gtk::GestureZoom::new();
        let base = Rc::new(Cell::new(1.0f32));
        let weak = self.downgrade();
        let b = base.clone();
        pinch.connect_begin(move |_, _| {
            if let Some(canvas) = weak.upgrade() {
                b.set(canvas.zoom());
            }
        });
        let weak = self.downgrade();
        pinch.connect_scale_changed(move |g, scale| {
            let Some(canvas) = weak.upgrade() else { return };
            let (cx, cy) = g.bounding_box_center().unwrap_or((0.0, 0.0));
            canvas.zoom_at(base.get() * scale as f32, cx as f32, cy as f32);
        });
        self.add_controller(pinch);

        // Pointer position (zoom anchor) and hover readout for debugging.
        let motion = gtk::EventControllerMotion::new();
        let weak = self.downgrade();
        motion.connect_motion(move |c, x, y| {
            let Some(canvas) = weak.upgrade() else { return };
            canvas.imp().pointer.set((x as f32, y as f32));
            if canvas.menu_open() {
                canvas.radial_hover(x as f32, y as f32);
            }
            if canvas.imp().debug.borrow().flags.input {
                canvas.debug_event_state(c.current_event_state(), c.current_event());
                canvas.queue_debug_draw();
            }
        });
        self.add_controller(motion);

        // Pen side buttons pressed/released while the tip is already down:
        // switch the running gesture right away. Raw events, because the
        // held-button state isn't reliably part of every motion event.
        let raw = gtk::EventControllerLegacy::new();
        raw.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.downgrade();
        raw.connect_event(move |_, event| {
            let Some(canvas) = weak.upgrade() else { return glib::Propagation::Proceed };
            if event.device_tool().is_none() {
                return glib::Propagation::Proceed;
            }
            let held = canvas.imp().pen_buttons.get();
            let (button, pressed) = if let Some(b) = event.downcast_ref::<gdk::ButtonEvent>() {
                let button = b.button();
                if button == gdk::BUTTON_PRIMARY {
                    return glib::Propagation::Proceed;
                }
                let pressed = event.event_type() == gdk::EventType::ButtonPress;
                if let Some(mask) = button_mask(button) {
                    canvas.imp().pen_buttons.set(if pressed { held | mask } else { held - mask });
                }
                (button, pressed)
            } else if event.event_type() == gdk::EventType::MotionNotify {
                // Windows Ink sends no button events for side buttons while
                // hovering, only the held mask on motion: turn its edges into
                // presses/releases. Mid-gesture, the stylus motion handler
                // already follows the mask.
                let now = (event.modifier_state() | crate::platform::pen_buttons())
                    & (gdk::ModifierType::BUTTON2_MASK | gdk::ModifierType::BUTTON3_MASK);
                canvas.imp().pen_buttons.set(now);
                if now == held || canvas.imp().action.get() != Action::Ignore {
                    return glib::Propagation::Proceed;
                }
                let changed = now ^ held;
                let button = if changed.contains(gdk::ModifierType::BUTTON3_MASK) { 3 } else { 2 };
                (button, now.intersects(changed))
            } else {
                return glib::Propagation::Proceed;
            };
            if canvas.imp().action.get() == Action::Ignore {
                // Hovering: a button mapped to the menu opens it at the pen;
                // releasing it over an item picks that item (released on the
                // center, the menu stays open for picking by touch).
                if canvas.button_opens_menu(button) {
                    if pressed && !canvas.menu_open() {
                        let (x, y) = canvas.imp().pointer.get();
                        canvas.log(format!("button {button}: radial menu"));
                        canvas.open_radial(x, y);
                    } else if !pressed && canvas.menu_hover().is_some() {
                        canvas.pick_radial();
                    }
                }
                return glib::Propagation::Proceed;
            }
            // Last pen position on the canvas (surface coordinates in the
            // event may be offset by window decorations).
            let (x, y) = canvas.imp().pointer.get();
            canvas.side_button_mid_gesture(button, pressed, x, y);
            glib::Propagation::Proceed
        });
        self.add_controller(raw);

        // Selection shortcuts (only while the canvas has focus, so text
        // fields elsewhere keep their own copy/paste).
        let keys = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(canvas) = weak.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match (ctrl, key.to_lower()) {
                (false, gdk::Key::Escape) if canvas.menu_open() => {
                    canvas.close_radial();
                }
                (false, gdk::Key::Delete | gdk::Key::BackSpace) if canvas.has_selection() => canvas.delete_selection(),
                (false, gdk::Key::Escape) if canvas.has_selection() => canvas.clear_selection(),
                (true, gdk::Key::c) => canvas.copy_selection(),
                (true, gdk::Key::x) => canvas.cut_selection(),
                (true, gdk::Key::v) => canvas.paste(true),
                (true, gdk::Key::d) => canvas.duplicate_selection(),
                (true, gdk::Key::a) => canvas.select_all(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.add_controller(keys);

        // Button presses (incl. pen side buttons while hovering) for the
        // debug readout and log; never claims, so normal handling continues.
        let clicks = gtk::GestureClick::builder().button(0).build();
        let weak = self.downgrade();
        clicks.connect_pressed(move |g, _, _, _| {
            let Some(canvas) = weak.upgrade() else { return };
            let button = g.current_button();
            let from_pen = g.current_event().and_then(|e| e.device_tool()).is_some();
            canvas.imp().debug.borrow_mut().last_button = Some(button);
            canvas.log(format!("button {button} pressed ({})", if from_pen { "pen" } else { "mouse" }));
            canvas.queue_debug_draw();
        });
        self.add_controller(clicks);
    }

    fn queue_debug_draw(&self) {
        let flags = self.imp().debug.borrow().flags;
        if flags.input || flags.perf {
            self.queue_draw();
        }
    }

    fn debug_event_state(&self, state: gdk::ModifierType, event: Option<gdk::Event>) {
        let mut d = self.imp().debug.borrow_mut();
        let names = [
            (gdk::ModifierType::BUTTON1_MASK, "1(tip/left)"),
            (gdk::ModifierType::BUTTON2_MASK, "2(middle)"),
            (gdk::ModifierType::BUTTON3_MASK, "3(right)"),
            (gdk::ModifierType::BUTTON4_MASK, "4"),
            (gdk::ModifierType::BUTTON5_MASK, "5"),
        ];
        d.buttons = names
            .iter()
            .filter(|(m, _)| state.contains(*m))
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" ");
        d.tool = match event.and_then(|e| e.device_tool()) {
            Some(tool) => format!("{:?} (serial {})", tool.tool_type(), tool.serial()),
            None => "mouse / no tool".into(),
        };
    }

    fn debug_stylus(&self, g: &gtk::GestureStylus) {
        let flags = self.imp().debug.borrow().flags;
        if !flags.input && !flags.log {
            return;
        }
        self.debug_event_state(g.current_event_state(), g.current_event());
        let mut d = self.imp().debug.borrow_mut();
        d.pressure = g.axis(gdk::AxisUse::Pressure).unwrap_or(0.0);
        d.tilt = (g.axis(gdk::AxisUse::Xtilt), g.axis(gdk::AxisUse::Ytilt));
    }

    fn main_action(&self) -> Action {
        match self.imp().config.borrow().tool {
            CanvasTool::Draw => Action::Draw,
            CanvasTool::Erase => Action::Erase,
            CanvasTool::Select => Action::Select,
            CanvasTool::Text => Action::Text,
            CanvasTool::Hand => Action::Pan,
        }
    }

    /// The pen's eraser end and held side buttons map through the settings.
    fn stylus_action(&self, g: &gtk::GestureStylus) -> Action {
        self.stylus_mapping(g).0
    }

    /// The action for the pen's current state, and whether a held side
    /// button decided it.
    fn stylus_mapping(&self, g: &gtk::GestureStylus) -> (Action, bool) {
        let state = g.current_event_state() | crate::platform::pen_buttons();
        let held = state.intersects(gdk::ModifierType::BUTTON2_MASK | gdk::ModifierType::BUTTON3_MASK);
        let config = self.imp().config.borrow();
        let mapped = if g.device_tool().is_some_and(|t| t.tool_type() == gdk::DeviceToolType::Eraser) {
            Some(config.eraser_end)
        } else if state.contains(gdk::ModifierType::BUTTON2_MASK) {
            Some(config.button_2)
        } else if state.contains(gdk::ModifierType::BUTTON3_MASK) {
            Some(config.button_3)
        } else {
            None
        };
        drop(config);
        let action = match mapped {
            Some(ButtonAction::Eraser) => Action::Erase,
            Some(ButtonAction::Pan) => Action::Pan,
            Some(ButtonAction::Select) => Action::Select,
            Some(ButtonAction::Draw) => Action::Draw,
            // The menu opens on the button press while hovering.
            Some(ButtonAction::Menu) => Action::Ignore,
            Some(ButtonAction::Click) | None => self.main_action(),
        };
        let is_eraser_end = g.device_tool().is_some_and(|t| t.tool_type() == gdk::DeviceToolType::Eraser);
        (action, held && !is_eraser_end && mapped != Some(ButtonAction::Click))
    }

    /// A side button event while the tip is down (from raw events).
    fn side_button_mid_gesture(&self, button: u32, pressed: bool, x: f32, y: f32) {
        let imp = self.imp();
        let mapped = {
            let c = imp.config.borrow();
            match button {
                2 => c.button_2,
                3 => c.button_3,
                _ => return,
            }
        };
        let wanted = match mapped {
            ButtonAction::Eraser => Action::Erase,
            ButtonAction::Pan => Action::Pan,
            ButtonAction::Select => Action::Select,
            ButtonAction::Draw => Action::Draw,
            ButtonAction::Menu | ButtonAction::Click => return,
        };
        let current = imp.action.get();
        let p = imp.live.borrow().as_ref().map_or(0.5, |l| l.last_raw.p);
        if pressed && wanted != current {
            self.log(format!("button {button} pressed mid-gesture: {current:?} -> {wanted:?}"));
            self.end(x, y);
            self.begin(wanted, x, y, p, true);
            imp.from_button.set(true);
        } else if !pressed && imp.from_button.get() && current == wanted {
            self.log(format!("button {button} released mid-gesture: {current:?} stopped"));
            self.end(x, y);
            imp.from_button.set(false);
        }
    }

    /// A side button pressed or released while the tip is down switches
    /// the gesture: finish what was being done and continue with the new
    /// action from here. Releasing the button stops (no stray line).
    fn switch_gesture_on_buttons(&self, g: &gtk::GestureStylus, x: f32, y: f32, p: f32) {
        let imp = self.imp();
        let current = imp.action.get();
        let (wanted, by_button) = self.stylus_mapping(g);
        if by_button && wanted != current {
            self.log(format!("button pressed mid-gesture: {current:?} -> {wanted:?}"));
            self.end(x, y);
            imp.mods.set(g.current_event_state());
            self.begin(wanted, x, y, p, true);
            imp.from_button.set(true);
        } else if !by_button && imp.from_button.get() && current != Action::Ignore {
            self.log(format!("button released mid-gesture: {current:?} stopped"));
            self.end(x, y);
            imp.from_button.set(false);
        }
    }

    /// Called while a pen input temporarily overrides the tool, with the
    /// action it maps to, and with `None` when it ends.
    pub fn connect_temp_tool(&self, f: impl Fn(Option<Action>) + 'static) {
        *self.imp().on_temp_tool.borrow_mut() = Some(Box::new(f));
    }

    fn set_temp_tool(&self, action: Option<Action>) {
        if let Some(f) = self.imp().on_temp_tool.borrow().as_ref() {
            f(action);
        }
    }

    fn cancel_input(&self) {
        let imp = self.imp();
        if let Some(t) = imp.shape_timer.take() {
            t.remove();
        }
        imp.live.take();
        imp.erasing.take();
        imp.panning.take();
        imp.sel_op.take();
        imp.eraser_at.set(None);
        imp.action.set(Action::Ignore);
        self.cancel_long_press();
        self.set_temp_tool(None);
    }

    /// Start a gesture at screen position (x, y). `pen` selects pen
    /// behaviour (pan rail).
    fn begin(&self, action: Action, x: f32, y: f32, p: f32, pen: bool) {
        let imp = self.imp();
        self.cancel_input();
        // With the radial menu open, any touch picks from it.
        if self.menu_open() {
            imp.action.set(Action::Menu);
            self.radial_hover(x, y);
            return;
        }
        if imp.path.borrow().is_none() {
            return;
        }
        // Touching the page outside the open text box finishes it (panning
        // keeps it open).
        if action != Action::Pan {
            if self.in_editor(x, y) {
                return;
            }
            self.commit_edit();
        }
        self.grab_focus();
        imp.action.set(action);
        imp.debug.borrow_mut().action = format!("{action:?}");
        if action != self.main_action() {
            self.set_temp_tool(Some(action));
        }
        if matches!(action, Action::Draw | Action::Erase) {
            self.clear_selection();
        }
        match action {
            Action::Draw => {
                let (brush, radius_mm) = {
                    let c = imp.config.borrow();
                    (c.brush, c.stabilizer_mm)
                };
                let (px, py) = self.to_page(x, y);
                let raw = Point { x: px, y: py, p };
                let mut stroke = Stroke::new(brush.tool, brush.color, brush.width);
                stroke.points.push(raw);
                // String length is in screen mm, so it feels the same at any zoom.
                let mut smoother = Smoother::new(radius_mm * ink::PX_PER_MM / self.zoom());
                smoother.push(raw);
                *imp.live.borrow_mut() =
                    Some(Live { stroke, smoother, last_raw: raw, raw: vec![raw], shape: None, hold_from: (x, y) });
                self.restart_shape_timer();
            }
            Action::Erase => {
                let before = imp.strokes.borrow().clone();
                *imp.erasing.borrow_mut() = Some(Erasing { before, last: None, changed: false });
                self.erase_to(x, y);
            }
            Action::Select => self.begin_select(x, y),
            Action::Text => imp.text_press.set(Some((x, y))),
            Action::Menu => {}
            Action::Pan => {
                *imp.panning.borrow_mut() =
                    Some(Panning { start: (x, y), view: imp.view.get(), rail: pen, decided: false });
                self.set_cursor_from_name(Some("grabbing"));
            }
            Action::Ignore => {}
        }
        if action != Action::Ignore {
            self.start_long_press(x, y);
        }
        self.queue_draw();
    }

    fn extend(&self, x: f32, y: f32, p: f32) {
        let imp = self.imp();
        self.check_long_press(x, y);
        match imp.action.get() {
            Action::Draw => {
                let (px, py) = self.to_page(x, y);
                let mut live = imp.live.borrow_mut();
                let Some(live) = live.as_mut() else { return };
                let raw = Point { x: px, y: py, p };
                live.last_raw = raw;
                live.raw.push(raw);
                match live.shape {
                    // A snapped line/arrow keeps following the pen (still
                    // snapping to 15° steps); circles stay put.
                    Some(kind @ (ShapeKind::Line | ShapeKind::Arrow)) => {
                        let first = live.stroke.points[0];
                        let start = (first.x, first.y);
                        let end = shapes::snap_angle(start, (px, py));
                        live.stroke.points = match kind {
                            ShapeKind::Arrow => shapes::arrow_points(start, end, first.p),
                            _ => shapes::line_points(start, end, first.p),
                        };
                    }
                    // A snapped circle keeps its center; its edge follows
                    // the pen, so it can be resized before lifting.
                    Some(ShapeKind::Circle) => {
                        let p = live.stroke.points[0].p;
                        let center = shapes::circle_center(&live.stroke.points);
                        let r = ((px - center.0).powi(2) + (py - center.1).powi(2)).sqrt().max(1.0);
                        live.stroke.points = shapes::circle_points(center, r, p);
                    }
                    None => {
                        if let Some(smoothed) = live.smoother.push(raw) {
                            ink::push_point(&mut live.stroke.points, smoothed);
                        }
                        let (hx, hy) = live.hold_from;
                        if ((x - hx).powi(2) + (y - hy).powi(2)).sqrt() > SHAPE_HOLD_SLOP {
                            live.hold_from = (x, y);
                            self.restart_shape_timer();
                        }
                    }
                }
            }
            Action::Erase => self.erase_to(x, y),
            Action::Select => self.extend_select(x, y),
            Action::Text => {}
            Action::Menu => self.radial_hover(x, y),
            Action::Pan => {
                let mut panning = imp.panning.borrow_mut();
                let Some(pan) = panning.as_mut() else { return };
                let (dx, dy) = (x - pan.start.0, y - pan.start.1);
                if pan.rail {
                    // Starting mostly sideways, or insisting far enough
                    // sideways, frees the pan; otherwise it stays vertical.
                    if !pan.decided && dx.abs() + dy.abs() > 12.0 {
                        pan.decided = true;
                        pan.rail = dx.abs() <= 2.0 * dy.abs();
                    }
                    if dx.abs() > RAIL_BREAK {
                        pan.rail = false;
                    }
                }
                let dx = if pan.rail { 0.0 } else { dx };
                let view = pan.view;
                drop(panning);
                self.set_view(View { sx: view.sx - dx / view.zoom, sy: view.sy - dy / view.zoom, ..view });
                return;
            }
            Action::Ignore => return,
        }
        self.queue_draw();
    }

    fn end(&self, x: f32, y: f32) {
        let imp = self.imp();
        let action = imp.action.replace(Action::Ignore);
        self.cancel_long_press();
        self.set_temp_tool(None);
        match action {
            Action::Draw => {
                let Some(mut live) = imp.live.take() else { return };
                if let Some(t) = imp.shape_timer.take() {
                    t.remove();
                }
                let passes = imp.config.borrow().smooth_passes;
                if live.shape.is_none() {
                    // Finish exactly where the pen lifted, not where the
                    // stabilized line had caught up to.
                    let (px, py) = self.to_page(x, y);
                    let last = Point { x: px, y: py, p: live.last_raw.p };
                    ink::push_point(&mut live.stroke.points, last);
                    live.stroke.points = ink::smooth_points(&live.stroke.points, passes);
                }
                live.stroke.update_bounds();
                self.log(format!(
                    "stroke: {} raw samples -> {} points ({passes} smoothing passes)",
                    live.raw.len(),
                    live.stroke.points.len(),
                ));
                let stroke = Rc::new(live.stroke);
                {
                    let mut d = imp.debug.borrow_mut();
                    if d.flags.points {
                        d.raw.push((Rc::as_ptr(&stroke), live.raw));
                        // Bound memory; only recent strokes are interesting.
                        if d.raw.len() > 200 {
                            d.raw.remove(0);
                        }
                    }
                }
                let before = imp.strokes.borrow().clone();
                let mut after = before.clone();
                after.push(stroke);
                self.commit(before, after);
            }
            Action::Erase => {
                self.erase_to(x, y);
                imp.eraser_at.set(None);
                if let Some(e) = imp.erasing.take()
                    && e.changed
                {
                    let after = imp.strokes.borrow().clone();
                    self.log(format!("erase: {} -> {} strokes", e.before.len(), after.len()));
                    self.commit(e.before, after);
                }
                self.queue_draw();
            }
            Action::Select => self.end_select(x, y),
            Action::Text => self.text_tool_click(x, y),
            Action::Menu => {
                // Lifting the tip picks; on the center it just closes.
                self.radial_hover(x, y);
                self.pick_radial();
            }
            Action::Pan => {
                imp.panning.take();
                self.reset_cursor();
            }
            Action::Ignore => {}
        }
    }

    /// (Re)start the "held still" timer for shape snapping.
    fn restart_shape_timer(&self) {
        let imp = self.imp();
        if let Some(t) = imp.shape_timer.take() {
            t.remove();
        }
        let Some(hold) = imp.config.borrow().shape_hold else { return };
        let weak = self.downgrade();
        let id = glib::timeout_add_local_once(hold, move || {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().shape_timer.take();
                canvas.snap_live_shape();
            }
        });
        *imp.shape_timer.borrow_mut() = Some(id);
    }

    /// The pen was held still: replace the live stroke with the shape it
    /// looks like, if any.
    fn snap_live_shape(&self) {
        let imp = self.imp();
        let zoom = self.zoom();
        let mut live = imp.live.borrow_mut();
        let Some(live) = live.as_mut().filter(|l| l.shape.is_none()) else { return };
        // Include the pen's current position (the stabilizer lags).
        let mut points = live.stroke.points.clone();
        ink::push_point(&mut points, live.last_raw);
        if let Some(shape) = shapes::recognize(&points, 3.0 / zoom) {
            let kind = shape.kind;
            live.stroke.points = shape.points;
            live.shape = Some(kind);
            self.log(format!("shape: {kind:?}"));
            self.queue_draw();
        }
    }

    // ---- selection ---------------------------------------------------------------

    pub fn has_selection(&self) -> bool {
        let imp = self.imp();
        !imp.selection.borrow().is_empty() || !imp.sel_texts.borrow().is_empty()
    }

    pub fn clear_selection(&self) {
        let imp = self.imp();
        let had = !imp.selection.take().is_empty() | !imp.sel_texts.take().is_empty();
        if had {
            self.queue_draw();
        }
    }

    /// Bounds of everything selected.
    fn selection_bounds(&self) -> Option<ink::Bounds> {
        let imp = self.imp();
        let strokes = union_bounds(imp.selection.borrow().iter());
        let texts = imp.sel_texts.borrow().iter().map(|t| text_bounds(t)).reduce(merge_bounds);
        match (strokes, texts) {
            (Some(a), Some(b)) => Some(merge_bounds(a, b)),
            (a, b) => a.or(b),
        }
    }

    /// Drop selected items that no longer exist (after undo, erase…).
    fn prune_selection(&self) {
        let imp = self.imp();
        let strokes = imp.strokes.borrow();
        imp.selection.borrow_mut().retain(|s| strokes.iter().any(|x| Rc::ptr_eq(x, s)));
        let texts = imp.texts.borrow();
        imp.sel_texts.borrow_mut().retain(|t| texts.iter().any(|x| Rc::ptr_eq(x, t)));
    }

    fn begin_select(&self, x: f32, y: f32) {
        let imp = self.imp();
        let (px, py) = self.to_page(x, y);
        let add = imp.mods.get().intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK);
        let op = match self.selection_bounds() {
            Some(b) if !add => {
                let r = 2.0 * HANDLE_RADIUS / self.zoom();
                let corners = [(b.x0, b.y0), (b.x1, b.y0), (b.x0, b.y1), (b.x1, b.y1)];
                let opposite = [(b.x1, b.y1), (b.x0, b.y1), (b.x1, b.y0), (b.x0, b.y0)];
                let handle = corners.iter().position(|(cx, cy)| (cx - px).abs() <= r && (cy - py).abs() <= r);
                match handle {
                    Some(i) => Some(SelOp::Resize {
                        anchor: opposite[i],
                        corner: corners[i],
                        scale: (1.0, 1.0),
                        free: imp.mods.get().contains(gdk::ModifierType::SHIFT_MASK),
                    }),
                    None if px >= b.x0 && px <= b.x1 && py >= b.y0 && py <= b.y1 => {
                        Some(SelOp::Move { start: (px, py), delta: (0.0, 0.0) })
                    }
                    None => None,
                }
            }
            _ => None,
        };
        let op = op.unwrap_or(SelOp::Lasso { points: vec![(px, py)], add });
        if op.transforms() {
            self.set_cursor_from_name(Some("grabbing"));
        }
        *imp.sel_op.borrow_mut() = Some(op);
    }

    fn extend_select(&self, x: f32, y: f32) {
        let (px, py) = self.to_page(x, y);
        let mut op = self.imp().sel_op.borrow_mut();
        match op.as_mut() {
            Some(SelOp::Lasso { points, .. }) => points.push((px, py)),
            Some(SelOp::Move { start, delta }) => *delta = (px - start.0, py - start.1),
            Some(SelOp::Resize { anchor, corner, scale, free }) => {
                let (w, h) = (corner.0 - anchor.0, corner.1 - anchor.1);
                let (sx, sy) = (
                    if w.abs() > 0.1 { (px - anchor.0) / w } else { 1.0 },
                    if h.abs() > 0.1 { (py - anchor.1) / h } else { 1.0 },
                );
                *scale = if *free {
                    (sx.max(0.05), sy.max(0.05))
                } else {
                    // Uniform: project the pointer onto the box diagonal.
                    let len2 = w * w + h * h;
                    let s = if len2 > 0.0 { ((px - anchor.0) * w + (py - anchor.1) * h) / len2 } else { 1.0 };
                    (s.max(0.05), s.max(0.05))
                };
            }
            None => {}
        }
        drop(op);
        // The live preview is drawn outside the cached ink.
        self.queue_draw();
        if self.imp().sel_op.borrow().as_ref().is_some_and(SelOp::transforms) {
            let cache_hides = self.imp().cache.borrow().as_ref().is_some_and(|c| c.1);
            if !cache_hides {
                self.imp().cache.take();
            }
        }
    }

    fn end_select(&self, x: f32, y: f32) {
        let imp = self.imp();
        self.extend_select(x, y);
        let Some(op) = imp.sel_op.take() else { return };
        self.reset_cursor();
        match op {
            SelOp::Lasso { points, add } => {
                let extent = points.iter().fold((f32::MAX, f32::MIN, f32::MAX, f32::MIN), |a, p| {
                    (a.0.min(p.0), a.1.max(p.0), a.2.min(p.1), a.3.max(p.1))
                });
                let tap = (extent.1 - extent.0).max(extent.3 - extent.2) * self.zoom() < 6.0;
                let strokes = imp.strokes.borrow().clone();
                let picked: Strokes = if tap {
                    let (px, py) = points[0];
                    let r = 6.0 / self.zoom();
                    strokes.iter().rev().find(|s| s.hit(px, py, r)).cloned().into_iter().collect()
                } else {
                    // A stroke is in if most of it is inside the loop.
                    strokes
                        .iter()
                        .filter(|s| {
                            let n = s.points.iter().filter(|p| inside(&points, p.x, p.y)).count();
                            n * 10 >= s.points.len() * 6
                        })
                        .cloned()
                        .collect()
                };
                // Text boxes: tapped on, or centered inside the loop. A tap
                // on a stroke wins over the box under it.
                let texts = imp.texts.borrow().clone();
                let picked_texts: Texts = if tap {
                    let (px, py) = points[0];
                    if picked.is_empty() {
                        texts.iter().rev().find(|t| t.contains(px, py, 0.0)).cloned().into_iter().collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    texts
                        .iter()
                        .filter(|t| {
                            let b = t.bounds();
                            inside(&points, (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
                        })
                        .cloned()
                        .collect()
                };
                merge_selection(&mut imp.selection.borrow_mut(), picked, add, tap);
                merge_selection(&mut imp.sel_texts.borrow_mut(), picked_texts, add, tap);
                self.log(format!(
                    "select: {} strokes, {} texts",
                    imp.selection.borrow().len(),
                    imp.sel_texts.borrow().len()
                ));
            }
            op => {
                let ctx = self.pango_context();
                self.edit_selected(|s| transformed(s, &op), |t| transformed_text(t, &op, &ctx));
            }
        }
        imp.cache.take();
        self.queue_draw();
    }

    /// Replace the selected strokes and text boxes with edited copies (one
    /// undo step) and select the copies.
    fn edit_selected(&self, stroke_edit: impl Fn(&Stroke) -> Stroke, text_edit: impl Fn(&TextBox) -> TextBox) {
        let imp = self.imp();
        let (sel, sel_texts) = (imp.selection.borrow().clone(), imp.sel_texts.borrow().clone());
        if sel.is_empty() && sel_texts.is_empty() {
            return;
        }
        let before = self.snap();
        let (mut new_sel, mut new_texts) = (Vec::new(), Vec::new());
        let strokes = before
            .0
            .iter()
            .map(|s| {
                if sel.iter().any(|x| Rc::ptr_eq(x, s)) {
                    let n = Rc::new(stroke_edit(s));
                    new_sel.push(n.clone());
                    n
                } else {
                    s.clone()
                }
            })
            .collect();
        let texts = before
            .1
            .iter()
            .map(|t| {
                if sel_texts.iter().any(|x| Rc::ptr_eq(x, t)) {
                    let n = Rc::new(text_edit(t));
                    new_texts.push(n.clone());
                    n
                } else {
                    t.clone()
                }
            })
            .collect();
        self.commit_all(before, (strokes, texts));
        *imp.selection.borrow_mut() = new_sel;
        *imp.sel_texts.borrow_mut() = new_texts;
    }

    /// Apply a pen's color and width to the selection. Returns false if
    /// nothing is selected.
    pub fn restyle_selection(&self, color: InkColor, width: f32) -> bool {
        // While typing, a pen's color colors the text box.
        if self.with_editor(|e| e.draft.color = color).is_some() {
            self.update_editor_css();
            self.focus_editor();
            return true;
        }
        if !self.has_selection() {
            return false;
        }
        self.edit_selected(
            |s| {
                let mut n = s.clone();
                n.color = color;
                n.width = width;
                n.update_bounds();
                n
            },
            |t| TextBox { color, ..t.clone() },
        );
        self.log("restyle selection".into());
        true
    }

    pub fn select_all(&self) {
        let imp = self.imp();
        *imp.selection.borrow_mut() = imp.strokes.borrow().clone();
        *imp.sel_texts.borrow_mut() = imp.texts.borrow().clone();
        self.queue_draw();
    }

    pub fn delete_selection(&self) {
        let imp = self.imp();
        let (sel, sel_texts) = (imp.selection.take(), imp.sel_texts.take());
        if sel.is_empty() && sel_texts.is_empty() {
            return;
        }
        let before = self.snap();
        let strokes = before.0.iter().filter(|s| !sel.iter().any(|x| Rc::ptr_eq(x, s))).cloned().collect();
        let texts = before.1.iter().filter(|t| !sel_texts.iter().any(|x| Rc::ptr_eq(x, t))).cloned().collect();
        self.commit_all(before, (strokes, texts));
        self.log(format!("delete {} strokes, {} texts", sel.len(), sel_texts.len()));
    }

    pub fn copy_selection(&self) {
        let imp = self.imp();
        if !self.has_selection() {
            return;
        }
        *imp.clipboard.borrow_mut() = imp.selection.borrow().iter().map(|s| (**s).clone()).collect();
        *imp.clipboard_texts.borrow_mut() = imp.sel_texts.borrow().iter().map(|t| (**t).clone()).collect();
        self.log("copy".into());
    }

    pub fn cut_selection(&self) {
        self.copy_selection();
        self.delete_selection();
    }

    /// Paste the clipboard centered on the pointer, or 20 px down-right of
    /// where it was copied when `at_pointer` is false.
    pub fn paste(&self, at_pointer: bool) {
        let imp = self.imp();
        let (clip, clip_texts) = (imp.clipboard.borrow().clone(), imp.clipboard_texts.borrow().clone());
        if (clip.is_empty() && clip_texts.is_empty()) || imp.path.borrow().is_none() {
            return;
        }
        let rcs: Vec<Rc<Stroke>> = clip.into_iter().map(Rc::new).collect();
        let b = [union_bounds(rcs.iter()), clip_texts.iter().map(text_bounds).reduce(merge_bounds)]
            .into_iter()
            .flatten()
            .reduce(merge_bounds)
            .expect("non-empty");
        let (dx, dy) = if at_pointer {
            let (x, y) = imp.pointer.get();
            let (px, py) = self.to_page(x, y);
            (px - (b.x0 + b.x1) / 2.0, py - (b.y0 + b.y1) / 2.0)
        } else {
            (20.0, 20.0)
        };
        let op = SelOp::Move { start: (0.0, 0.0), delta: (dx, dy) };
        let ctx = self.pango_context();
        let pasted: Strokes = rcs.iter().map(|s| Rc::new(transformed(s, &op))).collect();
        let pasted_texts: Texts = clip_texts.iter().map(|t| Rc::new(transformed_text(t, &op, &ctx))).collect();
        let before = self.snap();
        let mut after = before.clone();
        after.0.extend(pasted.iter().cloned());
        after.1.extend(pasted_texts.iter().cloned());
        self.commit_all(before, after);
        *imp.selection.borrow_mut() = pasted;
        *imp.sel_texts.borrow_mut() = pasted_texts;
        self.log("paste".into());
    }

    pub fn duplicate_selection(&self) {
        let imp = self.imp();
        let saved = (imp.clipboard.take(), imp.clipboard_texts.take());
        self.copy_selection();
        self.paste(false);
        *imp.clipboard.borrow_mut() = saved.0;
        *imp.clipboard_texts.borrow_mut() = saved.1;
    }

    /// Erase along the path from the last eraser position to screen (x, y).
    fn erase_to(&self, x: f32, y: f32) {
        let imp = self.imp();
        let mode = imp.config.borrow().eraser_mode;
        let r = self.eraser_radius_screen() / self.zoom();
        let (px, py) = self.to_page(x, y);
        let mut erasing = imp.erasing.borrow_mut();
        let Some(e) = erasing.as_mut() else { return };
        let (x0, y0) = e.last.unwrap_or((px, py));
        let dist = ((px - x0).powi(2) + (py - y0).powi(2)).sqrt();
        let steps = (dist / (r / 2.0)).ceil().max(1.0) as usize;
        let mut strokes = imp.strokes.borrow_mut();
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let (sx, sy) = (x0 + (px - x0) * t, y0 + (py - y0) * t);
            match mode {
                EraserMode::Stroke => {
                    let before = strokes.len();
                    strokes.retain(|s| !s.hit(sx, sy, r));
                    e.changed |= strokes.len() != before;
                }
                EraserMode::Partial => {
                    let mut out = Vec::with_capacity(strokes.len());
                    for s in strokes.drain(..) {
                        match s.split_by_eraser(sx, sy, r) {
                            Some(pieces) => {
                                e.changed = true;
                                out.extend(pieces.into_iter().map(Rc::new));
                            }
                            None => out.push(s),
                        }
                    }
                    *strokes = out;
                }
            }
        }
        e.last = Some((px, py));
        imp.eraser_at.set(Some((x, y)));
        drop(strokes);
        drop(erasing);
        imp.cache.take();
    }
}

// ---- radial menu -------------------------------------------------------------------

impl Canvas {
    pub fn connect_menu(&self, f: impl Fn(RadialItem) + 'static) {
        *self.imp().on_menu.borrow_mut() = Some(Box::new(f));
    }

    pub fn menu_open(&self) -> bool {
        self.imp().radial.borrow().is_some()
    }

    fn menu_hover(&self) -> Option<usize> {
        self.imp().radial.borrow().as_ref().and_then(|r| r.hover)
    }

    /// Close the menu and run the item under the pointer, if any.
    fn pick_radial(&self) {
        if let Some(item) = self.close_radial() {
            self.log(format!("menu: {item:?}"));
            if let Some(f) = self.imp().on_menu.borrow().as_ref() {
                f(item);
            }
        }
    }

    fn button_opens_menu(&self, button: u32) -> bool {
        let c = self.imp().config.borrow();
        match button {
            2 => c.button_2 == ButtonAction::Menu,
            3 => c.button_3 == ButtonAction::Menu,
            _ => false,
        }
    }

    /// Open the menu around screen (x, y), nudged to fit in the canvas.
    fn open_radial(&self, x: f32, y: f32) {
        let imp = self.imp();
        let (w, h) = (self.width() as f32, self.height() as f32);
        let (outer, _) = radial_geometry(imp.config.borrow().radial.len());
        let fit = |v: f32, max: f32| v.clamp(outer.min(max / 2.0), (max - outer).max(max / 2.0));
        *imp.radial.borrow_mut() = Some(Radial { center: (fit(x, w), fit(y, h)), hover: None });
        self.animate_radial(0.0, 1.0);
        self.grab_focus();
    }

    /// Close the menu; returns the item under the pointer, if any.
    fn close_radial(&self) -> Option<RadialItem> {
        let imp = self.imp();
        let radial = imp.radial.take()?;
        imp.radial_anim.take();
        imp.radial_open.set(0.0);
        self.queue_draw();
        let entries = &imp.config.borrow().radial;
        radial.hover.and_then(|i| entries.get(i)).map(|e| e.item).filter(|i| *i != RadialItem::None)
    }

    fn animate_radial(&self, from: f64, to: f64) {
        let weak = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            if let Some(canvas) = weak.upgrade() {
                canvas.imp().radial_open.set(v);
                canvas.queue_draw();
            }
        });
        let anim = adw::TimedAnimation::new(self, from, to, 140, target);
        anim.set_easing(adw::Easing::EaseOutBack);
        anim.play();
        *self.imp().radial_anim.borrow_mut() = Some(anim.upcast());
    }

    /// Which entry the pointer points at: by direction from the center, so
    /// a short flick is enough. Nothing within the center circle.
    fn radial_hover(&self, x: f32, y: f32) {
        let imp = self.imp();
        let count = imp.config.borrow().radial.len();
        let mut radial = imp.radial.borrow_mut();
        let Some(r) = radial.as_mut() else { return };
        let (dx, dy) = (x - r.center.0, y - r.center.1);
        let hover = if count == 0 || (dx * dx + dy * dy).sqrt() < RADIAL_CENTER {
            None
        } else {
            let angle = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU);
            let step = std::f32::consts::TAU / count as f32;
            let i = ((angle / step).round() as usize) % count;
            let empty = imp.config.borrow().radial[i].item == RadialItem::None;
            (!empty).then_some(i)
        };
        if r.hover != hover {
            r.hover = hover;
            drop(radial);
            self.queue_draw();
        }
    }

    // ---- long press -------------------------------------------------------------

    fn start_long_press(&self, x: f32, y: f32) {
        let imp = self.imp();
        self.cancel_long_press();
        let Some(delay) = imp.config.borrow().long_press else { return };
        let weak = self.downgrade();
        let timer = glib::timeout_add_local_once(delay, move || {
            if let Some(canvas) = weak.upgrade() {
                if let Some(lp) = canvas.imp().long_press.borrow_mut().as_mut() {
                    lp.timer = None;
                }
                canvas.long_press_fired();
            }
        });
        // Redraw every frame while waiting, for the filling ring.
        let tick = self.add_tick_callback(|canvas, _| {
            canvas.queue_draw();
            glib::ControlFlow::Continue
        });
        *imp.long_press.borrow_mut() =
            Some(LongPress { start: (x, y), began: Instant::now(), timer: Some(timer), tick: Some(tick) });
    }

    fn check_long_press(&self, x: f32, y: f32) {
        let moved = self.imp().long_press.borrow().as_ref().is_some_and(|lp| {
            let (dx, dy) = (x - lp.start.0, y - lp.start.1);
            (dx * dx + dy * dy).sqrt() > LONG_PRESS_SLOP
        });
        if moved {
            self.cancel_long_press();
        }
    }

    fn cancel_long_press(&self) {
        if let Some(lp) = self.imp().long_press.take() {
            if let Some(t) = lp.timer {
                t.remove();
            }
            if let Some(t) = lp.tick {
                t.remove();
            }
            self.queue_draw();
        }
    }

    /// Held still long enough: drop what the touch started (a dot, an
    /// erase, a lasso…) and open the menu there; lifting picks.
    fn long_press_fired(&self) {
        let imp = self.imp();
        let Some(lp) = imp.long_press.take() else { return };
        if let Some(t) = lp.tick {
            t.remove();
        }
        if let Some(e) = imp.erasing.take() {
            *imp.strokes.borrow_mut() = e.before;
            imp.cache.take();
        }
        imp.live.take();
        imp.sel_op.take();
        imp.panning.take();
        imp.text_press.take();
        imp.eraser_at.set(None);
        self.set_temp_tool(None);
        self.reset_cursor();
        self.log("long press: radial menu".into());
        self.open_radial(lp.start.0, lp.start.1);
        imp.action.set(Action::Menu);
    }

    fn draw_long_press(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let Some(delay) = imp.config.borrow().long_press else { return };
        let lp = imp.long_press.borrow();
        let Some(lp) = lp.as_ref() else { return };
        let elapsed = lp.began.elapsed();
        if elapsed < LONG_PRESS_RING_DELAY {
            return;
        }
        let t = ((elapsed - LONG_PRESS_RING_DELAY).as_secs_f32()
            / (delay.saturating_sub(LONG_PRESS_RING_DELAY)).as_secs_f32().max(0.01))
        .min(1.0);
        let accent = adw::StyleManager::default().accent_color_rgba();
        let b = gsk::PathBuilder::new();
        let (cx, cy, r) = (lp.start.0, lp.start.1, 18.0);
        b.move_to(cx, cy - r);
        // Arc as short segments (clockwise from the top).
        let steps = (48.0 * t).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let a = k as f32 / steps as f32 * t * std::f32::consts::TAU;
            b.line_to(cx + r * a.sin(), cy - r * a.cos());
        }
        let stroke = gsk::Stroke::new(3.0);
        stroke.set_line_cap(gsk::LineCap::Round);
        snapshot.append_stroke(&b.to_path(), &stroke, &accent);
    }

    fn draw_radial(&self, snapshot: &gtk::Snapshot) {
        let imp = self.imp();
        let radial = imp.radial.borrow();
        let Some(r) = radial.as_ref() else { return };
        let open = imp.radial_open.get() as f32;
        let dark = adw::StyleManager::default().is_dark();
        let bg = if dark { rgba((0.18, 0.18, 0.2), 0.97) } else { rgba((0.99, 0.99, 0.99), 0.97) };
        let fg = if dark { rgba((0.95, 0.95, 0.95), 1.0) } else { rgba((0.12, 0.12, 0.13), 1.0) };
        let accent = adw::StyleManager::default().accent_color_rgba();
        let circle = |cx: f32, cy: f32, rad: f32| gsk::RoundedRect::from_rect(rect(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad), rad);

        snapshot.save();
        snapshot.translate(&graphene::Point::new(r.center.0, r.center.1));
        snapshot.scale(0.6 + 0.4 * open, 0.6 + 0.4 * open);
        snapshot.push_opacity(open.clamp(0.0, 1.0) as f64);

        let (outer, ring) = radial_geometry(imp.config.borrow().radial.len());
        let disc = circle(0.0, 0.0, outer);
        snapshot.append_outset_shadow(&disc, &rgba((0.0, 0.0, 0.0), 0.3), 0.0, 3.0, 0.0, 12.0);
        snapshot.push_rounded_clip(&disc);
        snapshot.append_color(&bg, &rect(-outer, -outer, 2.0 * outer, 2.0 * outer));
        snapshot.pop();

        // Center: "close" mark.
        let center = circle(0.0, 0.0, RADIAL_CENTER);
        snapshot.push_rounded_clip(&center);
        let mut c = fg;
        c.set_alpha(if r.hover.is_none() { 0.14 } else { 0.07 });
        snapshot.append_color(&c, &rect(-RADIAL_CENTER, -RADIAL_CENTER, 2.0 * RADIAL_CENTER, 2.0 * RADIAL_CENTER));
        snapshot.pop();
        let x = gsk::PathBuilder::new();
        x.move_to(-6.0, -6.0);
        x.line_to(6.0, 6.0);
        x.move_to(6.0, -6.0);
        x.line_to(-6.0, 6.0);
        let mut dim = fg;
        dim.set_alpha(0.6);
        snapshot.append_stroke(&x.to_path(), &gsk::Stroke::new(1.6), &dim);

        let entries = imp.config.borrow().radial.clone();
        let n = entries.len().max(1) as f32;
        for (i, entry) in entries.iter().enumerate() {
            if entry.item == RadialItem::None {
                continue;
            }
            let a = i as f32 / n * std::f32::consts::TAU;
            let (ix, iy) = (ring * a.sin(), -ring * a.cos());
            let spot = circle(ix, iy, RADIAL_ITEM);
            if r.hover == Some(i) {
                snapshot.push_rounded_clip(&spot);
                let mut h = accent;
                h.set_alpha(0.3);
                snapshot.append_color(&h, &rect(ix - RADIAL_ITEM, iy - RADIAL_ITEM, 2.0 * RADIAL_ITEM, 2.0 * RADIAL_ITEM));
                snapshot.pop();
            }
            self.draw_entry_glyph(snapshot, &entry.look, ix, iy, &fg, dark);
        }
        snapshot.pop();
        snapshot.restore();
    }

    fn draw_entry_glyph(&self, snapshot: &gtk::Snapshot, look: &EntryLook, x: f32, y: f32, fg: &gdk::RGBA, dark: bool) {
        match look {
            EntryLook::Pen(color) => {
                let b = gsk::PathBuilder::new();
                b.add_circle(&graphene::Point::new(x, y), 8.0);
                snapshot.append_fill(&b.to_path(), gsk::FillRule::Winding, &rgba(color.resolve(dark), 1.0));
            }
            EntryLook::Highlighter(color) => {
                snapshot.append_color(&rgba(color.resolve(dark), 0.8), &rect(x - 9.0, y - 5.0, 18.0, 10.0));
            }
            EntryLook::Eraser => {
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x, y));
                snapshot.rotate(-45.0);
                let b = gsk::PathBuilder::new();
                b.add_rect(&rect(-9.0, -5.0, 18.0, 10.0));
                snapshot.append_stroke(&b.to_path(), &gsk::Stroke::new(1.5), fg);
                snapshot.append_color(fg, &rect(-9.0, -5.0, 6.0, 10.0));
                snapshot.restore();
            }
            EntryLook::Select => {
                let b = gsk::PathBuilder::new();
                b.add_circle(&graphene::Point::new(x, y), 8.0);
                let s = gsk::Stroke::new(1.5);
                s.set_dash(&[3.0, 2.0]);
                snapshot.append_stroke(&b.to_path(), &s, fg);
            }
            EntryLook::Hand => {
                let b = gsk::PathBuilder::new();
                for (dx, dy) in [(0.0f32, -1.0f32), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
                    let (tx, ty) = (x + dx * 9.0, y + dy * 9.0);
                    b.move_to(x, y);
                    b.line_to(tx, ty);
                    // Arrow head.
                    b.move_to(tx - dy * 3.0 - dx * 3.0, ty + dx * 3.0 - dy * 3.0);
                    b.line_to(tx, ty);
                    b.line_to(tx + dy * 3.0 - dx * 3.0, ty - dx * 3.0 - dy * 3.0);
                }
                let s = gsk::Stroke::new(1.6);
                s.set_line_cap(gsk::LineCap::Round);
                snapshot.append_stroke(&b.to_path(), &s, fg);
            }
            EntryLook::Icon(name) => {
                let Some(display) = gdk::Display::default() else { return };
                let icon = gtk::IconTheme::for_display(&display).lookup_icon(
                    name,
                    &[],
                    16,
                    self.scale_factor(),
                    gtk::TextDirection::None,
                    gtk::IconLookupFlags::empty(),
                );
                snapshot.save();
                snapshot.translate(&graphene::Point::new(x - 8.0, y - 8.0));
                icon.snapshot_symbolic(snapshot.upcast_ref::<gdk::Snapshot>(), 16.0, 16.0, &[*fg]);
                snapshot.restore();
            }
        }
    }
}

// ---- text editing --------------------------------------------------------------

const TAGS: [&str; 3] = ["bold", "italic", "underline"];

fn style_has(style: Style, tag: &str) -> bool {
    match tag {
        "bold" => style.bold,
        "italic" => style.italic,
        _ => style.underline,
    }
}

fn style_set(style: &mut Style, tag: &str, on: bool) {
    match tag {
        "bold" => style.bold = on,
        "italic" => style.italic = on,
        _ => style.underline = on,
    }
}

/// Read a text buffer back into styled runs.
fn buffer_runs(buffer: &gtk::TextBuffer) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut it = buffer.start_iter();
    while !it.is_end() {
        let mut style = Style::default();
        for tag in TAGS {
            let on = buffer.tag_table().lookup(tag).is_some_and(|t| it.has_tag(&t));
            style_set(&mut style, tag, on);
        }
        let ch = it.char();
        match runs.last_mut() {
            Some(r) if r.style == style => r.text.push(ch),
            _ => runs.push(Run { text: ch.to_string(), style }),
        }
        it.forward_char();
    }
    runs
}

impl Canvas {
    pub fn is_editing(&self) -> bool {
        self.imp().editor.borrow().is_some()
    }

    fn with_editor<R>(&self, f: impl FnOnce(&mut Editor) -> R) -> Option<R> {
        self.imp().editor.borrow_mut().as_mut().map(f)
    }

    /// Give keyboard focus back to the text being edited (after toolbar use).
    pub fn focus_editor(&self) {
        if let Some(view) = self.with_editor(|e| e.view.clone()) {
            view.grab_focus();
        }
    }

    /// Open an editor for `draft`, replacing `original` when committed.
    fn start_edit(&self, draft: TextBox, original: Option<Rc<TextBox>>) {
        self.commit_edit();
        let view = gtk::TextView::builder()
            .wrap_mode(if draft.width.is_some() { gtk::WrapMode::WordChar } else { gtk::WrapMode::None })
            .accepts_tab(false)
            .left_margin(0)
            .right_margin(0)
            .top_margin(0)
            .bottom_margin(0)
            .pixels_above_lines(0)
            .pixels_below_lines(0)
            .css_classes(["rt-text-editor"])
            .build();
        let buffer = view.buffer();
        let table = buffer.tag_table();
        table.add(&gtk::TextTag::builder().name("bold").weight(700).build());
        table.add(&gtk::TextTag::builder().name("italic").style(gtk::pango::Style::Italic).build());
        table.add(&gtk::TextTag::builder().name("underline").underline(gtk::pango::Underline::Single).build());
        for run in &draft.runs {
            let names: Vec<&str> = TAGS.into_iter().filter(|t| style_has(run.style, t)).collect();
            let mut end = buffer.end_iter();
            buffer.insert_with_tags_by_name(&mut end, &run.text, &names);
        }
        buffer.place_cursor(&buffer.end_iter());

        // Newly typed text takes the pending style (B/I/U toggled with
        // nothing selected), like in a word processor.
        let pending = Rc::new(Cell::new(Style::default()));
        let inserted: Rc<Cell<Option<(i32, i32)>>> = Rc::default();
        let ins = inserted.clone();
        buffer.connect_insert_text(move |_, iter, text| ins.set(Some((iter.offset(), text.chars().count() as i32))));
        let (p, weak) = (pending.clone(), self.downgrade());
        buffer.connect_changed(move |b| {
            if let Some((start, len)) = inserted.take() {
                let (a, z) = (b.iter_at_offset(start), b.iter_at_offset(start + len));
                for tag in TAGS {
                    if style_has(p.get(), tag) {
                        b.apply_tag_by_name(tag, &a, &z);
                    } else {
                        b.remove_tag_by_name(tag, &a, &z);
                    }
                }
            }
            // Auto-width boxes grow as you type.
            if let Some(canvas) = weak.upgrade() {
                canvas.queue_allocate();
            }
        });

        let keys = gtk::EventControllerKey::new();
        let weak = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(canvas) = weak.upgrade() else { return glib::Propagation::Proceed };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match (ctrl, key.to_lower()) {
                (false, gdk::Key::Escape) => {
                    // Finishing removes this view; do it after the event.
                    glib::idle_add_local_once(move || canvas.commit_edit());
                }
                (true, gdk::Key::b) => canvas.text_command(TextCommand::Bold),
                (true, gdk::Key::i) => canvas.text_command(TextCommand::Italic),
                (true, gdk::Key::u) => canvas.text_command(TextCommand::Underline),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        view.add_controller(keys);

        view.set_parent(self);
        *self.imp().editor.borrow_mut() = Some(Editor { view: view.clone(), draft, original, pending });
        self.update_editor_css();
        self.invalidate();
        self.queue_allocate();
        view.grab_focus();
        self.log("text: editing".into());
    }

    /// Font and color of the editor follow the box and the zoom.
    fn update_editor_css(&self) {
        let imp = self.imp();
        let Some((size, font, color)) = self.with_editor(|e| (e.draft.size, e.draft.font.clone(), e.draft.color)) else {
            return;
        };
        let dark = adw::StyleManager::default().is_dark();
        let (r, g, b) = color.resolve(dark);
        let px = size.px() * self.zoom();
        let family = font.as_deref().map(|f| {
            let desc = gtk::pango::FontDescription::from_string(f);
            desc.family().map(|s| s.to_string()).unwrap_or_else(|| f.to_string())
        });
        let family = family.unwrap_or_else(|| "Sans".into()).replace('"', "");
        imp.editor_css.load_from_string(&format!(
            "textview.rt-text-editor, textview.rt-text-editor > text {{ background: transparent; \
             color: rgb({}, {}, {}); font-family: \"{family}\"; font-size: {px:.2}px; font-weight: normal; }} \
             textview.rt-text-editor {{ outline: 1px dashed alpha(currentColor, 0.45); outline-offset: 3px; }}",
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
        ));
        self.queue_allocate();
    }

    /// Size the editor for its current text, measured with the same font,
    /// zoom and bold runs (GtkTextView doesn't report a useful natural
    /// size of its own, and measures before its zoomed font applies).
    fn editor_size(e: &Editor, zoom: f32) -> (i32, i32) {
        let buffer = e.view.buffer();
        let source = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let layout = e.view.create_pango_layout(Some(&source));
        let mut desc = e.draft.font_description();
        let px = e.draft.size.px() * zoom;
        desc.set_absolute_size(px as f64 * gtk::pango::SCALE as f64);
        layout.set_font_description(Some(&desc));
        let attrs = gtk::pango::AttrList::new();
        let mut at = 0u32;
        for run in buffer_runs(&buffer) {
            let end = at + run.text.len() as u32;
            if run.style.bold {
                let mut a = gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold);
                a.set_start_index(at);
                a.set_end_index(end);
                attrs.insert(a);
            }
            if run.style.italic {
                let mut a = gtk::pango::AttrInt::new_style(gtk::pango::Style::Italic);
                a.set_start_index(at);
                a.set_end_index(end);
                attrs.insert(a);
            }
            at = end;
        }
        layout.set_attributes(Some(&attrs));
        if let Some(w) = e.draft.width {
            layout.set_width((w * zoom * gtk::pango::SCALE as f32) as i32);
            layout.set_wrap(gtk::pango::WrapMode::WordChar);
        }
        let (tw, th) = layout.pixel_size();
        let line = (px * 1.35).ceil() as i32;
        let width = match e.draft.width {
            Some(w) => (w * zoom) as i32,
            // Room for the caret and the next character while typing.
            None => tw + (px * 0.8) as i32 + 4,
        };
        (width.max(line * 2), th.max(line) + 2)
    }

    /// Place the editor over its box (screen coordinates).
    fn allocate_editor(&self) {
        let imp = self.imp();
        let editor = imp.editor.borrow();
        let Some(e) = editor.as_ref() else { return };
        let v = imp.view.get();
        let (x, y) = ((e.draft.x - v.sx) * v.zoom, (e.draft.y - v.sy) * v.zoom);
        let (width, height) = Self::editor_size(e, v.zoom);
        // GTK wants children measured before they're allocated.
        let _ = e.view.measure(gtk::Orientation::Horizontal, -1);
        let _ = e.view.measure(gtk::Orientation::Vertical, width);
        let transform = gsk::Transform::new().translate(&graphene::Point::new(x, y));
        e.view.allocate(width, height, -1, Some(transform));
    }

    /// Whether screen (x, y) is inside the open editor.
    fn in_editor(&self, x: f32, y: f32) -> bool {
        self.with_editor(|e| e.view.compute_bounds(self).is_some_and(|b| b.contains_point(&graphene::Point::new(x, y))))
            .unwrap_or(false)
    }

    /// Close the editor, saving its text as one undo step (an emptied box
    /// is removed).
    pub fn commit_edit(&self) {
        let imp = self.imp();
        let Some(e) = imp.editor.take() else { return };
        let runs = buffer_runs(&e.view.buffer());
        e.view.unparent();
        let mut draft = e.draft;
        draft.runs = runs;
        draft.measure(&self.pango_context());
        let before = self.snap();
        let mut texts = before.1.clone();
        let index = e.original.as_ref().and_then(|o| texts.iter().position(|t| Rc::ptr_eq(t, o)));
        let same = |a: &TextBox, b: &TextBox| {
            serde_json::to_string(a).ok() == serde_json::to_string(b).ok()
        };
        match (index, draft.is_empty()) {
            (Some(i), true) => {
                texts.remove(i);
            }
            (Some(i), false) if !same(&texts[i], &draft) => texts[i] = Rc::new(draft),
            (None, false) => texts.push(Rc::new(draft)),
            _ => {
                self.invalidate();
                return;
            }
        }
        self.commit_all(before.clone(), (before.0, texts));
        self.log("text: saved".into());
        self.grab_focus();
    }

    /// Text tool released at screen (x, y): edit the box there, or start a
    /// new one (a drag sets its wrap width).
    fn text_tool_click(&self, x: f32, _y: f32) {
        let imp = self.imp();
        let Some((x0, y0)) = imp.text_press.take() else { return };
        let (px, py) = self.to_page(x0, y0);
        let hit = imp.texts.borrow().iter().rev().find(|t| t.contains(px, py, 4.0 / self.zoom())).cloned();
        if let Some(t) = hit {
            self.start_edit((*t).clone(), Some(t));
            return;
        }
        let (size, font, color) = {
            let c = imp.config.borrow();
            (c.text_size, c.text_font.clone(), c.brush.color)
        };
        let dx = x - x0;
        let (left, width) = if dx.abs() > 24.0 {
            (self.to_page(x0.min(x), y0).0, Some(dx.abs() / self.zoom()))
        } else {
            (px, None)
        };
        // Put the click in the middle of the first line.
        let top = py - size.px() * 0.6;
        self.start_edit(TextBox::new(left, top, width, size, font, color), None);
    }

    /// Size / font / B / I / U from the toolbar or shortcuts.
    pub fn text_command(&self, cmd: TextCommand) {
        let imp = self.imp();
        match cmd {
            TextCommand::Size(size) => {
                imp.config.borrow_mut().text_size = size;
                if self.with_editor(|e| e.draft.size = size).is_none() && !imp.sel_texts.borrow().is_empty() {
                    let ctx = self.pango_context();
                    self.edit_selected(|s| s.clone(), |t| {
                        let mut n = TextBox { size, ..t.clone() };
                        n.measure(&ctx);
                        n
                    });
                }
            }
            TextCommand::Font(font) => {
                imp.config.borrow_mut().text_font = font.clone();
                if self.with_editor(|e| e.draft.font = font.clone()).is_none() && !imp.sel_texts.borrow().is_empty() {
                    let ctx = self.pango_context();
                    self.edit_selected(|s| s.clone(), |t| {
                        let mut n = TextBox { font: font.clone(), ..t.clone() };
                        n.measure(&ctx);
                        n
                    });
                }
            }
            TextCommand::Bold | TextCommand::Italic | TextCommand::Underline => {
                let tag = match cmd {
                    TextCommand::Bold => "bold",
                    TextCommand::Italic => "italic",
                    _ => "underline",
                };
                self.with_editor(|e| {
                    let buffer = e.view.buffer();
                    match buffer.selection_bounds() {
                        Some((a, z)) => {
                            // Toggle: remove if all of the selection has it.
                            let t = buffer.tag_table().lookup(tag).expect("tag");
                            let mut it = a;
                            let mut all = true;
                            while it < z {
                                all &= it.has_tag(&t);
                                it.forward_char();
                            }
                            if all {
                                buffer.remove_tag(&t, &a, &z);
                            } else {
                                buffer.apply_tag(&t, &a, &z);
                            }
                        }
                        None => {
                            let mut p = e.pending.get();
                            let on = !style_has(p, tag);
                            style_set(&mut p, tag, on);
                            e.pending.set(p);
                        }
                    }
                });
            }
        }
        self.update_editor_css();
        self.invalidate();
        self.focus_editor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lasso_point_in_polygon() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(inside(&square, 5.0, 5.0));
        assert!(!inside(&square, 15.0, 5.0));
        // Concave "C" shape: the notch is outside.
        let c = [(0.0, 0.0), (10.0, 0.0), (10.0, 3.0), (3.0, 3.0), (3.0, 7.0), (10.0, 7.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(!inside(&c, 7.0, 5.0));
        assert!(inside(&c, 1.0, 5.0));
    }

    #[test]
    fn move_and_resize_transforms() {
        let mut s = Stroke::new(Tool::Pen, InkColor::Ink, 2.0);
        s.points = vec![Point { x: 10.0, y: 10.0, p: 0.5 }, Point { x: 20.0, y: 20.0, p: 0.5 }];
        s.update_bounds();
        let moved = transformed(&s, &SelOp::Move { start: (0.0, 0.0), delta: (5.0, -5.0) });
        assert_eq!((moved.points[0].x, moved.points[0].y), (15.0, 5.0));
        assert_eq!(moved.width, 2.0);
        let op = SelOp::Resize { anchor: (10.0, 10.0), corner: (20.0, 20.0), scale: (2.0, 2.0), free: false };
        let big = transformed(&s, &op);
        assert_eq!((big.points[1].x, big.points[1].y), (30.0, 30.0));
        assert_eq!(big.points[0].x, 10.0);
        assert_eq!(big.width, 4.0);
        assert!(big.bounds.x1 > 30.0);
    }
}
