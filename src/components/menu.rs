use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::*;

/// A menu item that updates `entity` when clicked. Does nothing if the entity
/// was dropped while the menu was open.
pub fn entity_menu_item<T: 'static>(
    entity: &WeakEntity<T>,
    label: &'static str,
    icon: Option<IconName>,
    handler: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> PopupMenuItem {
    let entity = entity.clone();
    let item = PopupMenuItem::new(label).on_click(move |_, window, cx| {
        if let Err(error) = entity.update(cx, |this, cx| handler(this, window, cx)) {
            eprintln!("menu target dropped before its action ran: {error:#}");
        }
    });
    match icon {
        Some(icon) => item.icon(icon),
        None => item,
    }
}
