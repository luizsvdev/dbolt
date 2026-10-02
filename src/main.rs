use gpui::*;

struct Workspace;

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x1e1e2e))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        if let Err(error) =
            cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_cx| Workspace))
        {
            eprintln!("failed to open window: {error:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
