//! Configuration management for claudatui.
//!
//! Handles persistence and loading of user preferences including layout settings.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A named profile containing a set of workspace directories.
///
/// Profiles let you scope the sidebar to a subset of your projects.
/// When a profile is active, only its workspaces appear prominently;
/// everything else is collapsed under "Other."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEntry {
    /// Human-readable profile name (e.g., "Personal", "Work")
    pub name: String,
    /// Workspace directory prefixes for this profile
    pub workspaces: Vec<String>,
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Layout configuration
    #[serde(default)]
    pub layout: LayoutConfig,

    /// Whether dangerous mode (--dangerously-skip-permissions) is enabled
    #[serde(default = "default_dangerous_mode")]
    pub dangerous_mode: bool,

    /// Workspace directories for sidebar filtering (legacy).
    /// When non-empty and no profiles are defined, only projects under these paths
    /// appear as primary groups; everything else is placed in a collapsible "Other" section.
    /// Ignored when `profiles` is non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,

    /// Named profiles, each containing a set of workspace directories.
    /// When non-empty, takes precedence over `workspaces`.
    /// The active profile is selected at runtime; defaults to "All" (no filtering).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileEntry>,
}

fn default_dangerous_mode() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
            dangerous_mode: true,
            workspaces: Vec::new(),
            profiles: Vec::new(),
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
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let contents = serde_json::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(&path, contents)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    /// Check if any workspaces are configured (legacy path)
    pub fn has_workspaces(&self) -> bool {
        !self.workspaces.is_empty()
    }

    /// Check if any profiles are defined
    pub fn has_profiles(&self) -> bool {
        !self.profiles.is_empty()
    }

    /// Check if a project path falls under any configured workspace directory (prefix match)
    pub fn is_in_workspace(&self, project_path: &str) -> bool {
        self.workspaces
            .iter()
            .any(|ws| project_path.starts_with(ws.as_str()))
    }

    /// Get the path to the config file
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir().context("Could not find config directory")?;

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
    fn default_config_has_expected_layout_and_dangerous_mode() {
        let config = Config::default();
        assert_eq!(config.layout.sidebar_width_pct, 25);
        assert_eq!(config.layout.sidebar_position, SidebarPosition::Left);
        assert!(!config.layout.sidebar_minimized);
        assert!(config.dangerous_mode);
    }

    #[test]
    fn layout_validate_clamps_sidebar_width_to_valid_range() {
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
    fn sidebar_position_toggle_swaps_left_and_right() {
        assert_eq!(SidebarPosition::Left.toggle(), SidebarPosition::Right);
        assert_eq!(SidebarPosition::Right.toggle(), SidebarPosition::Left);
    }

    #[test]
    fn config_roundtrips_through_json_serialization() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.layout.sidebar_width_pct,
            config.layout.sidebar_width_pct
        );
    }

    #[test]
    fn workspaces_empty_by_default_and_skipped_in_serialization() {
        let config = Config::default();
        assert!(!config.has_workspaces());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("workspaces"));
    }

    #[test]
    fn workspaces_roundtrip_through_json() {
        let config = Config {
            workspaces: vec!["/Users/brandon/work".to_string()],
            ..Config::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("workspaces"));
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.workspaces, vec!["/Users/brandon/work"]);
    }

    #[test]
    fn is_in_workspace_uses_prefix_matching() {
        let config = Config {
            workspaces: vec![
                "/Users/brandon/work".to_string(),
                "/Users/brandon/personal".to_string(),
            ],
            ..Config::default()
        };
        assert!(config.is_in_workspace("/Users/brandon/work/project-a"));
        assert!(config.is_in_workspace("/Users/brandon/personal/claudatui"));
        assert!(!config.is_in_workspace("/Users/brandon/other/project"));
        assert!(!config.is_in_workspace("/tmp/random"));
    }

    #[test]
    fn is_in_workspace_returns_false_when_no_workspaces() {
        let config = Config::default();
        assert!(!config.is_in_workspace("/any/path"));
    }

    #[test]
    fn has_workspaces_returns_true_when_configured() {
        let config = Config {
            workspaces: vec!["/some/path".to_string()],
            ..Config::default()
        };
        assert!(config.has_workspaces());
    }

    #[test]
    fn profiles_empty_by_default_and_skipped_in_serialization() {
        let config = Config::default();
        assert!(!config.has_profiles());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("profiles"));
    }

    #[test]
    fn profiles_roundtrip_through_json() {
        let config = Config {
            profiles: vec![
                ProfileEntry {
                    name: "Personal".to_string(),
                    workspaces: vec!["/Users/brandon/personal".to_string()],
                },
                ProfileEntry {
                    name: "Work".to_string(),
                    workspaces: vec!["/Users/brandon/work".to_string()],
                },
            ],
            ..Config::default()
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("profiles"));
        assert!(json.contains("Personal"));
        assert!(json.contains("Work"));
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.profiles.len(), 2);
        assert_eq!(parsed.profiles[0].name, "Personal");
        assert_eq!(parsed.profiles[1].name, "Work");
        assert_eq!(
            parsed.profiles[0].workspaces,
            vec!["/Users/brandon/personal"]
        );
    }

    #[test]
    fn has_profiles_returns_true_when_configured() {
        let config = Config {
            profiles: vec![ProfileEntry {
                name: "Test".to_string(),
                workspaces: vec!["/test".to_string()],
            }],
            ..Config::default()
        };
        assert!(config.has_profiles());
    }

    #[test]
    fn backward_compat_old_config_without_profiles_deserializes() {
        let json = r#"{"dangerous_mode": true, "workspaces": ["/old/path"]}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.has_workspaces());
        assert!(!config.has_profiles());
        assert_eq!(config.workspaces, vec!["/old/path"]);
    }
}
