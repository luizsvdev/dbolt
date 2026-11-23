use gpui_kit::base::Selectable;
use gpui_kit::component::IconName as ComponentIconName;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{assets::IconName, component::ActiveTheme, prelude::FluentBuilder, *};

use crate::components::button::ClickHandler;
use crate::theme::{ICON_SIZE_MEDIUM, ICON_SIZE_SMALL, RADIUS_BUTTON};

pub enum IconSource {
    Path(SharedString),
    Named(IconName),
}

impl From<IconName> for IconSource {
    fn from(name: IconName) -> Self {
        Self::Named(name)
    }
}

impl From<ComponentIconName> for IconSource {
    fn from(name: ComponentIconName) -> Self {
        Self::Path(gpui_kit::assets::IconNamed::path(name))
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

#[derive(IntoElement)]
pub struct IconButton {
    base: Stateful<Div>,
    icon: IconSource,
    size: Pixels,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: impl Into<IconSource>) -> Self {
        Self {
            base: div().id(id),
            icon: icon.into(),
            size: ICON_SIZE_MEDIUM,
            selected: false,
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

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.base = self
            .base
            .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx));
        self
    }

    pub fn small(mut self) -> Self {
        self.size = ICON_SIZE_SMALL;
        self
    }
}

impl Styled for IconButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for IconButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl Selectable for IconButton {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl DropdownMenu for IconButton {}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let icon_color = cx.theme().foreground;
        let hover = cx.theme().accent;

        self.base
            .flex()
            .items_center()
            .justify_center()
            // Zed pads icon buttons by 2px, making a 20px square for a 16px icon.
            .p(px(2.))
            .rounded(RADIUS_BUTTON)
            .cursor_pointer()
            .when(self.selected, |this| this.bg(hover))
            .hover(|style| style.bg(hover))
            // Keeps a press from focusing the button or, in the title bar,
            // from starting a window drag.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .child(
                svg()
                    .path(self.icon.path())
                    .size(self.size)
                    .text_color(icon_color)
                    .flex_shrink_0(),
            )
            .when_some(self.on_click, |this, handler| this.on_click(handler))
    }
}
