mod assets;
mod components;
mod project;
mod query_execution;
mod sql_statements;
mod theme;
mod ui;
mod util;
mod workspace;

use std::path::PathBuf;

use gpui_kit::{
    component::{Theme, ThemeRegistry, TitleBar},
    *,
};

use theme::{RADIUS_BUTTON, UI_FONT_FAMILY};
use workspace::Workspace;

fn load_fonts(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_family = UI_FONT_FAMILY.into();
        theme.font_size = rems(0.8).to_pixels(px(16.));
        theme.mono_font_family = "Lilex".into();
        theme.mono_font_size = rems(1.).to_pixels(px(16.));
        theme.radius = RADIUS_BUTTON;
    });
}

fn load_theme(cx: &mut App) {
    let theme_name = SharedString::from("One Dark");

    if let Err(error) = ThemeRegistry::watch_dir(PathBuf::from("./themes"), cx, move |cx| {
        if let Some(theme) = ThemeRegistry::global(cx)
            .themes()
            .get(&theme_name)
            .cloned()
        {
            Theme::update(cx, |current| {
                current.apply_config(&theme);
                current.radius = RADIUS_BUTTON;
            });
        }
    }) {
        eprintln!("failed to load themes: {error:#}");
    }
}

fn main() {
    application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            init(cx);
            ui::editor_panel::init(cx);
            load_fonts(cx);
            load_theme(cx);

            open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("DBolt".into()),
                        ..TitleBar::title_bar_options()
                    }),
                    app_id: Some("io.github.luizsvdev.dbolt".into()),
                    app_owns_titlebar_drag: true,
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Workspace::new(window, cx)),
            )
            .expect("Failed to open window");
        });
}
