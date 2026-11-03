# Tessel Native Rust UI & Behavioral Parity Specification (`UI_PARITY.md`)

Working parity specification between:
- **Reference Electron/Vue app (`C:\Tessel`, read-only):** commit `2bb41843`
- **Native Rust rewrite (`C:\Tessel-rust`):** commit `942e092` (`b55`, `CARGO_PKG_VERSION = 0.1.0`)

## Non-Negotiable Parity Rules

1. **Exact visual, structural, shortcut, and behavioral parity** with reference Tessel (`@2bb41843`) — no approximate layouts and no generic top-level tab bars.
2. **Zero dummy or inert controls:** every visible button, menu item, chip, tab, and shortcut must perform its real backing action or remain hidden until its real Rust store/action is wired.
3. **Truthful native build identity:** never fabricate Electron's `1.17.2` release number in native Rust; surface the real Cargo package version (`0.1.0`, `env!("CARGO_PKG_VERSION")`) and real Git build stamp (`env!("TESSEL_BUILD")`, e.g. `b26 (6b92e6f)` from `build.rs`).
4. **Strict isolation from running `C:\Tessel`:** `C:\Tessel` is strictly read-only; native Rust keeps its own state under `%APPDATA%\TesselRust` and never imports live credentials, sessions, or browser profiles automatically.

---

## 1. Main Shell & Chrome (`App.vue`, `style.css`, `StatusBar.vue` vs `src/app.rs`)

### 1.1 Architectural Gap Summary

| Surface | Electron Reference (`@2bb41843`) | Current Rust (`@6b92e6f`) | Required Parity Change |
| --- | --- | --- | --- |
| Root shell | 4-zone shell: `40px` Top Toolbar + (`WorkspaceSidebar` \| `workspace-main` \| `SidePanel`) + Bottom `StatusBar` (`App.vue:14728-15273`, `style.css:107-250`) | Top bar with 11 flat `enum View` buttons (`Work`, `Files`, `Tasks`, `Settings`, `Git`, `Chat`, `Share`, `Remote`, `Browser`, `Vault`, `Account`) (`src/app.rs:64-79, 1486-1535`) | Remove the 11-tab top-bar view switcher; route every capability to its exact Tessel location (center grid/tabs, right `SidePanel`, or modal dialog). |
| `Work` + `Chat` | Unified per-project center `PaneGrid` under the `Terminal` `ViewTab` (shells, CLI agents, and native chat panes coexist as leaves in one split tree) | Persistent per-project split trees (`terminal_layout.rs`) now unify terminals and native Codex/Claude chat panes in the same grid (`b38`, dormant on restore) | Wire per-pane chat/terminal view toggles, multi-conversation/additional chat providers, and center `Editor`/`Browser` `ViewTabs` leaves. |
| `Files` | Right `SidePanel` `Files` tab (`Ctrl+Shift+X`), opening files into center `ViewTabs` (`Editor` tabs) | Native expandable `ExplorerTree` (`src/explorer_tree.rs`) wired into right `SidePanel` (`b41`, async off-UI worker, persistent open folders, root confinement) | Add Git status tinting (`explorerStatus.js`), Open Editors section, and remote SFTP tree source. |
| `Git` | Right `SidePanel` `Changes` tab (`Ctrl+Shift+G`) with changed-file count badge (`1..99+`) | Top-level `View::Git` | Move Git status/diff/stage/commit into right `SidePanel` `Changes` tab; clicking a diff opens/previews in center `Editor` tab (`F7` / `Shift+F7` hunk navigation). |
| `Tasks` | Right `SidePanel` `Tasks` tab (`Ctrl+Shift+K`) with 4-column board (`To do`, `Doing`, `Review`, `Done`) | Top-level `View::Tasks` (already had 4 columns `To do` / `Doing` / `Review` / `Done` in `src/app.rs`, rendered as a top-level view) | Move the existing 4-column Kanban board (`To do`, `Doing`, `Review`, `Done`) from top-level `View::Tasks` into the right `SidePanel` `Tasks` tab (`Ctrl+Shift+K`). |
| `Browser` | Center `ViewTabs` `Browser` leaf tabs AND right `SidePanel` `+` (`web-*`) browser tabs | Top-level `View::Browser` | Support Browser leaves as center `ViewTabs` tabs and right `SidePanel` `+` tabs, hiding WebView2 bounds when occluded by modals or inactive tabs. |
| `Settings`, `Vault`, `Account`, `Remote` | `SettingsDialog` modal (`Ctrl+,`, `settingsPages.js`) with 5 sidebar groups (`AI`, `Workspace`, `Workflows`, `Insights`, `About`) + SSH badges/SFTP integrated into projects & `Files` | 4 separate top-level views (`Settings`, `Vault`, `Account`, `Remote`) | Consolidate into `Settings` modal (`Ctrl+,`) pages (`Appearance`, `Shortcuts`, `Agent CLIs`, `SSH Hosts`, `Passwords`, `Tessel Account`, `Backup`, `About`); integrate remote SSH project badges/SFTP into `WorkspaceSidebar` & `Files`. |
| `Share` | Toolbar `Live Share` button/pill opening `LiveShareDialog` modal + `LIVE` badge on shared project | Top-level `View::Share` | Replace top-level `Share` tab with toolbar `Live Share` button/active pill + modal dialog + `LIVE` project badge. |

### 1.2 Top Toolbar (`App.vue:14729-15060`, `style.css:9-25, 114-250`)

- **Container:** Fixed height `40px`, `padding: 0 10px`, `gap: 6px`, background `--chrome` (`#101216` in Classic), bottom border `1px solid --border` (`#252932` in Classic), 3 flex regions (`tb-left`, `tb-center`, `tb-right`).
- **Left (`tb-left`):**
  - Tessel 6-tile T + cursor mark (`16×16px`, `viewBox="0 0 24 24"`, `App.vue:14734-14748`: release `#6b8fff`/`#5e78d4`/`#44589b`/`#e6e6e6`; dev `#f5c542`/`#c9a23b`/`#957a31`/`#e6e6e6`).
  - App title `"Tessel"` (`12.5px`, weight `600`, `--text`).
  - Native build identity (`CARGO_PKG_VERSION` `0.1.0` + `TESSEL_BUILD` `b<count> (<commit>)`, hover tooltip showing full build identity).
- **Center (`tb-center`):**
  - Centered search trigger button (`max-width: 380px`, `height: 26px`, `--surface-2` `#1f232b` / `--surface` `#181b21`, `1px solid --border` `#252932`, `border-radius: 7px`): magnifier icon + `"Search panes, run a command"` + `Ctrl+Shift+P` keycap badge. Clicking opens `CommandPalette`.
