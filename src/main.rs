mod workspace;

use gpui::{App, AppContext, WindowOptions};
use workspace::Workspace;

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        if let Err(error) = cx.open_window(WindowOptions::default(), |_window, cx| {
            cx.new(|_cx| Workspace::new())
        }) {
            eprintln!("failed to open window: {error:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
