//! Tab bar configuration.

use alacritty_config_derive::ConfigDeserialize;
use serde::Serialize;

use crate::display::color::Rgb;

/// Tab bar configuration.
#[derive(ConfigDeserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TabsConfig {
    /// Whether custom tabs are enabled.
    ///
    /// On macOS, this defaults to `false` and native OS tabs are used.
    /// Setting this to `true` on macOS uses the custom tab bar instead.
    /// On Linux/Windows, this defaults to `true` for custom tab bar rendering.
    ///
    /// Changing this option requires a restart to take effect.
    #[config(alias = "enable")]
    pub enabled: bool,

    /// Position of the tab bar.
    pub position: TabPosition,

    /// Tab bar appearance.
    pub indicator: TabIndicator,
}

#[allow(clippy::derivable_impls)] // Platform-conditional default for `enabled`.
impl Default for TabsConfig {
    fn default() -> Self {
        Self {
            // Only enable by default on non-macOS platforms
            #[cfg(target_os = "macos")]
            enabled: false,
            #[cfg(not(target_os = "macos"))]
            enabled: true,
            position: TabPosition::default(),
            indicator: TabIndicator::default(),
        }
    }
}

/// Position of the tab bar.
#[derive(ConfigDeserialize, Serialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPosition {
    /// Tab bar at the top of the window.
    #[default]
    Top,
}

/// Tab bar indicator/appearance configuration.
#[derive(ConfigDeserialize, Serialize, Clone, Debug, PartialEq)]
#[derive(Default)]
pub struct TabIndicator {
    /// Background color for the tab bar.
    pub background: Option<Rgb>,

    /// Background color for the active tab.
    pub active_background: Option<Rgb>,

    /// Foreground (text) color for tabs.
    pub foreground: Option<Rgb>,

    /// Foreground color for the active tab.
    pub active_foreground: Option<Rgb>,
}

