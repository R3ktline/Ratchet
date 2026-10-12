//! App preferences, stored in `<config dir>/ratchet/settings.json`.

use std::fs;
use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::ink::{InkColor, Tool};
use crate::text::TextSize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    pub fn apply(self) {
        let scheme = match self {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        };
        adw::StyleManager::default().set_color_scheme(scheme);
    }
}

/// A saved pen in the toolbar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PenSlot {
    pub tool: Tool,
    pub color: InkColor,
    pub width_mm: f32,
}

impl PenSlot {
    /// Fine / Medium / Thick widths in mm.
    pub fn presets(tool: Tool) -> [f32; 3] {
        match tool {
            Tool::Pen => [0.35, 0.7, 1.4],
            Tool::Highlighter => [3.0, 6.0, 10.0],
        }
    }

    /// Slider range in mm.
    pub fn range(tool: Tool) -> (f64, f64) {
        match tool {
            Tool::Pen => (0.1, 3.0),
            Tool::Highlighter => (1.0, 20.0),
        }
    }

    pub fn new(tool: Tool, color: InkColor) -> Self {
        PenSlot { tool, color, width_mm: Self::presets(tool)[1] }
    }
}

fn default_pens() -> Vec<PenSlot> {
    vec![
        PenSlot::new(Tool::Pen, InkColor::Ink),
        PenSlot::new(Tool::Pen, InkColor::Rgb(0x1c, 0x71, 0xd8)),
        PenSlot::new(Tool::Pen, InkColor::Rgb(0xe0, 0x1b, 0x24)),
        PenSlot::new(Tool::Pen, InkColor::Rgb(0x26, 0xa2, 0x69)),
        PenSlot::new(Tool::Highlighter, InkColor::Rgb(0xf6, 0xd3, 0x2d)),
        PenSlot::new(Tool::Highlighter, InkColor::Rgb(0x8f, 0xf0, 0xa4)),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EraserMode {
    /// Touching a stroke removes all of it.
    #[default]
    Stroke,
    /// Cuts strokes where the eraser passes.
    Partial,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Smoothing {
    Off,
    Low,
    #[default]
    Medium,
    High,
}

impl Smoothing {
    pub const ALL: [Smoothing; 4] = [Smoothing::Off, Smoothing::Low, Smoothing::Medium, Smoothing::High];

    /// Live stabilizer string length, in mm of on-screen movement.
    pub fn radius_mm(self) -> f32 {
        match self {
            Smoothing::Off => 0.0,
            Smoothing::Low => 0.5,
            Smoothing::Medium => 1.0,
            Smoothing::High => 2.0,
        }
    }

    /// Curve-fitting passes applied when the pen lifts.
    pub fn passes(self) -> usize {
        match self {
            Smoothing::Off => 0,
            Smoothing::Low => 1,
            Smoothing::Medium => 2,
            Smoothing::High => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Smoothing::Off => "Off",
            Smoothing::Low => "Low",
            Smoothing::Medium => "Medium",
            Smoothing::High => "High",
        }
    }
}

/// What a pen input (held side button, or the eraser end) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction {
    Eraser,
    Pan,
    Select,
    /// Draw with the current pen, whatever tool is selected.
    Draw,
    /// Open the radial menu at the pen.
    Menu,
    /// Leave it to the system (a normal middle/right click).
    Click,
}

impl ButtonAction {
    pub const ALL: [ButtonAction; 6] = [
        ButtonAction::Menu,
        ButtonAction::Eraser,
        ButtonAction::Pan,
        ButtonAction::Select,
        ButtonAction::Draw,
        ButtonAction::Click,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ButtonAction::Eraser => "Eraser",
            ButtonAction::Pan => "Pan",
            ButtonAction::Select => "Select",
            ButtonAction::Draw => "Draw",
            ButtonAction::Menu => "Radial menu",
            ButtonAction::Click => "Normal click",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "index")]
pub enum ActiveTool {
    Slot(usize),
    Eraser,
    Select,
    Text,
    /// Pen/mouse drag the page.
    Hand,
}

/// An entry of the radial menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadialItem {
    /// A pen slot by index.
    Slot(usize),
    Eraser,
    Select,
    Text,
    Hand,
    Undo,
    Redo,
    NewPage,
    Paste,
    ZoomReset,
    Fullscreen,
    None,
}

impl RadialItem {
    /// Everything that can be put in the menu (slots are added separately).
    pub const ACTIONS: [RadialItem; 11] = [
        RadialItem::Eraser,
        RadialItem::Select,
        RadialItem::Text,
        RadialItem::Hand,
        RadialItem::Undo,
        RadialItem::Redo,
        RadialItem::NewPage,
        RadialItem::Paste,
        RadialItem::ZoomReset,
        RadialItem::Fullscreen,
        RadialItem::None,
    ];

    /// Theme icon for items that have one (pens/eraser/lasso/hand are
    /// drawn by hand, see glyphs.rs).
    pub fn icon_name(self) -> Option<&'static str> {
        Some(match self {
            RadialItem::Text => "insert-text-symbolic",
            RadialItem::Undo => "edit-undo-symbolic",
            RadialItem::Redo => "edit-redo-symbolic",
            RadialItem::NewPage => "document-new-symbolic",
            RadialItem::Paste => "edit-paste-symbolic",
            RadialItem::ZoomReset => "zoom-original-symbolic",
            RadialItem::Fullscreen => "view-fullscreen-symbolic",
            _ => return None,
        })
    }

    pub fn label(self) -> String {
        match self {
            RadialItem::Slot(i) => format!("Pen slot {}", i + 1),
            RadialItem::Eraser => "Eraser".into(),
            RadialItem::Select => "Select".into(),
            RadialItem::Text => "Text".into(),
            RadialItem::Hand => "Pan (hand tool)".into(),
            RadialItem::Undo => "Undo".into(),
            RadialItem::Redo => "Redo".into(),
            RadialItem::NewPage => "New page".into(),
            RadialItem::Paste => "Paste".into(),
            RadialItem::ZoomReset => "Zoom to 100%".into(),
            RadialItem::Fullscreen => "Fullscreen".into(),
            RadialItem::None => "(empty)".into(),
        }
    }
}

