//! Exact keyboard shortcut routing and searchable EN/FR shortcut reference.
//!
//! Transcribed directly from:
//! - `src/renderer/src/floatingTerminal.js` (`isFloatingToggleKey`)
//! - `src/renderer/src/App.vue` (`onKey`, `floatingPaneKey`, `typingInField`, `dialogOpen`)
//! - `src/renderer/src/components/TerminalPane.vue` (`isAppShortcut`, `attachCustomKeyEventHandler`)
//! - `src/renderer/src/components/EditorPane.vue` (`isAppShortcut`, `onKeydownCapture`)
//! - `src/renderer/src/components/HelpDialog.vue` (`onEscape`)
//! - `src/renderer/src/helpShortcuts.js` & `src/renderer/src/i18n/locales/fr/app.json`

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowDir {
    Left,
    Right,
    Up,
    Down,
}

impl ArrowDir {
    pub fn from_key_name(key: &str) -> Option<Self> {
        match key {
            "ArrowLeft" => Some(Self::Left),
            "ArrowRight" => Some(Self::Right),
            "ArrowUp" => Some(Self::Up),
            "ArrowDown" => Some(Self::Down),
            _ => None,
        }
    }
}

/// Platform-neutral keyboard event descriptor mirroring DOM `KeyboardEvent` fields
/// inspected by `App.vue`, `TerminalPane.vue`, `EditorPane.vue`, and `floatingTerminal.js`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    /// Logical key (`"e"`, `"="`, `"+"`, `"0"`, `"à"`, `"`"`, `"²"`, `"Escape"`, `"F1"`, `"ArrowLeft"`, `"PageUp"`, `" "`, etc.).
    pub key: String,
    /// Physical key code (`"Backquote"`, `"Digit0"`, `"Numpad0"`, `"NumpadAdd"`, `"NumpadSubtract"`, `"KeyZ"`, etc.).
    pub code: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub repeat: bool,
}

impl KeyInput {
    pub fn new(key: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            ..Self::default()
        }
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub fn with_meta(mut self) -> Self {
        self.meta = true;
        self
    }

    pub fn with_repeat(mut self) -> Self {
        self.repeat = true;
        self
    }
}

/// `Ctrl+\`` (the physical key left of `1`, whatever symbol the layout prints:
/// `²` on French AZERTY, `#` on Canadian French), without `Shift`, `Alt`, or `Meta`
/// (`floatingTerminal.js:isFloatingToggleKey`).
pub fn is_floating_toggle_key(e: &KeyInput) -> bool {
    if !e.ctrl || e.shift || e.alt || e.meta {
        return false;
    }
    e.code == "Backquote" || e.key == "`"
}

