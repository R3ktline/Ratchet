//! App preferences, stored in `<config dir>/ratchet/settings.json`.

use std::fs;
use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Where notebooks live; `None` means the default location.
    pub library_dir: Option<PathBuf>,
    /// Last open page as `[notebook folder, section folder, page file]`.
    pub last_page: Option<[String; 3]>,
    /// Expanded sidebar items: notebook folders and "notebook/section" paths.
    pub expanded: Vec<String>,
}

fn settings_path() -> PathBuf {
    glib::user_config_dir().join("ratchet").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        fs::read(settings_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
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