- **Right (`tb-right`):**
  1. Conditional parked `Agent request` shield chip and conditional `Update` chip (shown only when real state exists).
  2. **Split Launch control:** primary `+` button (`New terminal` / default agent, `Ctrl+Shift+T`) joined to narrow `▾` chevron button (`LaunchMenu`, `Ctrl+Shift+Space`).
  3. Vertical divider (`1px × 18px`, `--border`).
  4. Toolbar icon buttons (`30×28px`, `border-radius: 6px`):
     - `Live Share` (share icon, or active live pill when hosting/joined)
     - `Broadcast` (`Ctrl+Shift+B`; when active, renders `.broadcast-banner` strip directly below the toolbar)
     - `Layout` menu (`Split right Ctrl+Shift+E`, `Split down Ctrl+Shift+O`, `Arrange panes in a grid`, `Even grid` chips `1x2`, `2x1`, `2x2`, `3x2`, `3x3`, `Close active pane Ctrl+Shift+W`)
     - `Floating terminal` toggle (`Ctrl+\``)
     - `Toggle left sidebar` (split-rect icon with filled left pane)
     - `Toggle right sidebar` (split-rect icon with filled right pane)
     - `Settings` gear (`Ctrl+,`)
     - *(Staged only when real backing state is wired: `Usage`, `Notifications`, `Issues`)*

### 1.3 Bottom Status Bar (`StatusBar.vue`, `statusBarModel.js`)

- **Height:** `24px`, `--chrome` background, top border `1px solid --border` (`#252932` in Classic), `--text-dim` (`#8a92a0` in Classic).
- **Responsive breakpoints (`statusBarModel.js`):**
  - `width >= 900px`: full labels.
  - `500px <= width < 900px` (`compact`): shortened labels.
  - `width < 500px` (`iconOnly`): icons + counts only.
- **Left items:** Active input target indicator, pane summary, active worktree/project `cwd`.
- **Right items:** Keep Awake (`Off` / `Agent` / `On`), Resource Manager (`Memory · N terminals` + popover), Listening Ports count + popover, SSH host status (when remote projects exist), build/status message, right-click visibility menu.

---

## 2. Left Workspace Sidebar (`WorkspaceSidebar.vue`, `sidebarModel.js`, `layoutLimits.js`)

### 2.1 Exact Sizing, Collapse & Rail Rules (`WorkspaceSidebar.vue:370-424`, `layoutLimits.js`, `App.vue:3279`)

- **Default expanded width (`DEFAULT_WIDTH`):** `280px`.
- **Minimum expanded width (`MIN_WIDTH`):** `220px`.
- **Maximum width (`sidebarMaxWidth`):** `max(500, round(window_width * 0.3))` (`SIDEBAR_MIN_MAX = 500`).
- **Collapsed rail width:** `52px` (`App.vue:3279`).
- **Drag collapse / re-expand (`RESIZE_COLLAPSE_BELOW = 110`):**
  - When expanded, releasing drag with `raw < 110px` keeps the previous width saved and toggles `collapsed = true`.
  - When collapsed, dragging outward past `raw > 110px` toggles `collapsed = false`.
- **Double-click handle (`RESIZE_DOUBLE_CLICK_MS = 400`):** two pointer-downs within `< 400ms` uncollapse (if collapsed) and reset width to `280px`.

### 2.2 Structure & Interactions

1. **Header & Controls:** `Projects` / `Workspaces` header with `Workspace options` (group/sort/sleeping/default branch), `Add project` (`Ctrl+Shift+N`), and `New task/workspace`.
2. **Project Rows:** Folder chevron + icon (`Server` + `SSH · host` badge for remote projects; `LIVE` badge when shared), double-click inline rename, dragreorder, `+` new task, and `…` project context menu (`Rename`, `Change Project Folder…`, `Group`, `New Task…`, `Clean Up Worktrees…`, `Remove Project`).
3. **Worktree Cards & Inline Session Rows:** Main card + git worktree cards, listing live shell/agent leaves belonging to each card (`viewKey(leaf, cwd)`) with `AgentStateDot` (`working`, `approval`, `attention`, `idle`, `asleep`, `exited`), port pills, click-to-focus, and right-click menu (`Go to Pane`, `Sleep`, `Remove`).
4. **No File Tree in Left Sidebar:** all project file browsing lives in the right `SidePanel` (`Files` tab).

---

## 3. Center Workspace Area: Split Grid, `ViewTabs`, Headers, Launcher & Floating Terminal

### 3.1 Single Split-Tree & Per-Worktree View Layers (`paneGrid.js`, `paneViews.js`)

- **Tree nodes:**
  - `Split { dir: Row | Col, children, sizes }` (`shareOf(node, i)` returns `sizes[i]` if finite `> 0` and `sizes.len() == children.len()`, else `100.0 / children.len()`).
  - `Leaf` with `id`, `kind` (`Shell`, `Agent`, `Editor`, `Browser`), `worktree_path`, `view_path`, `live_share_page`.
