use gpui_kit::{assets::IconName, component::ActiveTheme, prelude::FluentBuilder, *};

use crate::theme::{ICON_BUTTON_PADDING, ICON_SIZE_MEDIUM, ICON_SIZE_SMALL, RADIUS_SM};

pub enum IconSource {
    Path(SharedString),
    Named(IconName),
}

impl From<IconName> for IconSource {
    fn from(name: IconName) -> Self {
        Self::Named(name)
    }
}

impl From<&'static str> for IconSource {
    fn from(path: &'static str) -> Self {
        Self::Path(path.into())
    }
}

impl From<SharedString> for IconSource {
    fn from(path: SharedString) -> Self {
        Self::Path(path)
    }
}

impl IconSource {
    fn path(&self) -> SharedString {
        match self {
            Self::Path(path) => path.clone(),
            Self::Named(name) => name.path(),
        }
    }
}

// Button definition

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconSource,
    size: Pixels,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<IconSource>) -> Self {
        Self {
            id: id.into(),
            icon: icon.into(),
            size: ICON_SIZE_MEDIUM,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn small(mut self) -> Self {
        self.size = ICON_SIZE_SMALL;
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let side = self.size + ICON_BUTTON_PADDING * 2.0;

        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .size(side)
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .child(
                svg()
                    .path(self.icon.path())
                    .size(self.size),
            )
            .when_some(self.on_click, |this, handler| this.on_click(handler))
    }
}
