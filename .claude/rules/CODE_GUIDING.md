# Code guiding

Lessons from building the workspace foundation (phases A and B). Each one cost extra rounds of fixes, so apply them on the first attempt.

## Copying Zed's UI

* When the owner asks for something "like Zed", read the matching code in the `zed-industries/zed` repository before writing anything, and copy the **logic** (how state, styles and components fit together), not just the numbers. Copying pixel values without the structure behind them caused the rename flicker to come back three times.
* Name the Zed file you followed in your reply (for example `crates/project_panel/src/project_panel.rs`), never in code comments.
* Change only what was asked. If a fix needs another area to change (for example a panel's font size), ask first. An unrequested size change in the connections panel had to be rolled back.

## Sizes and text style

* The app's root size is 12.8px, so `rems(1.0)` and `.text_base()` are 12.8px, not 16px. Check what a size actually resolves to before relying on it.
* A child that swaps in for another element (label ↔ editor) must **inherit** the parent's text style (`window.text_style()`, `window.line_height()`) instead of using its own constants. Same font family, size, weight and line height means no jump.
* Rows that change state keep a fixed height. Never let an edit, a focus or a selection change a row's box size.
* Use the constants in `src/theme.rs` (`RADIUS_BUTTON`, `LABEL_SIZE_SMALL`, `ICON_SIZE_SMALL`, ...) instead of new magic numbers. Add a constant there when a value is shared.

## Focus and borders

* Borders that toggle are always present and only change colour (transparent ↔ `theme.ring`). Adding a border on focus adds 1px per side and shifts the content.
* The focus ring goes on the **whole row**, edge to edge, not on the input inside it. Row focus shows when the row is active *and* the panel contains focus.
* Inline inputs are borderless: `appearance(false)`, `bordered(false)`, `focus_bordered(false)`, like the "Search connections" input.
* Separators in popovers and menus touch the popover edges (`.p_0()` on the popover, padding only inside lists).

## gpui-kit and GPUI quirks

* An `absolute` element is placed relative to its **direct parent**, not the nearest `relative` ancestor as in CSS. Prefer normal flex columns (this is how indent guides work).
* The library input defaults to a 32px control with 14px text, and its frame can end up taller than the height given. Wrap it in a flex container that centres it vertically, and give it the row's line height.
* Lucide icons are compiled in only when listed in `src/assets.rs`. Add a new icon there before using it.

## Reuse before building

Check `src/components/` first: `Button` (`Subtle` / `Filled`), `IconButton`, `SearchBar`, `InlineNameEditor`, `DragPreview` and the menu helpers in `menu.rs`. New panels and dialogs should use them so every control looks the same. Extend a component rather than styling a one-off copy.

## Verifying UI changes

* `cargo check` passing is not "fixed". For visual bugs, measure in the running app: a temporary debug hook that reaches the state, screenshots with `grim`, and logged layout boxes. Remove the hook afterwards.
* If a change was not seen on screen, say so plainly in the reply and tell the owner what to check.
* When a fix does not work, find the cause with measurements before trying another guess.

## State and files

* One owner per piece of persistent state. `Project` owns the connection stores and the keyring; panels ask it to change data and react to its events.
* Refuse writes while data is still loading, and never save a scope whose file failed to load.
* File operations never overwrite silently (`std::fs::rename` does), run on a background task, report errors as notifications and refresh the affected folders right away, including folders expanded but not yet read.
* Deleting from the project panel moves to the system trash.

## Comments

* Only comments that explain a non-obvious "why" (for example "`std::fs::rename` silently overwrites").
* No mentions of Zed, DataGrip or other products, no comments justifying a choice because the owner requested it, and no doc comments that restate the function name.

## Commits and agents

* Every commit must build on its own. When files mix several features, split them so that each commit still compiles, and run `cargo check` on each.
* Parallel agents work in worktrees created from the **current feature branch**, not `main`. Check the base commit before they start. Agents leave changes uncommitted; the main session reviews, merges and integrates.
* Remove agent worktrees in `.claude/worktrees/` once merged.