- **View partitioning (`paneViews.js`):**
  - `normPath(p)`: trim trailing `/` or `\`, convert `\` to `/`, lowercase.
  - `leafViewPath(leaf)`: `leaf.worktree.path` if non-empty, else `leaf.viewPath`, else `""`.
  - `viewKey(leaf, cwd)`: `let k = normPath(leafViewPath(leaf)); if !k.is_empty() && k != normPath(cwd) { k } else { "" }`.
  - `treeViews(tree, cwd)`: sorted with `""` **always first**, then lexicographically by `key` (never tree order, so splitting/re-tiling never reorders view layers).
  - `isTabLeaf(leaf)`: `kind == Editor || kind == Browser || isLiveSharePage(leaf)`.
  - `shownTab(ws, view)` (`paneViews.js:114-121`):
    - Let `tab_ids` be `view.tabs` leaf IDs.
    - If `ws.active_id` is in `view.ids`:
      - If `ws.active_id` is NOT in `tab_ids`, return `"terminal"`.
      - Else if `ws.view == "terminal"` and `view.grid.is_none()`, return `"terminal"`; otherwise return `ws.active_id`.
    - Else return `ws.view` if `ws.view` is in `tab_ids`, otherwise `"terminal"`.
  - **`gridReturnLeaf` invariant (`App.vue:1277-1281, 2842-2853`):**
    - `gridLeafFor(ws, from)`: among non-tab leaves in `ws.tree` with the same `viewKey(leaf, ws.cwd) == viewKey(from, ws.cwd)`, returns the one matching `ws.grid_id` if present, else the first matching non-tab leaf.
    - `gridReturnLeaf(ws, from, tree)`: if `ws` is the current workspace and `maximized_id` points to a non-tab leaf in `tree` with the **same `viewKey`** as `grid`, return `maximized_id`; otherwise return `grid`. Used by both `showViewTab(ws, "terminal")` and `closeLeaf`.

### 3.2 Auto-Tiling, Dividers & Geometric `Alt+Arrows` (`paneGrid.js`, `PaneGrid.vue`, `App.vue:5840-5884`)

- **Auto-tile shape (`paneGrid.js:16-64`):**
  - `TARGET_ASPECT = 1.2`, `MIN_CELL_WIDTH = 360.0`, `MIN_CELL_HEIGHT = 160.0` (fallback room `1600×900` when `width <= 0` or `height <= 0`).
  - `rowCounts(n, rows)`: `base = n / rows`, first `n % rows` rows have `base + 1` panes (`rowCounts(5, 2) == [3, 2]`).
  - `shapeCost(counts, w, h)`: mean over all `n` cells of `|ln((cell_w / cell_h) / 1.2)| + (cell_w < 360 ? 4 * (360 / cell_w - 1) : 0) + (cell_h < 160 ? 4 * (160 / cell_h - 1) : 0) + 0.5 * |ln((cell_w * cell_h) / mean_area)|`.
  - `gridShape(n, w, h)`: tests `rows = 1..=n`; **ties within `1e-9` go to fewer rows** (`cost < best.cost - 1e-9`).
- **Interactive dividers (`PaneGrid.vue:25-26, 68-127`):**
  - `MIN_PANE_WIDTH = 325.0`, `MIN_PANE_HEIGHT = 156.0`.
  - Pointer drag between child `i` and `i + 1` changes **only** `(sizes[i], sizes[i + 1])` keeping `pair_sum = sizes[i] + sizes[i + 1]` constant, clamping each side to `min(pair_sum / 2.0, (min_px / split_inner_px) * sum_shares)`, then writes proportional shares back via `writeViewSizes`.
  - Keyboard resize (`ArrowLeft`/`ArrowRight` on `Row`, `ArrowUp`/`ArrowDown` on `Col`): `±2.0%` step, rejected if either neighbor would drop `< 5.0%`.
- **Geometric `Alt+Arrows` (`App.vue:5840-5884`):**
  - Candidate tile `r` relative to active tile `cur` (2px edge tolerance):
    - `Left`: `r.right <= cur.left + 2`, `dist = cur.left - r.right`, `off = |cy(r) - cy(cur)|`
    - `Right`: `r.left >= cur.right - 2`, `dist = r.left - cur.right`, `off = |cy(r) - cy(cur)|`
    - `Up`: `r.bottom <= cur.top + 2`, `dist = cur.top - r.bottom`, `off = |cx(r) - cx(cur)|`
    - `Down`: `r.top >= cur.bottom - 2`, `dist = r.top - cur.bottom`, `off = |cx(r) - cx(cur)|`
  - Picks candidate minimizing `score = dist * 4.0 + off`.

### 3.3 `ViewTabs`, Pane Headers, `ProjectLauncher` & `FloatingTerminal`

- **`ViewTabs` (`ViewTabs.vue`, `EditorPane.vue`, `BrowserPane.vue`):**
  - `Terminal` tab always first, followed by Editor file tabs (italic preview on single-click, double-click to pin, `7px` dirty dot + accessible dirty state, strikethrough if deleted, `--danger` if failed, `×` / middle-click / `Ctrl+W` close) and Browser tabs (title/host, audio button, `×` / middle-click close).
- **Terminal / Agent Pane Header (`TerminalPane.vue:4348-4560`, `PaneActionsMenu.vue`):**
  - Provider/shell icon + status dot, double-click inline rename title, optional model/account chips, **at most one priority status badge** (`exited`, `approve?`, `question?`, `limit`, `stuck`, `Restart to apply`, `needs you`, `working`, `asleep`), conditional broadcast `write` toggle, chat/terminal mode switch, Maximize/Restore, `…` `PaneActionsMenu`, and `×` close.
- **Empty-Grid `ProjectLauncher` (`ProjectLauncher.vue`):**
  - Shown when `view.grid` is empty; supports `1..9`, `ArrowUp`/`ArrowDown`, `Enter`, shell selector, detected Agent CLIs, Browser, and recent sessions.
- **Slide-Down `FloatingTerminal` (`floatingTerminal.js`, `FloatingTerminal.vue`):**
  - Height fraction bounds: `MIN_HEIGHT = 0.15` (`15%`), `MAX_HEIGHT = 0.90` (`90%`), `DEFAULT_HEIGHT = 0.45` (`45%`), rounded to `0.001` (`clampHeight`).
  - Toggled **only** by `Ctrl+\`` (`isFloatingToggleKey`, when `!dialogOpen()`) or the toolbar button — **`Escape` belongs to the terminal inside `FloatingTerminal`** (`floatingTerminal.js:7`), never hiding the panel so `vi`/`less`/shells behave naturally.
  - Persistent PTY outside the workspace split tree; `Ctrl+Shift+W` inside `.floating-term` kills & hides the floating terminal (`floating.close()`), and `Ctrl+Shift+R` restarts it (`floating.restart()`).

---

## 4. Right Resizable Side Panel (`SidePanel.vue`, `sideTabs.js`, `App.vue:3242-3305`)

### 4.1 Exact Sizing, Collapse & Fullscreen Rules

- **Default width (`TASK_PANEL_DEFAULT`):** `330px`.
- **Minimum width (`TASK_PANEL_MIN`):** `260px`.
- **Dynamic maximum width (`clampTaskPanel`, `App.vue:3278-3282`):**
  - `side = sidebar_collapsed ? 52 : sidebar_width`
  - `max = max(260, win_width - side - 420)` (always leaves at least `420px` for center panes alongside the left sidebar; no fixed cap on wide/ultrawide monitors).
  - `shown_width = round(min(max, max(260, requested_width)))`.
- **Drag-to-close (`RESIZE_COLLAPSE_BELOW = 110`):** dragging within `110px` of the right window edge closes the panel on release while preserving `taskPanelWidth`.
- **Double-click handle (`RESIZE_DOUBLE_CLICK_MS = 400`):** resets `taskPanelWidth` to `330px`.
- **Fullscreen (`SidePanel.vue:190-218`):** `Escape` (when event is not already `defaultPrevented`, and focus is inside the side panel or `document.body`) exits fullscreen (`setFullscreen(false)`).

### 4.2 Canonical Tab Order & History (`sideTabs.js`, `SidePanel.vue:36, 112-140, 232-240`)

- **`SIDE_TABS`:** `['dashboard', 'files', 'changes', 'tasks', 'taskHistory', 'history', 'conversations']`, followed by dynamic `web-*` browser tabs and `+` (`New page`).
- **Default initial prop tab:** `'tasks'` (`SidePanel.vue:36`).
- **Saved-tab restoration (`restoredSideTab`, `sideTabs.js:8-12`):**
  - Valid `SIDE_TABS` entry or live `web-*` id -> restored as-is.
  - Missing `web-*` id -> falls back to `'dashboard'`.
  - Empty / unknown -> `None` (keep caller default).
- **Closing a `web-*` tab (`SidePanel.vue:127-138, 232-240`):**
  - `visited` records the last `<= 20` previous tab IDs; closing the active `web-*` tab switches to the latest still-existing entry in `visited`, or `'dashboard'` if none.
