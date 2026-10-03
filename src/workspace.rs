use gpui::{
    ClickEvent, Context, Div, IntoElement, Render, SharedString, Window, div, prelude::*, px, rgb,
    svg,
};

const BACKGROUND: u32 = 0x282c33;
const PANEL_BACKGROUND: u32 = 0x2f343e;
const STATUS_BAR_BACKGROUND: u32 = 0x3b414d;
const BORDER: u32 = 0x464b57;
const HOVER: u32 = 0x363c46;
const ACTIVE: u32 = 0x454a56;
const TEXT: u32 = 0xdce0e5;
const MUTED_TEXT: u32 = 0xa9afbc;

const SIDE_PANEL_WIDTH: f32 = 260.0;
const BOTTOM_PANEL_HEIGHT: f32 = 220.0;

pub struct Workspace {
    left_panel_open: bool,
    right_panel_open: bool,
    bottom_panel_open: bool,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            left_panel_open: true,
            right_panel_open: false,
            bottom_panel_open: false,
        }
    }

    fn status_bar_button(
        &self,
        id: &'static str,
        icon: &'static str,
        is_active: bool,
        on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .p_1()
            .rounded_sm()
            .cursor_pointer()
            .when(is_active, |this| this.bg(rgb(ACTIVE)))
            .hover(|style| style.bg(rgb(HOVER)))
            .child(svg().path(icon).size_4().flex_none().text_color(rgb(TEXT)))
            .on_click(cx.listener(on_click))
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(28.0))
            .px_2()
            .border_t_1()
            .border_color(rgb(BORDER))
            .bg(rgb(STATUS_BAR_BACKGROUND))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.status_bar_button(
                        "toggle-left-panel",
                        "icons/panel-left.svg",
                        self.left_panel_open,
                        |this, _event, _window, cx| {
                            this.left_panel_open = !this.left_panel_open;
                            cx.notify();
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.status_bar_button(
                        "toggle-bottom-panel",
                        "icons/panel-bottom.svg",
                        self.bottom_panel_open,
                        |this, _event, _window, cx| {
                            this.bottom_panel_open = !this.bottom_panel_open;
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.status_bar_button(
                        "toggle-right-panel",
                        "icons/panel-right.svg",
                        self.right_panel_open,
                        |this, _event, _window, cx| {
                            this.right_panel_open = !this.right_panel_open;
                            cx.notify();
                        },
                        cx,
                    )),
            )
    }
}

fn panel(title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .overflow_hidden()
        .bg(rgb(PANEL_BACKGROUND))
        .child(
            div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(rgb(MUTED_TEXT))
                .child(title.into()),
        )
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.left_panel_open, |this| {
                        this.child(
                            panel("Left panel")
                                .w(px(SIDE_PANEL_WIDTH))
                                .h_full()
                                .border_r_1()
                                .border_color(rgb(BORDER)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().flex_1().min_h_0())
                            .when(self.bottom_panel_open, |this| {
                                this.child(
                                    panel("Bottom panel")
                                        .h(px(BOTTOM_PANEL_HEIGHT))
                                        .w_full()
                                        .border_t_1()
                                        .border_color(rgb(BORDER)),
                                )
                            }),
                    )
                    .when(self.right_panel_open, |this| {
                        this.child(
                            panel("Right panel")
                                .w(px(SIDE_PANEL_WIDTH))
                                .h_full()
                                .border_l_1()
                                .border_color(rgb(BORDER)),
                        )
                    }),
            )
            .child(self.status_bar(cx))
    }
}
