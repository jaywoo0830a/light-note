//! Settings: the little the app remembers between runs.
//!
//! JSON in `%APPDATA%\light-note\settings.json`.  Three properties matter and
//! all three are tested:
//!
//! * **forward compatible** — a field written by a newer version is ignored;
//! * **never fatal** — a truncated or nonsensical file falls back to defaults,
//!   because failing to start over a settings file would be absurd;
//! * **normalized on load** — a rate that is not one of 60/120/180/240 is
//!   snapped, and widths are clamped to something writable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::display::{RefreshChoice, snap_to_supported};
use crate::ink::Tool;

/// Everything the app persists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which frame rate to pace at.
    pub refresh: RefreshChoice,
    /// The tool selected at startup.
    pub tool: Tool,
    /// Pen width in pt.
    pub pen_width_pt: f32,
    /// Highlighter width in pt.
    pub highlighter_width_pt: f32,
    /// Eraser radius in pt.
    pub eraser_radius_pt: f32,
    /// Show the input/frame diagnostics panel.
    pub show_dev_panel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh: RefreshChoice::Auto,
            tool: Tool::Pen,
            pen_width_pt: Tool::Pen.default_width_pt(),
            highlighter_width_pt: Tool::Highlighter.default_width_pt(),
            eraser_radius_pt: Tool::Eraser.default_width_pt(),
            show_dev_panel: false,
        }
    }
}

/// Settings could not be written (reading never fails — it falls back).
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not write the settings file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize settings: {0}")]
    Json(#[from] serde_json::Error),
}

impl Settings {
    /// Reads settings from JSON, falling back to defaults when the text is
    /// damaged or the wrong shape.
    pub fn from_json(json: &str) -> Result<Self, SettingsError> {
        let mut settings: Self = serde_json::from_str(json).unwrap_or_default();
        settings.normalize();
        Ok(settings)
    }

    /// The settings as JSON.
    pub fn to_json(&self) -> Result<String, SettingsError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Loads `settings.json` from the app's directory (defaults when missing).
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// Loads from a specific path (a file that does not exist means defaults).
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(json) => Self::from_json(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Writes `settings.json` into the app's directory.
    pub fn save(&self) -> Result<(), SettingsError> {
        self.save_to(&Self::path())
    }

    /// Writes to a specific path, creating the directory if needed.
    pub fn save_to(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let json = self.to_json()?;
        std::fs::write(path, json).map_err(|source| SettingsError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// `%APPDATA%\light-note` — where the app's own files live.
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("light-note")
    }

    /// `%APPDATA%\light-note\settings.json`.
    pub fn path() -> PathBuf {
        Self::dir().join("settings.json")
    }

    /// `%APPDATA%\light-note\debug.log` — the trace the "Log" button writes.
    ///
    /// It sits next to `settings.json` so there is one place to look, and it is
    /// **not** a settings field: a log is per-run evidence, not a preference.
    pub fn debug_path() -> PathBuf {
        Self::dir().join("debug.log")
    }

    /// Snaps and clamps everything that has a legal range.
    pub fn normalize(&mut self) {
        self.refresh = match self.refresh {
            RefreshChoice::Auto => RefreshChoice::Auto,
            RefreshChoice::Fixed(hz) => RefreshChoice::Fixed(snap_to_supported(hz)),
        };
        self.pen_width_pt = clamp(self.pen_width_pt, 0.5, 40.0, 2.0);
        self.highlighter_width_pt = clamp(self.highlighter_width_pt, 1.0, 80.0, 14.0);
        self.eraser_radius_pt = clamp(self.eraser_radius_pt, 1.0, 120.0, 12.0);
    }

    /// The nib width of `tool` according to these settings.
    pub fn width_for(&self, tool: Tool) -> f32 {
        match tool {
            Tool::Pen => self.pen_width_pt,
            Tool::Highlighter => self.highlighter_width_pt,
            Tool::Eraser => self.eraser_radius_pt,
        }
    }
}

fn clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}