- **Shortcut toggle (`toggleSideTab`, `App.vue:3416-3423`):**
  - If `taskPanelOpen && sideTab == tab`, sets `taskPanelOpen = false`; otherwise sets `sideTab = tab` and `taskPanelOpen = true`.
- **No dummy tabs:** expose only tabs backed by live Rust state (`files`, `changes` with `1..99+` badge, `tasks` 4-column board, `dashboard`, `+` browser pages, and `history`/`taskHistory`/`conversations` as each store is wired).

---

## 5. Modals & Exact Theme Palettes (`style.css`, `themes.css`, `themes.js`, `src/shared/themePalettes.js`)

### 5.1 Modal Dialogs

1. **`SettingsDialog` (`Ctrl+,`):** modal consolidating `Appearance & Theme`, `Terminal & Shell`, `Keyboard Shortcuts`, `Agent CLIs`, `SSH Hosts`, `Saved Passwords / Vault`, `Tessel Account`, `Backup & Restore`, and `About` (`0.1.0` + `TESSEL_BUILD`).
2. **`CommandPalette` (`Ctrl+Shift+P`, `CommandPalette.vue`):**
   - Multi-word filter over `"{title} {group} {hint}"` (case-insensitive, every whitespace-split query word must match).
   - Score: `0` if `title` starts with trimmed lowercase query `q`, `1` if `title` contains `q`, `2` otherwise; stable sort by score; group headers emitted on group transitions; `ArrowUp`/`ArrowDown` wrap modulo `results.len()`; `query` change resets `index = 0`.
3. **`FileFinder` (`Ctrl+Shift+J`):** fuzzy project file jump opening into center `Editor` tab.
4. **`HelpDialog` (`F1`, `HelpDialog.vue`, `helpShortcuts.js`):**
   - 2-column searchable shortcut reference in EN/FR (`Shift` -> `Maj`, `Esc` -> `Échap`, `Space` -> `Espace`, `PageUp` -> `Pg préc`, `PageDown` -> `Pg suiv`, `Arrows` -> `Flèches`), Unicode NFD mark-stripped search (`filterShortcuts`); first `Escape` clears non-empty `query`, next `Escape` closes dialog.
5. **`LiveShareDialog`:** opened from toolbar `Live Share` button/pill.

### 5.2 Exact 7 Themes (Direct Source Transcription)

From `style.css:9-25`, `themes.css:3-22`, `themes.js:10-64`, and `src/shared/themePalettes.js:5-69`:
- **`classic` and `warp` do NOT override the 16 ANSI terminal colors** (`ansi = None`; programs retain default ANSI semantics).
- **The 5 named palette themes (`dracula`, `nord`, `tokyo-night`, `gruvbox`, `solarized-dark`) set `--backdrop = ui.chrome`, `cursorAccent = term.background`, and supply full 16-color ANSI tables.**

| ID | Label (EN / FR) | `--chrome` | `--surface` | `--surface-2` | `--surface-3` | `--term` | `--backdrop` | `--border` | `--border-strong` | `--accent` | `--warn` | `--danger` | `--text` | `--text-strong` | `--text-dim` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `classic` | Classic / Classique | `#101216` | `#181b21` | `#1f232b` | `#282d37` | `#15171c` | `#0c0d10` | `#252932` | `#333946` | `#6c9cff` | `#f5a524` | `#e5484d` | `#d6d9df` | `#f1f3f6` | `#8a92a0` |
| `warp` | Warp-inspired / Inspiré de Warp | `#161917` | `#202321` | `#252b27` | `#2c3930` | `#191b1a` | `#161917` | `#303631` | `#47544a` | `#aed5b2` | `#dbba87` | `#e5a6a0` | `#dfe5df` | `#eff3ed` | `#939e94` |
| `dracula` | Dracula | `#1e1f29` | `#21222c` | `#282a36` | `#343746` | `#282a36` | `#1e1f29` | `#343746` | `#44475a` | `#bd93f9` | `#ffb86c` | `#ff5555` | `#f8f8f2` | `#ffffff` | `#8b8fa8` |
| `nord` | Nord | `#242933` | `#2b303b` | `#2e3440` | `#3b4252` | `#2e3440` | `#242933` | `#3b4252` | `#4c566a` | `#88c0d0` | `#ebcb8b` | `#bf616a` | `#d8dee9` | `#eceff4` | `#8a93a6` |
| `tokyo-night` | Tokyo Night | `#16161e` | `#1a1b26` | `#1f2335` | `#292e42` | `#1a1b26` | `#16161e` | `#292e42` | `#3b4261` | `#7aa2f7` | `#e0af68` | `#f7768e` | `#c0caf5` | `#e0e6ff` | `#737aa2` |
| `gruvbox` | Gruvbox | `#1d2021` | `#242424` | `#282828` | `#3c3836` | `#282828` | `#1d2021` | `#3c3836` | `#504945` | `#fabd2f` | `#fe8019` | `#fb4934` | `#ebdbb2` | `#fbf1c7` | `#a89984` |
| `solarized-dark` | Solarized Dark | `#00212b` | `#002833` | `#002b36` | `#073642` | `#002b36` | `#00212b` | `#073642` | `#1b4a55` | `#268bd2` | `#b58900` | `#dc322f` | `#93a1a1` | `#eee8d5` | `#657b83` |

Terminal base colors (`themes.js:39-64`, `themePalettes.js:10,16,22,28,34`):
- `classic`: `bg #15171c`, `fg #d6d9df`, `cursor #e8eaee`, `cursorAccent #15171c`, `selection #2f4a7a`, `ansi: None` (`--radius: 7px`)
- `warp`: `bg #191b1a`, `fg #dfe5df`, `cursor #aed5b2`, `cursorAccent #191b1a`, `selection #3b5342`, `ansi: None` (`--radius: 5px`, `--green/#a1c8a7`)
- `dracula`: `bg #282a36`, `fg #f8f8f2`, `cursor #f8f8f2`, `cursorAccent #282a36`, `selection #44475a`, `ansi: Some(16)`
- `nord`: `bg #2e3440`, `fg #d8dee9`, `cursor #d8dee9`, `cursorAccent #2e3440`, `selection #434c5e`, `ansi: Some(16)`
- `tokyo-night`: `bg #1a1b26`, `fg #c0caf5`, `cursor #c0caf5`, `cursorAccent #1a1b26`, `selection #33467c`, `ansi: Some(16)`
- `gruvbox`: `bg #282828`, `fg #ebdbb2`, `cursor #ebdbb2`, `cursorAccent #282828`, `selection #504945`, `ansi: Some(16)`
- `solarized-dark`: `bg #002b36`, `fg #839496`, `cursor #93a1a1`, `cursorAccent #002b36`, `selection #073642`, `ansi: Some(16)`

---

## 6. Exact Keyboard Shortcut & Routing Rules (`App.vue:14112-14300`, `TerminalPane.vue`, `EditorPane.vue`, `helpShortcuts.js`)

### 6.1 Exact Event Routing Order (No Blanket "All Globals Before PTY")

