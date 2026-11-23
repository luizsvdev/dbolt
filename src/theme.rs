use std::sync::Arc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, FontFeatures, FontWeight, Hsla, Pixels, px};

pub const UI_FONT_FAMILY: &str = "IBM Plex Sans";
pub const PANEL_FONT_SIZE: Pixels = px(16.0);
pub const PANEL_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;

pub const EDITOR_FONT_FAMILY: &str = "Lilex";
pub const EDITOR_FONT_SIZE: Pixels = px(15.0);
pub const EDITOR_FONT_WEIGHT: FontWeight = FontWeight::NORMAL;
pub const EDITOR_LINE_HEIGHT: f32 = 1.618;

pub const RADIUS_BUTTON: Pixels = px(4.0);
pub const LABEL_SIZE_SMALL: Pixels = px(12.0);
pub const ICON_SIZE_XSMALL: Pixels = px(12.0);
pub const ICON_SIZE_SMALL: Pixels = px(14.0);
pub const ICON_SIZE_MEDIUM: Pixels = px(16.0);
pub const PANEL_HEADER_HEIGHT: Pixels = px(32.0);

pub fn editor_font_features() -> FontFeatures {
    FontFeatures(Arc::new(vec![("calt".to_string(), 1)]))
}

/// The editor paints this only behind its line numbers, so the text area and
/// the surfaces joined to it must paint it themselves.
pub fn editor_background(cx: &App) -> Hsla {
    let theme = cx.theme();
    theme
        .highlight_theme
        .style
        .editor_background
        .unwrap_or(theme.background)
}