/// How many items the radial menu can hold; the ring spreads them evenly.
pub const RADIAL_MIN: usize = 2;
pub const RADIAL_MAX: usize = 12;

/// Default radial menu, clockwise from the top.
pub fn default_radial(pens: &[PenSlot]) -> Vec<RadialItem> {
    let pen = |n: usize| pens.iter().enumerate().filter(|(_, p)| p.tool == Tool::Pen).nth(n).map(|(i, _)| i);
    let hl = pens.iter().position(|p| p.tool == Tool::Highlighter);
    [
        pen(0).map(RadialItem::Slot),
        Some(RadialItem::Hand),
        Some(RadialItem::Eraser),
        Some(RadialItem::Undo),
        Some(RadialItem::Redo),
        Some(RadialItem::Select),
        hl.map(RadialItem::Slot),
        pen(1).map(RadialItem::Slot),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Unit for custom page sizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    #[default]
    Mm,
    Cm,
    In,
}

impl LengthUnit {
    pub const ALL: [LengthUnit; 3] = [LengthUnit::Mm, LengthUnit::Cm, LengthUnit::In];

    pub fn label(self) -> &'static str {
        match self {
            LengthUnit::Mm => "mm",
            LengthUnit::Cm => "cm",
            LengthUnit::In => "in",
        }
    }

    pub fn mm_per_unit(self) -> f64 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::Cm => 10.0,
            LengthUnit::In => 25.4,
        }
    }
}

