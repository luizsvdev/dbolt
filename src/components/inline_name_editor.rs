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
    finished: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InlineNameEvent> for InlineNameEditor {}

impl InlineNameEditor {
    pub fn new(initial_name: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Like Zed's filename editor, the field takes the text style of the
        // row it sits in (a view renders inside its parent's text style), so
        // swapping the label for the field changes neither the text nor the
        // row height.
        let text_style = window.text_style();
        let line_height = window.line_height();

        div()
            .flex_1()
            .min_w_0()
            .h(line_height)
            // The input's frame ends up taller than the line it is given, so
            // it is centred here; placed from the top, its text sat 2-3px
            // below the label's.
            .flex()
            .items_center()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.finish(InlineNameEvent::Cancel, cx);
                }
            }))
            // No border of its own: the row being edited shows the focus
            // border instead.
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
                    .px_0()
                    .py_0()
                    .h(line_height)
                    .line_height(text_style.line_height)
                    .text_size(text_style.font_size)
                    .font_family(text_style.font_family.clone())
                    .font_weight(text_style.font_weight),
            )
    }
}
