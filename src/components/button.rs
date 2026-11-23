use gpui_kit::base::Selectable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::RADIUS_BUTTON;

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonStyle {
    #[default]
    Subtle,
    Filled,
}

const HEIGHT: Pixels = px(22.);
const HORIZONTAL_PADDING: Pixels = px(4.);

#[derive(IntoElement)]
pub struct Button {
    base: Stateful<Div>,
    label: Option<SharedString>,
    label_size: Option<Pixels>,
    style: ButtonStyle,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            label: None,
            label_size: None,
            style: ButtonStyle::default(),
            selected: false,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label_size(mut self, size: Pixels) -> Self {
        self.label_size = Some(size);
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.base = self
            .base
            .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx));
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl DropdownMenu for Button {}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (background, hover, active, border, foreground) = match self.style {
            ButtonStyle::Subtle => (
                theme.transparent,
                theme.secondary_hover,
                theme.secondary_active,
                theme.transparent,
                theme.foreground,
            ),
            ButtonStyle::Filled => (
                theme.button,
                theme.button_hover,
                theme.button_active,
                theme.input,
                theme.button_foreground,
            ),
        };
        let background = if self.selected { active } else { background };

        self.base
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap_1()
            .h(HEIGHT)
            .px(HORIZONTAL_PADDING)
            .rounded(RADIUS_BUTTON)
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(
                self.label_size
                    .unwrap_or(theme.font_size),
            )
            .cursor_pointer()
            .hover(|style| style.bg(hover))
            .active(|style| style.bg(active))
            // Keeps a press from focusing the button or, in the title bar,
            // from starting a window drag.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .when_some(self.label, |this, label| this.child(label))
            .when_some(self.on_click, |this, handler| this.on_click(handler))
    }
}