/// Bumped when defaults change in a way that should reach existing users.
const SETTINGS_VERSION: u32 = 4;

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Missing in old files, which must read as 0 to get migrated.
    #[serde(default)]
    pub version: u32,
    pub theme: Theme,
    /// Where notebooks live; `None` means the default location.
    pub library_dir: Option<PathBuf>,
    /// Last open page as `[notebook folder, section folder, page file]`.
    pub last_page: Option<[String; 3]>,
    /// Expanded sidebar items: notebook folders and "notebook/section" paths.
    pub expanded: Vec<String>,
    pub pens: Vec<PenSlot>,
    pub active_tool: ActiveTool,
    pub eraser_mode: EraserMode,
    pub smoothing: Smoothing,
    /// Pen side buttons, as GTK reports them (button 2 = middle, 3 = right).
    pub pen_button_2: ButtonAction,
    pub pen_button_3: ButtonAction,
    /// The pen's eraser mode (flipped pen or a driver-mapped button).
    pub pen_eraser: ButtonAction,
    pub custom_unit: LengthUnit,
    /// Defaults for new text boxes.
    pub text_size: TextSize,
    pub text_font: Option<String>,
    pub radial_items: Vec<RadialItem>,
    /// Holding the tip still this long opens the radial menu (None = off).
    pub long_press_ms: Option<u32>,
    /// Holding still at the end of a stroke this long snaps it to a shape.
    pub shape_hold_ms: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            theme: Theme::Dark,
            library_dir: None,
            last_page: None,
            expanded: Vec::new(),
            pens: default_pens(),
            active_tool: ActiveTool::Slot(0),
            eraser_mode: EraserMode::default(),
            smoothing: Smoothing::Low,
            pen_button_2: ButtonAction::Menu,
            pen_button_3: ButtonAction::Pan,
            pen_eraser: ButtonAction::Eraser,
            custom_unit: LengthUnit::default(),
            text_size: TextSize::default(),
            text_font: None,
            radial_items: default_radial(&default_pens()),
            long_press_ms: Some(600),
            shape_hold_ms: Some(500),
        }
    }
}

fn settings_path() -> PathBuf {
    glib::user_config_dir().join("ratchet").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        fs::read(settings_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .map(Self::migrate)
            .unwrap_or_default()
    }

    fn migrate(mut s: Settings) -> Settings {
        if s.version < 2 {
            // Pen button defaults changed: middle pans, right clicks.
            s.pen_button_2 = ButtonAction::Pan;
            s.pen_button_3 = ButtonAction::Click;
            s.pen_eraser = ButtonAction::Eraser;
        }
        if s.version < 3 {
            // The middle button opens the radial menu; the right one pans.
            s.pen_button_2 = ButtonAction::Menu;
            s.pen_button_3 = ButtonAction::Pan;
            s.radial_items = default_radial(&s.pens);
        }
        if s.version == 3 {
            // Long press got 40% shorter (older files never had one saved).
            s.long_press_ms = s.long_press_ms.map(|ms| ms * 6 / 10);
        }
        s.version = SETTINGS_VERSION;
        if s.pens.is_empty() {
            s.pens = default_pens();
        }
        // The menu has no empty spots any more: it shrinks to what's in it.
        s.radial_items.retain(|i| *i != RadialItem::None);
        s.radial_items.truncate(RADIAL_MAX);
        if s.radial_items.len() < RADIAL_MIN {
            s.radial_items = default_radial(&s.pens);
        }
        if matches!(s.active_tool, ActiveTool::Slot(i) if i >= s.pens.len()) {
            s.active_tool = ActiveTool::Slot(0);
        }
        s
    }

    pub fn save(&self) {
        let path = settings_path();
        let result = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&path, serde_json::to_vec_pretty(self).unwrap_or_default()));
        if let Err(e) = result {
            glib::g_warning!("ratchet", "Couldn't save settings to {}: {e}", path.display());
        }
    }

    pub fn library_dir(&self) -> PathBuf {
        self.library_dir.clone().unwrap_or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Documents)
                .unwrap_or_else(|| glib::home_dir().join("Documents"))
                .join("Ratchet")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_get_new_button_defaults() {
        let old = r#"{"pen_button_2":"eraser","pen_button_3":"pan"}"#;
        let mut s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.version, 0);
        s = Settings::migrate(s);
        assert_eq!((s.pen_button_2, s.pen_button_3, s.pen_eraser), (ButtonAction::Menu, ButtonAction::Pan, ButtonAction::Eraser));
        assert_eq!(s.radial_items[0], RadialItem::Slot(0));
        assert_eq!(s.radial_items[1], RadialItem::Hand);
        assert_eq!(s.radial_items[6], RadialItem::Slot(4));
        // Already migrated settings keep the user's choices.
        assert_eq!(s.long_press_ms, Some(600));
        let v3: Settings = serde_json::from_str(r#"{"version":3,"long_press_ms":1200}"#).unwrap();
        assert_eq!(Settings::migrate(v3).long_press_ms, Some(720));
        let mut s2: Settings = serde_json::from_str(r#"{"version":4,"pen_button_2":"select"}"#).unwrap();
        s2 = Settings::migrate(s2);
        assert_eq!(s2.pen_button_2, ButtonAction::Select);
    }
}