/// Exact `TerminalPane.vue:isAppShortcut` (`lines 2306-2319`):
/// keys withheld from the PTY so the shell does not receive escape sequences alongside Tessel's action.
pub fn is_terminal_app_shortcut(e: &KeyInput) -> bool {
    if is_floating_toggle_key(e) {
        return true;
    }
    if e.ctrl && e.shift && !e.alt {
        let lower = e.key.to_lowercase();
        return matches!(
            lower.as_str(),
            "e" | "o" | "w" | "b" | "k" | "n" | "f" | "r" | "p" | "x" | "g"
        );
    }
    if e.ctrl && !e.shift && !e.alt {
        return matches!(
            e.key.as_str(),
            "=" | "+" | "-" | "0" | "," | "PageUp" | "PageDown"
        ) || matches!(
            e.code.as_str(),
            "NumpadAdd" | "NumpadSubtract" | "Digit0" | "Numpad0"
        );
    }
    if e.alt && !e.ctrl && !e.shift {
        return ArrowDir::from_key_name(&e.key).is_some();
    }
    e.key == "F1"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKeyOutcome {
    OpenFind,
    CopySelection,
    PasteClipboard,
    AgentNewline,
    ScrollPageUp,
    ScrollPageDown,
    WithholdForApp,
    SendToPty,
}

/// Exact `TerminalPane.vue` key routing (`attachCustomKeyEventHandler`, `lines 3944-4010`).
pub fn route_terminal_key(e: &KeyInput, is_agent: bool) -> TerminalKeyOutcome {
    if e.ctrl && e.shift && e.key.eq_ignore_ascii_case("f") {
        return TerminalKeyOutcome::OpenFind;
    }
    if is_terminal_app_shortcut(e) {
        return TerminalKeyOutcome::WithholdForApp;
    }
    if e.key == "Enter" && e.shift && !e.ctrl && !e.alt && !e.meta && is_agent {
        return TerminalKeyOutcome::AgentNewline;
    }
    if e.ctrl && !e.shift && !e.alt && e.key.eq_ignore_ascii_case("v") {
        return TerminalKeyOutcome::PasteClipboard;
    }
    if e.ctrl && e.shift {
        if e.key.eq_ignore_ascii_case("c") {
            return TerminalKeyOutcome::CopySelection;
        }
        if e.key.eq_ignore_ascii_case("v") {
            return TerminalKeyOutcome::PasteClipboard;
        }
    }
    if e.shift && !e.ctrl && !e.alt && !e.meta {
        if e.key == "PageUp" {
            return TerminalKeyOutcome::ScrollPageUp;
        }
        if e.key == "PageDown" {
            return TerminalKeyOutcome::ScrollPageDown;
        }
    }
    TerminalKeyOutcome::SendToPty
}

/// Exact `EditorPane.vue:isAppShortcut` (`lines 838-846`):
/// note `Alt+ArrowUp` and `Alt+ArrowDown` stay inside the editor (move line),
/// whereas `Alt+ArrowLeft` and `Alt+ArrowRight` move between panes.
pub fn is_editor_app_shortcut(e: &KeyInput) -> bool {
    if is_floating_toggle_key(e) {
        return true;
    }
    if e.ctrl && e.shift && !e.alt {
        let lower = e.key.to_lowercase();
        return matches!(
            lower.as_str(),
            "e" | "o" | "w" | "b" | "k" | "n" | "t" | "r" | "p" | "x" | "j" | "g" | " "
        );
    }
    if e.ctrl && !e.shift && !e.alt {
        return matches!(
            e.key.as_str(),
            "=" | "+" | "-" | "0" | "," | "PageUp" | "PageDown"
        ) || matches!(
            e.code.as_str(),
            "NumpadAdd" | "NumpadSubtract" | "Digit0" | "Numpad0"
        );
    }
    if e.alt && !e.ctrl && !e.shift {
        return matches!(e.key.as_str(), "ArrowLeft" | "ArrowRight");
    }
    e.key == "F1"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKeyOutcome {
    Save,
    CloseTab,
    ToggleWordWrap,
    JumpDiff { previous: bool },
    ConsumedRepeat,
    ForwardToApp,
    LetEditorHandle,
}

/// Exact `EditorPane.vue:onKeydownCapture` (`lines 847-883`).
pub fn route_editor_key(e: &KeyInput, in_diff_comment: bool, show_diff: bool) -> EditorKeyOutcome {
    if in_diff_comment && !(e.ctrl && e.key.eq_ignore_ascii_case("s")) {
        return EditorKeyOutcome::LetEditorHandle;
    }
    let ctrl_only = e.ctrl && !e.shift && !e.alt && !e.meta;
    if ctrl_only && e.key.eq_ignore_ascii_case("s") {
        return if e.repeat {
            EditorKeyOutcome::ConsumedRepeat
        } else {
            EditorKeyOutcome::Save
        };
    }
    if ctrl_only && e.key.eq_ignore_ascii_case("w") {
        return if e.repeat {
            EditorKeyOutcome::ConsumedRepeat
        } else {
            EditorKeyOutcome::CloseTab
        };
    }
    if e.alt && !e.ctrl && !e.shift && (e.key.eq_ignore_ascii_case("z") || e.code == "KeyZ") {
        return if e.repeat {
            EditorKeyOutcome::ConsumedRepeat
        } else {
            EditorKeyOutcome::ToggleWordWrap
        };
    }
    if e.key == "F7" && !e.ctrl && !e.alt && show_diff {
        return if e.repeat {
            EditorKeyOutcome::ConsumedRepeat
        } else {
            EditorKeyOutcome::JumpDiff { previous: e.shift }
        };
    }
    if is_editor_app_shortcut(e) {
        return EditorKeyOutcome::ForwardToApp;
    }
    EditorKeyOutcome::LetEditorHandle
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpEscapeOutcome {
    ClearQuery,
    CloseDialog,
}

/// Exact `HelpDialog.vue:onEscape` (`lines 23-26`).
pub fn route_help_dialog_escape(query: &str) -> HelpEscapeOutcome {
    if !query.is_empty() {
        HelpEscapeOutcome::ClearQuery
    } else {
        HelpEscapeOutcome::CloseDialog
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppKeyContext {
    pub from_editor: bool,
    /// True when focus is in a UI `<input>`, `<textarea>`, `<select>`, `[contenteditable]`,
    /// or inside `.editor-pane` without `from_editor` (`App.vue:typingInField`, `lines 14135-14143`).
    /// False when focus is in a terminal (`.xterm`).
    pub typing_in_field: bool,
    /// True when focus is inside `.floating-term` (`App.vue:floatingPaneKey`, `lines 14172-14190`).
    pub in_floating_terminal: bool,
    /// True when any modal/dialog/launcher is open (`App.vue:dialogOpen`, `lines 14112-14131`).
    pub dialog_open: bool,
    pub side_panel_fullscreen: bool,
    pub focus_in_side_panel_or_body: bool,
    pub has_maximized_pane: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppShortcut {
    None,
    ConsumedNoop,
    ToggleFloatingTerminal,
    FloatingClose,
    FloatingRestart,
    CloseSettings,
    CloseHelp,
    SplitRight,
    SplitDown,
    CloseActivePane,
    ToggleBroadcast,
    ToggleSideTasks,
    ToggleSideFiles,
    ToggleSideChanges,
    CreateWorkspace,
    NewDefaultTerminal,
    OpenLauncherCentered,
    RestartActivePane,
    ToggleCommandPalette,
    OpenFileFinder,
    Zoom(i8),
    MoveFocus(ArrowDir),
    ToggleSettings,
    ToggleHelp,
    CycleWorkspace(i8),
    ExitSidePanelFullscreen,
    /// Mirrors `App.vue:14287-14299`: restores `maximized_id` if set and closes non-busy dialogs/menus,
    /// WITHOUT `preventDefault()` so an active terminal PTY still receives `ESC` (`\x1b`).
    EscapeCleanup {
        restore_maximized: bool,
    },
}

/// Exact window `onKey(e, opts)` state machine (`App.vue:14192-14300` + `SidePanel.vue:211-218`).
pub fn route_app_key(e: &KeyInput, ctx: AppKeyContext) -> AppShortcut {
    if is_floating_toggle_key(e) && !ctx.dialog_open {
        return if e.repeat {
            AppShortcut::ConsumedNoop
        } else {
            AppShortcut::ToggleFloatingTerminal
        };
    }

    if !ctx.from_editor && ctx.typing_in_field && e.key != "Escape" && e.key != "F1" {
        return AppShortcut::None;
    }

    if ctx.in_floating_terminal {
        if e.ctrl && e.shift && !e.alt {
            let k = e.key.to_lowercase();
            match k.as_str() {
                "w" => return AppShortcut::FloatingClose,
                "r" => return AppShortcut::FloatingRestart,
                "e" | "o" | "b" => return AppShortcut::ConsumedNoop,
                _ => {}
            }
        }
        if e.alt && !e.ctrl && !e.shift && ArrowDir::from_key_name(&e.key).is_some() {
            return AppShortcut::ConsumedNoop;
        }
    }

    if ctx.dialog_open {
        if e.ctrl && !e.shift && !e.alt && e.key == "," {
            return AppShortcut::CloseSettings;
        }
        if e.key == "F1" {
            return AppShortcut::CloseHelp;
        }
        if e.key != "Escape" {
            return AppShortcut::None;
        }
    }

    if e.ctrl && e.shift {
        let k = e.key.to_lowercase();
        match k.as_str() {
            "e" => return AppShortcut::SplitRight,
            "o" => return AppShortcut::SplitDown,
            "w" => return AppShortcut::CloseActivePane,
            "b" => return AppShortcut::ToggleBroadcast,
            "k" => return AppShortcut::ToggleSideTasks,
            "x" => return AppShortcut::ToggleSideFiles,
            "g" => return AppShortcut::ToggleSideChanges,
            "n" => return AppShortcut::CreateWorkspace,
            "t" => return AppShortcut::NewDefaultTerminal,
            " " => return AppShortcut::OpenLauncherCentered,
            "r" => return AppShortcut::RestartActivePane,
            "p" => return AppShortcut::ToggleCommandPalette,
            "j" => return AppShortcut::OpenFileFinder,
            _ => {}
        }
    }

    if e.ctrl && !e.shift && !e.alt {
        if e.key == "=" || e.key == "+" || e.code == "NumpadAdd" {
            return AppShortcut::Zoom(1);
        }
        if e.key == "-" || e.code == "NumpadSubtract" {
            return AppShortcut::Zoom(-1);
        }
        if e.key == "0" || e.code == "Digit0" || e.code == "Numpad0" {
            return AppShortcut::Zoom(0);
        }
        if e.key == "," {
            return AppShortcut::ToggleSettings;
        }
        if e.key == "PageUp" {
            return AppShortcut::CycleWorkspace(-1);
        }
        if e.key == "PageDown" {
            return AppShortcut::CycleWorkspace(1);
        }
    }

    if e.alt
        && !e.ctrl
        && !e.shift
        && let Some(dir) = ArrowDir::from_key_name(&e.key)
    {
        return AppShortcut::MoveFocus(dir);
    }

    if e.key == "F1" {
        return AppShortcut::ToggleHelp;
    }

    if e.key == "Escape" {
        if !ctx.dialog_open && ctx.side_panel_fullscreen && ctx.focus_in_side_panel_or_body {
            return AppShortcut::ExitSidePanelFullscreen;
        }
        return AppShortcut::EscapeCleanup {
            restore_maximized: ctx.has_maximized_pane,
        };
    }

    AppShortcut::None
}

// --- HelpDialog & helpShortcuts.js EN/FR reference & accent-insensitive search -------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutGroupSpec {
    pub title: &'static str,
    pub rows: Vec<(&'static str, &'static str)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilteredShortcutRow {
    pub keys: &'static str,
    pub text: &'static str,
    pub chords: Vec<Vec<&'static str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilteredShortcutGroup {
    pub title: &'static str,
    pub rows: Vec<FilteredShortcutRow>,
}

/// Keycap translation matching `helpShortcuts.js:keyName` and `locales/fr/app.json:88-97`.
pub fn key_name(key: &str, french: bool) -> &str {
    match (key, french) {
        ("Ctrl", _) => "Ctrl",
        ("Shift", false) => "Shift",
        ("Shift", true) => "Maj",
        ("Alt", _) => "Alt",
        ("Esc", false) => "Esc",
        ("Esc", true) => "Échap",
        ("Space", false) => "Space",
        ("Space", true) => "Espace",
        ("PageUp", false) => "Page Up",
        ("PageUp", true) => "Pg préc",
        ("PageDown", false) => "Page Down",
        ("PageDown", true) => "Pg suiv",
        ("Arrows", false) => "Arrow keys",
        ("Arrows", true) => "Flèches",
        _ => key,
    }
}

pub fn chords_of(keys: &'static str, french: bool) -> Vec<Vec<&'static str>> {
    keys.split(" / ")
        .map(|chord| {
            chord
                .split('+')
                .map(|k| match (k, french) {
                    ("Ctrl", _) => "Ctrl",
                    ("Shift", false) => "Shift",
                    ("Shift", true) => "Maj",
                    ("Alt", _) => "Alt",
                    ("Esc", false) => "Esc",
                    ("Esc", true) => "Échap",
                    ("Space", false) => "Space",
                    ("Space", true) => "Espace",
                    ("PageUp", false) => "Page Up",
                    ("PageUp", true) => "Pg préc",
                    ("PageDown", false) => "Page Down",
                    ("PageDown", true) => "Pg suiv",
                    ("Arrows", false) => "Arrow keys",
                    ("Arrows", true) => "Flèches",
                    (other, _) => other,
                })
                .collect()
        })
        .collect()
}

/// Complete 5-group, 38-shortcut help specification (`helpShortcuts.js` + `locales/fr/app.json`).
pub fn shortcut_groups(french: bool) -> Vec<ShortcutGroupSpec> {
    if !french {
        vec![
            ShortcutGroupSpec {
                title: "Panes",
                rows: vec![
                    (
                        "Ctrl+Shift+T",
                        "New pane: the default agent, or the default shell",
                    ),
                    ("Ctrl+Shift+Space", "Open a terminal or agent"),
                    (
                        "Ctrl+Shift+P",
                        "Command palette: find panes, workspaces, commands",
                    ),
                    ("Ctrl+Shift+J", "Jump to a file of the workspace's project"),
                    ("Ctrl+Shift+E", "Split right"),
                    ("Ctrl+Shift+O", "Split down"),
                    ("Ctrl+Shift+W", "Close pane"),
                    ("Ctrl+Shift+R", "Restart pane"),
                    ("Alt+Arrows", "Move between panes"),
                    ("Esc", "Restore a maximized pane"),
                    (
                        "Ctrl+`",
                        "Show or hide the floating terminal (the key left of 1)",
                    ),
                ],
            },
            ShortcutGroupSpec {
                title: "Terminal",
                rows: vec![
                    ("Ctrl+Shift+F", "Find"),
                    ("Ctrl+Shift+C", "Copy"),
                    ("Ctrl+Shift+V", "Paste"),
                    ("Ctrl+=", "Bigger text"),
                    ("Ctrl+-", "Smaller text"),
                    ("Ctrl+0", "Reset text size"),
                    ("Shift+PageUp", "Scroll up in the pane"),
                    ("Shift+PageDown", "Scroll down in the pane"),
                ],
            },
            ShortcutGroupSpec {
                title: "Workspaces",
                rows: vec![
                    ("Ctrl+Shift+N", "New workspace"),
                    ("Ctrl+PageUp", "Previous workspace"),
                    ("Ctrl+PageDown", "Next workspace"),
                ],
            },
            ShortcutGroupSpec {
                title: "Editor",
                rows: vec![
                    ("Ctrl+S", "Save the file"),
                    ("Ctrl+W", "Close the editor tab"),
                    ("Ctrl+F", "Find in the file"),
                    ("Ctrl+H", "Replace"),
                    ("Ctrl+G", "Go to line"),
                    ("Alt+Z", "Word wrap on or off"),
                    ("F7 / Shift+F7", "Next / previous change (a diff)"),
                    (
                        "Ctrl+Shift+A",
                        "Add Review Note (a diff: the selected lines)",
                    ),
                ],
            },
            ShortcutGroupSpec {
                title: "App",
                rows: vec![
                    ("Ctrl+Shift+B", "Broadcast typing to all panes"),
                    (
                        "Alt+A",
                        "In a chat pane: go to the request waiting for your answer",
                    ),
                    ("Ctrl+Shift+K", "Task board"),
                    ("Ctrl+Shift+X", "File explorer"),
                    ("Ctrl+Shift+G", "Source Control (the git changes)"),
                    ("Ctrl+,", "Settings"),
                    ("Win+H", "Voice typing (Windows)"),
                    ("F1", "This help"),
                ],
            },
        ]
    } else {
        vec![
            ShortcutGroupSpec {
                title: "Panneaux",
                rows: vec![
                    (
                        "Ctrl+Shift+T",
                        "Nouveau panneau : l'agent par défaut, ou le shell par défaut",
                    ),
                    ("Ctrl+Shift+Space", "Ouvrir un terminal ou un agent"),
                    (
                        "Ctrl+Shift+P",
                        "Palette de commandes : trouver panneaux, espaces de travail, commandes",
                    ),
                    (
                        "Ctrl+Shift+J",
                        "Aller à un fichier du projet de l'espace de travail",
                    ),
                    ("Ctrl+Shift+E", "Scinder vers la droite"),
                    ("Ctrl+Shift+O", "Scinder vers le bas"),
                    ("Ctrl+Shift+W", "Fermer le panneau"),
                    ("Ctrl+Shift+R", "Redémarrer le panneau"),
                    ("Alt+Arrows", "Passer d'un panneau à l'autre"),
                    ("Esc", "Restaurer un panneau agrandi"),
                    (
                        "Ctrl+`",
                        "Afficher ou masquer le terminal flottant (la touche à gauche de 1)",
                    ),
                ],
            },
            ShortcutGroupSpec {
                title: "Terminal",
                rows: vec![
                    ("Ctrl+Shift+F", "Rechercher"),
                    ("Ctrl+Shift+C", "Copier"),
                    ("Ctrl+Shift+V", "Coller"),
                    ("Ctrl+=", "Texte plus grand"),
                    ("Ctrl+-", "Texte plus petit"),
                    ("Ctrl+0", "Taille de texte par défaut"),
                    ("Shift+PageUp", "Défiler vers le haut dans le panneau"),
                    ("Shift+PageDown", "Défiler vers le bas dans le panneau"),
                ],
            },
            ShortcutGroupSpec {
                title: "Espaces de travail",
                rows: vec![
                    ("Ctrl+Shift+N", "Nouvel espace de travail"),
                    ("Ctrl+PageUp", "Espace de travail précédent"),
                    ("Ctrl+PageDown", "Espace de travail suivant"),
                ],
            },
            ShortcutGroupSpec {
                title: "Éditeur",
                rows: vec![
                    ("Ctrl+S", "Enregistrer le fichier"),
                    ("Ctrl+W", "Fermer l'onglet de l'éditeur"),
                    ("Ctrl+F", "Rechercher dans le fichier"),
                    ("Ctrl+H", "Remplacer"),
                    ("Ctrl+G", "Aller à la ligne"),
                    ("Alt+Z", "Retour à la ligne activé ou désactivé"),
                    (
                        "F7 / Shift+F7",
                        "Modification suivante / précédente (un diff)",
                    ),
                    (
                        "Ctrl+Shift+A",
                        "Ajouter une note de revue (un diff : les lignes sélectionnées)",
                    ),
                ],
            },
            ShortcutGroupSpec {
                title: "Application",
                rows: vec![
                    ("Ctrl+Shift+B", "Diffuser la saisie à tous les panneaux"),
                    (
                        "Alt+A",
                        "Dans un panneau de chat : aller à la demande qui attend votre réponse",
                    ),
                    ("Ctrl+Shift+K", "Tableau des tâches"),
                    ("Ctrl+Shift+X", "Explorateur de fichiers"),
                    (
                        "Ctrl+Shift+G",
                        "Contrôle de code source (les modifications git)",
                    ),
                    ("Ctrl+,", "Paramètres"),
                    ("Win+H", "Saisie vocale (Windows)"),
                    ("F1", "Cette aide"),
                ],
            },
        ]
    }
}

/// Folds Latin diacritics and case to match `helpShortcuts.js:fold` (`NFD` + strip `\p{M}` + `toLowerCase`).
pub fn fold_for_search(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        // Skip combining diacritical marks if input is already decomposed.
        if ('\u{0300}'..='\u{036f}').contains(&ch) {
            continue;
        }
        for lc in ch.to_lowercase() {
            let mapped = match lc {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
                'ç' => 'c',
                'è' | 'é' | 'ê' | 'ë' => 'e',
                'ì' | 'í' | 'î' | 'ï' => 'i',
                'ñ' => 'n',
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
                'ù' | 'ú' | 'û' | 'ü' => 'u',
                'ý' | 'ÿ' => 'y',
                other => other,
            };
            out.push(mapped);
        }
    }
    out
}

/// Filters shortcut groups by action description, group title, or localized keycaps
/// (`"maj+t"`, `"echap"`, `"ctrl t"`, etc. — `helpShortcuts.js:filterShortcuts`).
pub fn filter_shortcuts(french: bool, query: &str) -> Vec<FilteredShortcutGroup> {
    let folded_query = fold_for_search(query);
    let words: Vec<&str> = folded_query
        .split(|c: char| c.is_whitespace() || c == '+')
        .filter(|w| !w.is_empty())
        .collect();
    shortcut_groups(french)
        .into_iter()
        .filter_map(|group| {
            let rows: Vec<FilteredShortcutRow> = group
                .rows
                .into_iter()
                .map(|(keys, text)| FilteredShortcutRow {
                    keys,
                    text,
                    chords: chords_of(keys, french),
                })
                .filter(|row| {
                    if words.is_empty() {
                        return true;
                    }
                    let chords_text = row
                        .chords
                        .iter()
                        .map(|c| c.join(" "))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let hay =
                        fold_for_search(&format!("{} {} {}", group.title, row.text, chords_text));
                    words.iter().all(|w| hay.contains(w))
                })
                .collect();
            (!rows.is_empty()).then_some(FilteredShortcutGroup {
                title: group.title,
                rows,
            })
        })
        .collect()
}
