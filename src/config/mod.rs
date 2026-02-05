//! Configuration management for claudatui.
//!
//! Handles persistence and loading of user preferences including layout settings.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Layout configuration
    #[serde(default)]
    pub layout: LayoutConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from disk, or return default if not found
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }

    /// Save configuration to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
        }

        let contents = serde_json::to_string_pretty(self)
            .context("Failed to serialize config")?;

        fs::write(&path, contents)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    /// Get the path to the config file
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not find config directory")?;

        Ok(config_dir.join("claudatui").join("config.json"))
    }
}

/// Layout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Sidebar width as percentage (10-50%)
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width_pct: u8,

    /// Sidebar position (left or right)
    #[serde(default)]
    pub sidebar_position: SidebarPosition,

    /// Whether sidebar is minimized
    #[serde(default)]
    pub sidebar_minimized: bool,
}

fn default_sidebar_width() -> u8 {
    25
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            sidebar_width_pct: 25,
            sidebar_position: SidebarPosition::Left,
            sidebar_minimized: false,
        }
    }
}

impl LayoutConfig {
    /// Validate and clamp sidebar width to valid range (10-50%)
    pub fn validate(&mut self) {
        self.sidebar_width_pct = self.sidebar_width_pct.clamp(10, 50);
    }
}

/// Sidebar position
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
    #[default]
    Left,
    Right,
}

impl SidebarPosition {
    /// Toggle between left and right
    pub fn toggle(&self) -> Self {
        match self {
            SidebarPosition::Left => SidebarPosition::Right,
            SidebarPosition::Right => SidebarPosition::Left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.layout.sidebar_width_pct, 25);
        assert_eq!(config.layout.sidebar_position, SidebarPosition::Left);
        assert!(!config.layout.sidebar_minimized);
    }

    #[test]
    fn test_layout_validate() {
        let mut layout = LayoutConfig {
            sidebar_width_pct: 5, // Below minimum
            ..Default::default()
        };
        layout.validate();
        assert_eq!(layout.sidebar_width_pct, 10);

        let mut layout = LayoutConfig {
            sidebar_width_pct: 75, // Above maximum
            ..Default::default()
        };
        layout.validate();
        assert_eq!(layout.sidebar_width_pct, 50);
    }

    #[test]
    fn test_sidebar_position_toggle() {
        assert_eq!(SidebarPosition::Left.toggle(), SidebarPosition::Right);
        assert_eq!(SidebarPosition::Right.toggle(), SidebarPosition::Left);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.layout.sidebar_width_pct, config.layout.sidebar_width_pct);
    }
}