1. **Component-level capture handlers run first on their own focused surface:**
   - **Inside `HelpDialog` (`HelpDialog.vue:23-26`):** `Escape` (`preventDefault` + `stopPropagation`) clears `query` if non-empty, otherwise closes `HelpDialog`.
   - **Inside `CommandPalette` (`CommandPalette.vue:92-95`):** `ArrowDown` / `ArrowUp` move selection, `Enter` runs selected item, `Escape` (`stopPropagation`) closes palette.
   - **Inside `EditorPane` (`EditorPane.vue:838-883`):**
     - If inside an inline diff-note composer, only `Ctrl+S` is intercepted; other keys stay in the note input.
     - Plain `Ctrl+S` (no `Shift`/`Alt`/`Meta`, no repeat) saves active doc; plain `Ctrl+W` closes active editor tab; `Alt+Z` toggles `editorWordWrap`; `F7` / `Shift+F7` (when diff editor is shown, no `Ctrl`/`Alt`) jumps next/prev diff.
     - `EditorPane.isAppShortcut(e)` forwards ONLY: `isFloatingToggleKey(e)`, `Ctrl+Shift+{e,o,w,b,k,n,t,r,p,x,j,g,' '}` (no `Alt`), `Ctrl+{=,+,-,0,,,PageUp,PageDown}` (no `Shift`/`Alt`), `Alt+{ArrowLeft,ArrowRight}` (no `Ctrl`/`Shift` — **`Alt+ArrowUp/Down` stay in the editor** to move lines!), and `F1` to `onKey(e, { fromEditor: true })`.
   - **Inside `TerminalPane` (`TerminalPane.vue:2306-2319, 3944-4010`):**
     - `Ctrl+Shift+F`: opens terminal find bar (`openFind()`) and withholds key from PTY.
     - `TerminalPane.isAppShortcut(e)` withholds from PTY: `isFloatingToggleKey(e)`, `Ctrl+Shift+{e,o,w,b,k,n,f,r,p,x,g}` (no `Alt`), `Ctrl+{=,+,-,0,,,PageUp,PageDown}` (no `Shift`/`Alt`), `Alt+{ArrowLeft,ArrowRight,ArrowUp,ArrowDown}` (no `Ctrl`/`Shift`), and `F1` (while window `onKey` also handles `Ctrl+Shift+{t,' ',j}`).
     - Plain `Ctrl+V` (no `Shift`/`Alt`), `Ctrl+Shift+V` -> `pasteClipboard`; `Ctrl+Shift+C` -> `copySelection`; `Shift+Enter` in an agent PTY sends the agent newline sequence; `Shift+PageUp` / `Shift+PageDown` scroll terminal viewport.
     - All other keys (including plain `Ctrl+C`, `Ctrl+W`, `Ctrl+D`, `Ctrl+L`, `Ctrl+R`, `Tab`, plain `Arrows`, and `Escape`) pass to the PTY!
2. **Window `onKey(e, opts)` (`App.vue:14192-14300`) exact sequence:**
   - **Step 1 — Floating terminal toggle (`14193-14197`):** if `isFloatingToggleKey(e)` (`ctrl && !shift && !alt && !meta && (code == "Backquote" || key == "\`")`) and `!dialogOpen()`, prevent default, toggle floating terminal if `!e.repeat`, and `return`.
   - **Step 2 — UI text field guard (`14198`):** if `!opts.fromEditor && typingInField(e) && e.key !== "Escape" && e.key !== "F1"`, `return` immediately (`typingInField` is `false` for `.xterm`, `true` for `.editor-pane` and UI `input`/`textarea`/`select`/`[contenteditable="true"]`).
   - **Step 3 — Floating terminal focus guard (`floatingPaneKey`, `14172-14190, 14199`):** when target is inside `.floating-term`:
     - `Ctrl+Shift+W` (no `Alt`): calls `floating.close()` and returns.
     - `Ctrl+Shift+R` (no `Alt`): calls `floating.restart()` and returns.
     - `Ctrl+Shift+{E,O,B}` (no `Alt`) and `Alt+Arrow*` (no `Ctrl`/`Shift`): swallowed (`return`) so they never mutate or move focus in the background grid.
   - **Step 4 — Open dialog guard (`dialogOpen()`, `14200-14211`):** when any dialog/launcher is open:
     - `Ctrl+,` (no `Shift`/`Alt`) sets `settingsOpen = false` (never opens Settings over another dialog).
     - `F1` sets `helpOpen = false` (never opens Help over another dialog).
     - If `e.key !== "Escape"`, `return` immediately.
   - **Step 5 — `Ctrl+Shift` chords (`14212-14255`):** `e` (`SplitRight`), `o` (`SplitDown`), `w` (`CloseActive`), `b` (`ToggleBroadcast`), `k` (`ToggleSideTasks`), `x` (`ToggleSideFiles`), `g` (`ToggleSideChanges`), `n` (`CreateWorkspace`), `t` (`NewDefaultTerminal`), `' '` (`OpenLauncherCentered`), `r` (`RestartActive`), `p` (`TogglePalette`), `j` (`OpenFinder`).
   - **Step 6 — `Ctrl` (no `Shift`, no `Alt`) zoom (`14257-14268`):** `'=' | '+' | code=='NumpadAdd'` -> `Zoom(1)`; `'-' | code=='NumpadSubtract'` -> `Zoom(-1)`; `'0' | code=='Digit0' | code=='Numpad0'` -> `Zoom(0)` (AZERTY-safe).
   - **Step 7 — `Alt+Arrow*` (no `Ctrl`, no `Shift`) (`14269-14272`):** `moveFocus(left|right|up|down)`.
   - **Step 8 — `Ctrl+,` (`14273-14276`) & `F1` (`14277-14280`):** toggle `settingsOpen` / `helpOpen`.
   - **Step 9 — `Ctrl+PageUp` / `Ctrl+PageDown` (no `Shift`, no `Alt`) (`14283-14286`):** `cycleWorkspace(-1 | 1)` (`Ctrl+Alt` is intentionally avoided for European `AltGr`).
   - **Step 10 — `Escape` (`14287-14299` + `SidePanel.vue:211-218`):** if `SidePanel` is fullscreen and focus is in `SidePanel`/`body` (and not `defaultPrevented`), exits `SidePanel` fullscreen; in `App.onKey`, clears `maximizedId` if set, closes open non-busy dialogs (`help`, `settings`, `mcp`, `skills`, `sessions`, `github`, `linear`) and menus, **without** blocking `ESC` delivery to an active terminal PTY (`FloatingTerminal` stays open).

### 6.2 Reference `HelpDialog` Groups (`helpShortcuts.js:9-75`)

