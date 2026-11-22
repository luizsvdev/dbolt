# DBolt implementation plan

## Approach

Each panel is built directly with real logic: UI wired to `dbolt_database` / `dbolt_postgres`, with all I/O on background tasks. No mock layer. The driver abstraction already exists, so mocks would only be thrown away later and would hide integration problems (async loading, errors, cancellation) until the end.

Estimates are in sessions (1 session ≈ 1–3 focused hours).

Rules that apply to every phase (see `.rules`):

* Never block the foreground thread: database, filesystem and keyring work run in `cx.background_spawn`, results are applied to entities afterwards.
* Large lists (trees, result rows, console lines) are virtualized.
* Every failure (connection, query, file, keyring) reaches the UI, not only the log.
* SQL uses bound parameters and the dialect's identifier quoting, never string concatenation.
* Secrets never go to logs or plain-text files.

## Phase 1 — Project context (3–4 sessions)

**Goal:** open a folder as the project; connections are stored per project by default, with optional global connections (as in DataGrip); the project panel shows the folder's files.

- [ ] `Project` entity: root path, project connections, global connections, change events
- [ ] Open folder (native folder picker) and a recent projects list stored in the user config dir
- [ ] Project connections in `<project>/.dbolt/connections.json`
- [ ] Global connections in the user config dir (for example `~/.config/dbolt/connections.json`)
- [ ] Merged connection list; new connections are assigned to the project by default, with an option to make them global
- [ ] Passwords in the OS keyring via the `keyring` crate; profile files store only a reference
- [ ] Project panel file tree: directory reads on background tasks, folders loaded on expand, file watching to refresh
- [ ] Load/save errors shown in the UI

**Likely touched:** new crate `crates/dbolt_project` (`[lib] path = "src/dbolt_project.rs"`) for the model, storage and keyring; `src/workspace.rs`; `src/ui/project_panel.rs`; `src/ui/title_bar.rs` (project name, open action).

**Done when:** opening a folder shows its file tree; connections saved in one project don't appear in another; global ones appear everywhere; no password is found in any file on disk.

## Phase 2 — Connections panel (3–4 sessions)

**Goal:** a working database explorer.

- [ ] Tree with folding and user groups: group → connection → database → schema → tables/views
- [ ] Children loaded lazily on expand through `RelationalConnection` (`list_schemas`, `list_tables`) on background tasks; loading and error states per node
- [ ] Search input filters the tree
- [ ] Context menus: connect, disconnect, edit, duplicate, delete, refresh, new query
- [ ] New/edit connection dialog generated from `Driver::connection_form()`, with a "Test connection" action (`ping`)
- [ ] Connection state (disconnected, connecting, connected, failed) shown on the node and in the status bar
- [ ] Driver registry built at startup (`DriverRegistry` with the PostgreSQL driver)

**Likely touched:** `src/ui/connections_panel.rs`, a new `src/ui/connection_dialog.rs`, `src/components/search_bar.rs`, `src/workspace.rs`, `crates/dbolt_database` (small additions if needed).

**Done when:** a PostgreSQL connection can be created, tested, connected and browsed down to its tables; a wrong password or unreachable host shows a clear error.

## Phase 3 — SQL editor and tabs (2–3 sessions)

**Goal:** edit and run SQL in tabs.

- [ ] Enable gpui-kit's `tree-sitter-sql` feature; code editor input with line numbers and SQL highlighting
- [ ] Tab bar in the center area: new, close, switch; dirty marker and confirm on close
- [ ] Open and save `.sql` files from the project panel
- [ ] Per-tab connection (and database/schema) selector
- [ ] Run statement under cursor / selection (⌘↵ / Ctrl+↵) on a background task; the task is stored on the tab so closing the tab or pressing cancel drops it

**Likely touched:** new `src/ui/editor.rs` (editor pane and tabs), `src/workspace.rs`, `Cargo.toml` (feature flag).

**Done when:** several tabs can each run SQL against different connections; closing a tab cancels its running query.

## Phase 4 — Bottom panel: result grid and console (3–4 sessions)

**Goal:** see query results and what happened.

Result grid (2–3 sessions):

- [ ] Table delegate over `QueryResult`; only visible rows and cells are rendered
- [ ] Paging through `has_more` / `load_more` instead of loading the whole result
- [ ] Values colored by type per the DBolt design system (numbers orange and right-aligned, booleans purple, dates cyan, JSON green, NULL italic grey)
- [ ] Row count and timing in the panel header; SQL errors shown in place of the grid
- [ ] One result tab per executed statement

Console / log (1 session):

- [ ] Virtualized list of query history and server messages, with timestamps and durations
- [ ] Click an entry to copy or reopen the statement

**Likely touched:** new `src/ui/results_panel.rs`, new `src/ui/console.rs`, `src/workspace.rs` (bottom dock), `crates/dbolt_database/src/query.rs` (paging support), `crates/dbolt_postgres`.

**Done when:** a query returning 1,000,000 rows scrolls smoothly without loading everything into memory; errors and timings appear in the grid and the console.

## Phase 5 — Later stage (NOT part of the current work)

These are explicitly postponed until phases 1–4 are complete.

### Terminal (5–8 sessions)

- [ ] Real shell in a bottom panel tab using `alacritty_terminal` and `portable-pty`
- [ ] Grid rendering, keyboard input, selection, scrollback, resize
- [ ] Starts in the project folder

### Schema-aware autocomplete (large)

- [ ] `CompletionProvider` for the SQL editor fed by cached schema metadata from phase 2

## Summary

| Phase | Sessions | Status |
| --- | --- | --- |
| 1. Project context | 3–4 | current |
| 2. Connections panel | 3–4 | current |
| 3. SQL editor and tabs | 2–3 | current |
| 4. Result grid | 2–3 | current |
| 4. Console / log | 1 | current |
| 5. Terminal | 5–8 | later |
| 5. Autocomplete | large | later |

Current work total: about 11–15 sessions.
