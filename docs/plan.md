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

## Phase 3 — SQL editor and tabs (done)

**Goal:** edit and run SQL in tabs.

- [x] gpui-kit's `tree-sitter-sql` feature; code editor with line numbers and SQL highlighting, in the buffer font (Lilex 15px, line height 1.618, ligatures)
- [x] Tab bar in the center area: new, close, switch; dirty marker and confirm on close; hidden when no tab is open
- [x] Open `.sql` files from the project panel (double-click) and save them (Ctrl+S, with a save dialog for new tabs)
- [x] Per-tab connection selector, showing which connections are live
- [x] Run the statement under the cursor, or every statement in the selection (Ctrl+Enter), or the whole file (Ctrl+Shift+Enter); statements run one after another and stop at the first error
- [x] The run is stored on the tab, so closing the tab or pressing Cancel stops it
- [x] Running without a connection opens the connection menu (arrow keys and Enter) and runs once one is picked; a connection that is not connected is connected first
- [x] "New Query" from the connections panel menus opens a tab on that connection
- [ ] Per-tab database/schema selector

**Touched:** `src/ui/editor_panel.rs`, `src/sql_statements.rs` (statement splitting), `src/query_execution.rs` (`QueryRun`, `QueryExecution`), `src/ui/connections_panel.rs`, `src/ui/project_panel.rs`, `src/workspace.rs`.

**Open issues:**

- Cancel stops waiting, but the query keeps running on the server (needs the driver's cancel token).
- Closing the app with unsaved tabs gives no prompt.
- "Save As" does not check whether another tab already has that file open.
- Enter on a file row does not open it; only double-click does.

## Phase 4 — Bottom panel: results and output (done)

**Goal:** see query results and what happened.

The result grid and console became one DataGrip-style panel: the consoles that ran (one per editor tab, grouped by connection, with the last run's duration) on the left, and on the right a fixed Output tab plus one tab per `SELECT` of the latest run.

Result grid:

- [x] gpui-kit `DataTable` over the execution's rows; only visible rows and cells are rendered
- [x] Server-side cursor paging in PostgreSQL (`DECLARE` / `FETCH`, 500 rows a page), loading more while scrolling
- [x] Values colored by type per the DBolt design system (numbers orange and right-aligned, booleans purple, dates cyan, JSON green, NULL italic grey), mono cell font
- [x] Design system layout: 24px rows, 26px header with the column type and primary/foreign key icons (keys read from `pg_constraint`), row numbers, cell borders, editor background
- [x] Drag, row and column selection with row highlighting; Copy Value and Copy as CSV (with column names) from the right-click menu, Ctrl+C for CSV
- [x] Row count and timing above the grid
- [x] One result tab per `SELECT` of the latest run, titled `database.schema.table` from its `FROM`; each run replaces them

Output (replaces the console):

- [x] Log of every statement with its timestamp, then its outcome (rows retrieved, rows affected or the error) and duration
- [x] SQL highlighting, grey timestamps, colored prompt and red errors; selectable text with Copy and Select All; buffer font
- [x] Consoles can be closed from their right-click menu
- [ ] Server messages (`NOTICE`) and a "Connected to …" line
- [ ] Execution time separate from fetching time

Panel:

- [x] Bottom panel toggle in the status bar; a close button in the panel; the panel opens when a run starts

**Touched:** `src/ui/results_panel.rs`, `src/query_execution.rs`, `src/workspace.rs`, `src/ui/status_bar.rs`, `crates/dbolt_database/src/query.rs` (`QueryCursor`, `ColumnKey`), `crates/dbolt_postgres`, `themes/*.json` (table background).

**Open issues:**

- A connection keeps one open cursor: when a run has several `SELECT`s, a later one stops the earlier result from loading more rows than it already has.
- Loaded rows stay in memory, so scrolling to the end of a huge result keeps every row.
- Dragging a selection does not scroll the grid; a column selection covers only the loaded rows.
- The output prompt shows the database, not the current schema.

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
| 3. SQL editor and tabs | 2–3 | done |
| 4. Results and output | 3–4 | done |
| 5. Terminal | 5–8 | later |
| 5. Autocomplete | large | later |

Current work total: about 11–15 sessions.