- **Panes (11):** `Ctrl+Shift+T`, `Ctrl+Shift+Space`, `Ctrl+Shift+P`, `Ctrl+Shift+J`, `Ctrl+Shift+E`, `Ctrl+Shift+O`, `Ctrl+Shift+W`, `Ctrl+Shift+R`, `Alt+Arrows`, `Esc`, `Ctrl+\``
- **Terminal (8):** `Ctrl+Shift+F`, `Ctrl+Shift+C`, `Ctrl+Shift+V`, `Ctrl+=`, `Ctrl+-`, `Ctrl+0`, `Shift+PageUp`, `Shift+PageDown`
- **Workspaces (3):** `Ctrl+Shift+N`, `Ctrl+PageUp`, `Ctrl+PageDown`
- **Editor (8):** `Ctrl+S`, `Ctrl+W`, `Ctrl+F`, `Ctrl+H`, `Ctrl+G`, `Alt+Z`, `F7 / Shift+F7`, `Ctrl+Shift+A`
- **App (8):** `Ctrl+Shift+B`, `Alt+A`, `Ctrl+Shift+K`, `Ctrl+Shift+X`, `Ctrl+Shift+G`, `Ctrl+,`, `Win+H`, `F1`

---

## 7. Approved File Split — Batch 1

### Hopper (Integration & Chrome)
- `src/app.rs`, `src/main.rs`, `src/theme.rs`, `src/terminal.rs`, `README.md`, `MIGRATION.md`

### Franklin (Isolated Pure Parity Modules + Integration Test Harness)
- `src/theme_palettes.rs` — Exact 7 themes (`classic`, `warp`, `dracula`, `nord`, `tokyo-night`, `gruvbox`, `solarized-dark`), UI tokens, title-bar colors, terminal colors, optional 16-color ANSI palettes (`None` for `classic` & `warp`), and EN/FR labels & descriptions.
- `src/pane_grid.rs` — `PaneTree` (`Split`/`Leaf`), `grid_shape` / `row_counts` / `tile_tree` / `without_leaves` / `is_near_grid` / `grid_layout` / `px_length`, divider pointer & `±2%` keyboard resize, `move_focus` (`dist * 4 + off`), `paneViews` (`view_key`, `tree_views`, `prune_tree`, `write_view_sizes`, `shown_tab`, `grid_leaf_for`, `grid_return_leaf`), and preset even grids.
- `src/shortcuts.rs` — Exact `is_floating_toggle_key`, `route_key_event` (`EditorPane` / `TerminalPane` / `HelpDialog` / `CommandPalette` / `SidePanel` / `App.onKey` sequence including `typing_in_field`, `floating_pane_key`, `dialog_open`, AZERTY `Digit0`/`Numpad0`), and EN/FR `shortcut_groups`, `key_name`, `chords_of`, NFD-folded `filter_shortcuts`.
- `src/palette.rs` — `CommandPaletteState` (multi-word filter, `0/1/2` prefix/substring ranking, group headers, wrapping move), `SidePanelState` (`SIDE_TABS`, `restored_side_tab`, `visited` max-20 history fallback to `'dashboard'`, `toggle_side_tab`), `LayoutLimits` (`sidebar_max_width`, left `280/220/52` & right `330/260/win-side-420`, `110px` collapse, `400ms` double-click reset), and `FloatingTerminalState` (`0.15..=0.90`, default `0.45`).
- `tests/ui_parity_models.rs` — Standalone integration test suite validating all four modules with `cargo test --locked -j 2 --test ui_parity_models` and `cargo clippy --locked -j 2 --test ui_parity_models -- -D warnings`.

### Franklin — Batch 2 (Isolated Sidebar, Status Bar, Finder & Launcher Models)
- `src/sidebar_model.rs` — Exact `sidebarModel.js` (`pane_dot_state`, `agent_state_label`, `summarize_agents`, `select_summary_group_icon_agents`, `format_short_time_ago`, `card_status`, `attention_of`, `build_project_cards`, git worktree visibility/grouping/preview, `compare_cards`, `is_card_visible`, `build_sidebar_rows`, `project_drop_index`, `move_to_index`, `merge_project_order`, `order_workspaces`, `neighbor_card`, `card_target_pane`, `port_probes`), `statusBarModel.js` + `StatusBar.vue` (`StatusBarDensity`, `format_memory`, `format_cpu`, `awake_copy`, `resource_tree`, `ResourceHistory` 60-sample 10-min ring, `sparkline_points`, `sparkline_lead`, `ports_summary`), and `TerminalPane.vue` priority header `pane_header_badge`.
- `src/finder_model.rs` — Exact `src/shared/fuzzy.js` (`fuzzy_score`, `fuzzy_filter`), `FileFinderState` (`limit = 60`, `Enter` open vs `Ctrl+Enter` insert) + pure-Rust bounded directory walk `walk_project_files` (`MAX_LISTED_FILES = 20_000`, `SKIP_DIRS` from `fileOpen.js`, symlink-safe, zero subprocess spawning), `projectLauncher.js` + `ProjectLauncherState` (`valid_project_open`, `REMOTE_AGENT_IDS`, `launcher_agents`, `launcher_shells`, `project_open_choice`, `remembered_value`, `emptied_workspace_choice`, `recent_project_sessions` via `sessionHistory.js:filterSessions`, `SSH_DROPPED = 255`, `dropped_remote_pane`, `allow_drop_retry`, `1..9` / arrow / `Enter` / `Escape` navigation), and `LaunchMenuState` (`placements`, `where_text`, `host_menu`, `6px` viewport clamp).
- `tests/ui_sidebar_models.rs` — Standalone integration test suite validating `src/sidebar_model.rs` and `src/finder_model.rs` with zero warnings under `-D warnings`.

### Franklin — Batch 3 (Isolated Explorer, SCM Backoff/Labels & Settings Models)
- `src/explorer_scm_model.rs` — Exact `explorerRows.js` (`22px` row height, `8px` base/level indent, `16px` twistie, `MAX_COMPACT_CHAIN = 256` compact folders, `toggled_paths`, `sole_subfolder`, `active_guide`, `segment_of`, `sticky_folders` max `7` lines), `explorerStatus.js` (`D > M > A/U > R > C` folder priority, nearest-ancestor `in_untracked`, `explorer_status_of`, `untracked_folder`, `folder_status`, `ignored_set`, `is_ignored`), `explorerChanges.js` (`in_git_dir`, `status_matters`, `folders_to_reload`, `same_entries`, `same_status_map`), and `scmState.js` + `scmLabels.js` (`SLOW_STATUS_MS = 3_000`, `MAX_STATUS_GAP_MS = 300_000`, `background_gap_ms`, deterministic `ScmRefreshCoalescer`, `change_count`, EN/FR `scm_text`, `scm_status_title`, `ScmSectionKind`, `discard_entry_copy`).
- `src/settings_model.rs` — Exact `settingsPages.js` + `SettingsDialog.vue` (`5` sidebar groups `AI` / `Workspace` / `Workflows` / `Insights` / `About`, all `22` `SettingsPageId`s with EN/FR titles & descriptions, legacy `text -> terminal#set-terminal-font` alias, `^[\w-]{1,60}$` anchor validation, `SettingsShellState` `mcp_tab` reset & navigation, and EN/FR multi-word `search_settings` including English title fallback and `Text`/`Font` alias matching).
- `tests/ui_explorer_settings_models.rs` — Standalone integration test suite validating `src/explorer_scm_model.rs` and `src/settings_model.rs` with zero warnings under `-D warnings`.

