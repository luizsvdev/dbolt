use gpui_kit::{FontWeight, Pixels, px};

/// Zed's UI font (".ZedSans" is IBM Plex Sans) and its default UI size.
pub const UI_FONT_FAMILY: &str = "IBM Plex Sans";
pub const PANEL_FONT_SIZE: Pixels = px(16.0);
pub const PANEL_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

/// Matches Zed's buttons (`rounded_sm`).
pub const RADIUS_BUTTON: Pixels = px(4.0);
/// Zed's `LabelSize::Small`, used for title bar buttons.
pub const LABEL_SIZE_SMALL: Pixels = px(12.0);
pub const ICON_SIZE_SMALL: Pixels = px(14.0);
pub const ICON_SIZE_MEDIUM: Pixels = px(16.0);
pub const PANEL_HEADER_HEIGHT: Pixels = px(32.0);
