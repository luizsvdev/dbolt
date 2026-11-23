use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;

pub enum InlineNameEvent {
    /// The trimmed, non-empty name the user entered.
    Confirm(String),
    Cancel,
}

/// A name field shown in place of a tree row, for creating or renaming an
/// item in the panel itself. Enter or losing focus confirms; Escape or an
/// empty name cancels.
pub struct InlineNameEditor {
    input: Entity<InputState>,
    text_size: Pixels,
    finished: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InlineNameEvent> for InlineNameEditor {}

impl InlineNameEditor {
    /// `text_size` is the tree's own, so the field matches the other rows.
    pub fn new(
        initial_name: &str,
        text_size: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial_name));
        // Deferred because the editor usually opens from a menu, which hands
        // focus back to its trigger as it closes; focusing now would lose to
        // that and blur (and so end) the edit at once.
        window.defer(cx, {
            let input = input.clone();
            move |window, cx| {
                input.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.select_all(window, cx);
                })
            }
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
                    InputEvent::PressEnter { .. } | InputEvent::Blur => this.confirm(cx),
                    InputEvent::Change | InputEvent::Focus => {}
                }),
            ];
        Self {
            input,
            text_size,
            finished: false,
            _subscriptions: subscriptions,
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let name = self
            .input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            self.finish(InlineNameEvent::Cancel, cx);
        } else {
            self.finish(InlineNameEvent::Confirm(name), cx);
        }
    }

    /// Emits once: confirming with Enter also blurs the input, which must not
    /// confirm a second time.
    fn finish(&mut self, event: InlineNameEvent, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;
        cx.emit(event);
    }
}

impl Render for InlineNameEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.finish(InlineNameEvent::Cancel, cx);
                }
            }))
            // No border of its own: like Zed's filename editor, the row being
            // edited shows the focus border instead.
            // The input's default size is a 32px control with smaller text;
            // auto height and the tree's text size keep the row as tall as
            // its neighbours.
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .px_0()
                    .py_0()
                    .h_auto()
                    .text_size(self.text_size),
            )
    }
}