### Franklin — Batch 4 (Composed `ViewTabs`, Editor Tab Rules, Browser Address Guard & `PaneActionsMenu`)
- `src/view_tabs_browser_model.rs` — Composes directly with `crate::pane_grid::{EditorTabEntry, EditorTabStrip, ShownTab}` and `url::Url` (without duplicating `EditorTabStrip` or `src/browser.rs` session state): `editorTabs.js` (`DIFF_SEP`, `DiffTabArea`, `DiffTabMeta`, `path_key`, `same_path`, `diff_tab_path`, `is_diff_tab_path`, `doc_path_of`, `file_name_of`, `open_tab_in_strip` inserting immediately after active tab, `auto_save_delay_ms`, `valid_saved_files` capped at `50` absolute `<= 4000`-char paths, CRLF/UTF-8-safe `minimal_edit`, `main_eol`), `tabLabels.js` (`base_tab_label` parent-folder twin disambiguation `name · folder`, EN/FR `editor_tab_label`), `ViewTabs.vue` (`build_view_tabs` keeping `Terminal` first, Live Share host hints, preview/dirty/deleted/failed badges, `can_drag_view_tab`, `can_middle_close_view_tab`), and `browserUrl.js` (`BLANK_URL`, `SearchEngine`, `search_url`, `allowed_browser_url` with `allow_session_blob` compatibility, `normalize_browser_input` routing `localhost`/`127.*`/`[::1]`/`*.localhost:port` to `http://` and refusing `file:`/`javascript:`/`data:`/`user:pass@host`, `looks_like_search`, `is_local_url`, `display_url`, `host_of`).
- `src/pane_menu_model.rs` — Exact `PaneActionsMenu.vue` model & EN/FR strings shared by terminal and native chat headers: 8-item `facts` block (`Branch`, `Team`, `Account`, `Started` Yolo, `State` via `format_agent_state_fact`, `Quiet`, `Prompt cache`, read-only `Model`), `open_or_switch_chat_label`, and `build_pane_actions_menu` with exact item ordering, capability gating (zero dummy items), disabled reasons, Live Share text-size chips (`Fit` / `Width` / `Normal`), Voice typing chips, `SendSelectionTo` / agent-only `AskReview`, floating-pane filtering, and danger `ClosePane`.
- `tests/ui_tabs_pane_menu_models.rs` — Composed integration test suite validating `pane_grid` + `view_tabs_browser_model` + `pane_menu_model` with zero warnings under `-D warnings`.

### Franklin — Batch 5 (Native `egui` `ExplorerTree` Widget with Bounded Async Worker & Confinement)
- `src/explorer_tree.rs` — Native `egui` file explorer tree (`ExplorerTree::show(ui, root, active, actions_enabled) -> Option<TreeAction>`, `set_root(Option<&Path>)`, `refresh()`, `poll()`) composing with `explorer_scm_model` (`22px` row height, `8px` indent/base padding, `16px` twistie, active indent guides, optional compact folders with strict multi-segment `max_cached_dirs` accounting) and `i18n::tr`: zero UI-thread filesystem I/O, dirty-cached `ExplorerTreeResult` (`rows_dirty`) with zero steady-state cloning, strictly at most 1 background worker thread (`DirectoryLister` / `FsDirectoryLister`), immediate `set_root` stale-worker cancellation even when the panel is closed, `refresh()` pruning of deleted/missing folders from `open_dirs` and `path_by_key` at quiescence, directory/raw-examined-entry/wall-clock/total-entry/depth/project cache bounds (`TreeLimits`), lossless `OsString` path preservation + non-UTF-8 key disambiguation, symlink and Windows reparse-point (`FILE_ATTRIBUTE_REPARSE_POINT`) confinement inside canonicalized project root, and `OpenFile` + gated `CreateFile`/`CreateFolder`/`Rename`/`Delete` context actions.
- `tests/ui_explorer_tree.rs` — Integration test suite exercising real `egui::Context` pointer clicks, steady-state `row_rebuild_count()` caching, compact-chain capacity bounds, folder expansion persistence + deleted-folder pruning across refresh and project switches, panel-closed `set_root()` single-worker cancellation/stale-result discard, raw/entry cache bounds, non-UTF-8 OS path disambiguation, and symlink/reparse/outside-root confinement with zero warnings under `-D warnings`.

### Franklin — Batch 6 (Native `egui` `HelpPanel` Composed with `shortcuts.rs`, `supported_keys` Filtering & Two-Step `Escape`)
- `src/help_panel.rs` — Native `egui` keyboard shortcuts dialog body (`HelpPanel::default()`, `reset()`, `show(ui, french, supported_keys) -> bool`) composing directly with `crate::shortcuts` (`shortcut_groups`, `chords_of`, `fold_for_search`, `route_help_dialog_escape`): filters strictly to caller-supplied `supported_keys` so unimplemented shortcuts/groups are never advertised, applies truthful native descriptions where current Rust scope is narrower (`Ctrl+Shift+T` default shell only; `Ctrl+Shift+P` commands only) prior to search indexing, renders flow-balanced 2-column groups (`>= 600px`, 1 column when narrow) with framed `<kbd>` keycaps, `+` chord joiners, and localized `or`/`ou` chord alternatives, auto-focuses the search input on open/`reset()`, and consumes `Escape` before `egui::TextEdit` and `egui::ModalResponse::should_close` (non-empty query clears search and keeps modal open + focused; empty query returns `true` to close; held key-repeat `Escape` is absorbed).
- `tests/ui_help_panel.rs` — Integration test suite driving `HelpPanel` inside a real `egui::Modal` (`ModalResponse::should_close`), verifying `supported_keys` gating, truthful `Ctrl+Shift+T`/`Ctrl+Shift+P` labels, EN/FR accent-insensitive search over titles/descriptions/localized keycaps (`maj+t`, `echap`, `fleches`, `editeur`, `controle git`), auto-focus typing, clear button, key-repeat guard, and two-step `Escape` dismissal with zero warnings under `-D warnings`.

### Franklin — Batch 7 (Pure-Rust Vector `icons.rs` from `App.vue` & Lucide + `THIRD_PARTY_NOTICES.md`)
- `src/icons.rs` — Pure-Rust vector icon painter (`Icon::{Search, Plus, ChevronDown, Close, Help, Settings, Share, Layout, Terminal, PanelLeftOpen, PanelLeftClosed, PanelRightOpen, PanelRightClosed}`, `paint(painter, rect, icon, color)`, `paint_brand(painter, rect, development)`) with zero runtime I/O, zero external crate dependencies, and zero glyph approximations: exact inline SVG coordinates/arcs/Béziers from `App.vue` (`14734..15058`: 6-tile brand mark in release/dev palettes, `16×16` toolbar icons, `24×24` 8-tooth `Settings` gear) and `lucide-vue-next` (`v0.577.0`: `Close`, `Help`, `Terminal`), preserving 1:1 aspect ratio and scaled stroke widths across arbitrary `egui::Rect` sizes.
- `THIRD_PARTY_NOTICES.md` — Verbatim ISC License + Cole Bemis Feather MIT License attribution from `lucide-vue-next` (`v0.577.0`).
- `tests/ui_icons.rs` — Standalone `egui` vector shape test suite verifying bounding rects at `10`, `13`, `16`, `24`, and non-square rects, degenerate rect safety, theme color propagation, open/closed sidebar toggle fills, 8-tooth gear tessellation, and 6-tile release/dev brand mark palettes with zero warnings under `-D warnings`.

### Franklin — Batch 8 (Native `egui` `LauncherPanel` Composed with `finder_model` & `icons::Terminal`)
- `src/launcher_panel.rs` — Native `egui` terminal/agent launcher menu (`LauncherPanel::default()`, `reset()`, `show(ui, LauncherInput) -> Option<LaunchMenuAction>`, `paint_placement_icon`) matching `LaunchMenu.vue` (`62..120`, `224..384`) and `style.css` (`2550..2767`) with zero process spawning and zero filesystem/env I/O: segmented `Auto` / `Left` / `Right` / `Below` placement bar with exact `16×16` shaded-split vector geometry + localized EN/FR `where_text`, catalog grouping into `Terminals` / `Native chats` / `AI agents` (plus injected `Browser`) using `icons::Icon::Terminal` and vector chat/globe icons, `↑`/`↓`/`Enter`/`Esc` keyboard navigation + pointer activation strictly skipping unavailable (`available == false`) entries and clamping selection across dynamic catalog mutations, and strict omission of unimplemented worktree/accounts/host/tools controls (`in_worktree: false`).
- `tests/ui_launcher_panel.rs` — Standalone `egui` integration test suite driving `LauncherPanel` across keyboard navigation, disabled-row skipping & click refusal, `egui::Modal` placement radio clicks, dynamic catalog shrinking/disabling, EN/FR localization, omission of unimplemented controls, and `paint_placement_icon` shaded-split vector bounds with zero warnings under `-D warnings`.

### Franklin — Batch 9 (Native `egui` `ProjectPanel` Composed with `sidebar_model`, `shortcuts` & Vector Icons)
- `src/project_panel.rs` — Native `egui` left project & session sidebar (`ProjectPanel::default()`, `show(ui, ProjectPanelInput) -> Option<ProjectAction>`, `is_project_collapsed(u64)`, `is_global_collapsed()`, `request_focus(ctx)`) matching `WorkspaceSidebar.vue` (`1361..1744`), `WorktreeCard.vue` (`150..247`), `CompactAgentRow.vue` (`186..234`), and `AgentStateDot.vue` (`1..63`) with zero filesystem/git/process I/O: top `Search` button (`icons::Icon::Search`, EN/FR `"Search"` / `"Rechercher"`, `Ctrl+Shift+P` / `Ctrl+Maj+P` `<kbd>` keycaps -> `ProjectAction::Search`), `"Projects"` / `"Projets"` header with vector `FolderPlus` `"Add project"` / `"Ajouter un projet"` button (`ProjectAction::OpenFolder`), collapsible project cards (`SelectProject(Some(id))`) and `"Sessions without a project"` / `"Sessions sans projet"` group (`SelectProject(None)`) with collapse state persisted by project ID and pruned automatically on catalog removal, full-width clickable session rows (`SelectSession { project, id }`) with vector `Terminal` / `Chat` / Lucide `Moon` icons and truthful `sleeping` status only (never fabricating unobserved `working` / `waiting` states), single-line `'…'` truncation, and focus-scoped keyboard navigation (`↑` / `↓` / `←` / `→` / `Enter` / `Space`) that never consumes keys when focus belongs outside `ProjectPanel`.
- `tests/ui_project_panel.rs` — Standalone `egui` integration test suite verifying `Search` and `OpenFolder` button clicks, EN/FR localization & keycaps, strict absence of unimplemented controls & glyph fallbacks, full-width right-edge session row hit-testing across project and global sessions, per-ID collapse persistence and automatic stale-ID pruning, focus-scoped keyboard navigation without stealing `ArrowDown` / `ArrowLeft` / `Enter` from a focused `CentralPanel` terminal/editor widget or when disabled under overlays (`ui.add_enabled_ui(false)`), `220px` narrow truncation bounds, and vector icon/dot/moon geometry with zero warnings under `-D warnings`.

### Franklin — Batch 10 (Native `egui` `SettingsNav` Rail Composed with `settings_model` & Vector Icons)
- `src/settings_nav.rs` — Native `egui` Settings left navigation rail (`SettingsNav::default()`, `new()`, `reset()`, `query()`, `set_query()`, `focus_search(ctx)`, `show(ui, SettingsNavInput) -> Option<SettingsNavAction>`) matching `SettingsDialog.vue` (`233..245`, `363..433`) and `style.css` (`3405..3528`) with zero vault, filesystem, process, or persistence side effects: top `.set-back` button (`"Back to app"` / `"Retour à l’application"`) with exact `16×16` stroke back arrow (`M13 8H3.5M7.5 3.5L3 8l4.5 4.5`), fixed-height `32px` `.set-search` input (`Icon::Search` + `256`-char `MAX_SETTINGS_SEARCH_CHARS` clamp + focus-within accent border + inline `Icon::Close` clear button), canonical `ALL_SETTINGS_GROUPS` (`AI` / `Workspace` / `Workflows` / `Insights` / `About`) uppercase headers rendering strictly caller-injected `SettingsNavEntry` sections (never advertising unimplemented pages), `.set-nav-item.on` active surface with accent-tinted `entry.icon`, bounded multi-word EN/FR search over titles/group labels/caller keywords while preserving `Space` typing inside search, self-contained `Ctrl+F` + fresh-press two-step `Escape` (clear query first, then `SettingsNavAction::Back`, holding `Escape` never double-fires, disabled & open-popup aware), and strictly focus-scoped `ArrowUp` / `ArrowDown` / `Enter` / `Space` navigation that never steals keys from controls in the Settings page body.
- `tests/ui_settings_nav.rs` — Standalone `egui` integration test suite verifying canonical group ordering, strict omission of uninjected pages, vector back-arrow/search/section icon geometry & active accent tinting, EN/FR multi-word search (including typed spaces, English title fallback in FR mode, `256`-char clamp, localized no-match message, and inline `×` clear click), held-key two-step `Escape`, external body focus isolation + `ui.add_enabled_ui(false)` modal disable guard, dynamic catalog mutation & `reset()`, and `200px` narrow rail single-line `'…'` truncation with zero warnings under `-D warnings`.
