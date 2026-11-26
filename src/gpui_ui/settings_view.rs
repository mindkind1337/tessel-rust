//! Native Settings workspace and page controls.

use super::{
    FocusInputTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    side_panels_view::render_focus_input_box,
    theme::{GpuiThemeTokens, TOOLBAR_HEIGHT_PX, mono_font_family},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiSettingsSnapshot, GpuiWorkspaceSnapshot},
    settings_extra_model::{
        AlertSound, ContextModelFamily, DEVTOOLS_CATALOG, MCP_CATALOG, TEAM_TOOLS_REFERENCE,
        TESSEL_CONNECTORS, WorkerMode, supported_voice_languages, winget_install_command,
    },
    settings_extra_panel::SettingsExtraAction,
    settings_pages_panel::{SettingsPageAction, preview_branch_name},
    theme_palettes::THEMES,
    workspace::{
        BranchPrefixMode, OpenLinksTarget, RendererPreference, ScmOrderSetting, ScmViewModeSetting,
        TerminalCursorStyle, UsagePercentDisplay,
    },
};
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

pub(crate) fn render_settings_modal(
    viewport_width: f32,
    snapshot: &GpuiWorkspaceSnapshot,
    modal_query: &super::modal_query_input::Input,
    swatches: &[gpui::FocusHandle; 8],
    focus_input: FocusInputTarget,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<AnyElement> {
    let s = snapshot.settings.as_ref()?;
    let french = snapshot.french;

    let back_label = if french {
        "Retour à l'application"
    } else {
        "Back to app"
    };
    let back = div()
        .id("settings-back")
        .debug_selector(|| "settings-back".into())
        .role(gpui::Role::Button)
        .aria_label(back_label)
        .tab_index(0)
        .focus_visible(|style| style.bg(tokens.surface_2).text_color(tokens.text_strong))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(7.))
        .rounded(px(tokens.radius_px))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(tokens.text_dim)
        .hover(|style| style.bg(tokens.surface_2).text_color(tokens.text_strong))
        .on_click(cx.listener(|root, _, _, cx| root.dispatch_action(GpuiAction::CloseOverlays, cx)))
        .on_key_down(cx.listener(|root, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                root.dispatch_action(GpuiAction::CloseOverlays, cx);
            }
        }))
        .child(GpuiIcon::ArrowLeft.render(14., tokens.text_dim))
        .child(back_label);
    let mut nav_scroll = div()
        .id("gpui-settings-nav-scroll")
        .debug_selector(|| "settings-nav".into())
        .role(gpui::Role::Navigation)
        .aria_label(if french {
            "Pages des paramètres"
        } else {
            "Settings pages"
        })
        .w(px((viewport_width * 0.24).clamp(200., 290.)))
        .h_full()
        .flex_none()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(12.))
        .pt(px(14.))
        .pb(px(18.))
        .bg(tokens.chrome)
        .border_r_1()
        .border_color(tokens.border)
        .child(back)
        .child(div().flex_none().child(modal_query.render(
            FocusInputTarget::SettingsSearch,
            french,
            tokens,
            cx,
        )));

    for (g_idx, (group_title, pages)) in s.groups.iter().enumerate() {
        let mut grp = div().flex_none().flex().flex_col().gap(px(2.0)).child(
            div()
                .px(px(6.0))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tokens.text_dim)
                .child(SharedString::from(group_title.to_uppercase())),
        );
        for (p_idx, (page_id, page_label)) in pages.iter().enumerate() {
            let active = s.active_page_id == *page_id;
            let pid = page_id.clone();
            grp = grp.child(
                div()
                    .id(SharedString::from(format!(
                        "gpui-settings-nav-{g_idx}-{p_idx}"
                    )))
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(if active {
                        tokens.accent_soft
                    } else {
                        gpui::rgba(0x0000_0000)
                    })
                    .text_size(px(12.0))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        tokens.text_strong
                    } else {
                        tokens.text_dim
                    })
                    .hover(|st| st.bg(tokens.surface_2).text_color(tokens.text_strong))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch_action(GpuiAction::OpenSettingsPage(pid.clone()), cx);
                    }))
                    .child(SharedString::from(page_label.clone())),
            );
        }
        nav_scroll = nav_scroll.child(grp);
    }

    #[cfg(windows)]
    {
        let active = s.active_page_id == "passwords";
        nav_scroll = nav_scroll.child(
            div()
                .id("gpui-settings-nav-passwords")
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(if active {
                    tokens.accent_soft
                } else {
                    gpui::rgba(0x0000_0000)
                })
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(12.0))
                .text_color(if active {
                    tokens.text_strong
                } else {
                    tokens.text_dim
                })
                .hover(|st| st.bg(tokens.surface_2))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dispatch_action(GpuiAction::OpenSettingsPage("passwords".into()), cx);
                }))
                .child(GpuiIcon::Lock.render(12.0, tokens.accent))
                .child(if french {
                    "Mots de passe"
                } else {
                    "Saved Passwords"
                }),
        );
    }

    let page_body = render_settings_page_body(snapshot, s, focus_input, swatches, tokens, cx);
    let wide = s.active_page_id == "skills" && s.search_query.trim().is_empty();
    let (horizontal, top, bottom) = if viewport_width <= 760. {
        (20., 24., 40.)
    } else {
        (40., 34., 60.)
    };
    let right_col = div()
        .id(SharedString::from(format!(
            "settings-body-{}",
            s.active_page_id
        )))
        .debug_selector(|| "settings-body".into())
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .child(
            div().w_full().flex().justify_center().child(
                div()
                    .w_full()
                    .min_w_0()
                    .max_w(px(if wide { 1180. } else { 870. }))
                    .px(px(horizontal))
                    .pt(px(top))
                    .pb(px(bottom))
                    .child(page_body),
            ),
        );
    Some(
        div()
            .id("gpui-settings-page")
            .debug_selector(|| "settings-page".into())
            .role(gpui::Role::Dialog)
            .aria_label(if french { "Paramètres" } else { "Settings" })
            .aria_modal(true)
            .absolute()
            .top(px(TOOLBAR_HEIGHT_PX))
            .bottom_0()
            .left_0()
            .right_0()
            .occlude()
            .bg(tokens.surface)
            .flex()
            .overflow_hidden()
            .child(nav_scroll)
            .child(right_col)
            .into_any_element(),
    )
}

fn render_settings_page_body(
    snapshot: &GpuiWorkspaceSnapshot,
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    swatches: &[gpui::FocusHandle; 8],
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let french = snapshot.french;
    match s.active_page_id.as_str() {
        "appearance" => render_appearance_page(snapshot, tokens, cx),
        "general" => render_general_page(snapshot, s, tokens, cx),
        "terminal" => render_terminal_page(s, french, tokens, cx),
        "editor" => render_editor_page(s, french, tokens, cx),
        "browser" => render_browser_page(s, french, tokens, cx),
        "passwords" => render_passwords_page(s, french, tokens, cx),
        "git" => render_git_page(s, french, tokens, cx),
        "agents" => render_agents_page(s, focus_input, swatches, french, tokens, cx),
        "accounts" => render_accounts_page(s, focus_input, french, tokens, cx),
        "skills" => render_skills_page(s, focus_input, french, tokens, cx),
        "mcp" => render_mcp_page(s, french, tokens, cx),
        "connections" => render_connections_page(french, tokens, cx),
        "context" => render_context_page(s, french, tokens, cx),
        "orchestration" => render_orchestration_page(s, french, tokens, cx),
        "alerts" => render_alerts_page(s, french, tokens, cx),
        "voice" => render_voice_page(s, french, tokens, cx),
        "devtools" => render_devtools_page(s, french, tokens, cx),
        "automations" => render_automations_page(s, focus_input, french, tokens, cx),
        "quick-commands" => render_quick_commands_page(s, focus_input, french, tokens, cx),
        "ssh" => render_ssh_page(s, focus_input, french, tokens, cx),
        "tessel-account" => render_tessel_account_page(s, french, tokens, cx),
        "stats" => render_stats_page(s, french, tokens, cx),
        "updates" => render_updates_page(snapshot, tokens, cx),
        _ => render_general_page(snapshot, s, tokens, cx),
    }
}

fn render_appearance_page(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let french = snapshot.french;

    let mut theme_grid = div().flex().flex_wrap().gap(px(6.0));
    for tid in THEMES {
        let active = tid == snapshot.theme;
        theme_grid = theme_grid.child(
            div()
                .id(SharedString::from(format!(
                    "gpui-st-theme-{}",
                    tid.as_str()
                )))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(tokens.radius_px))
                .bg(if active {
                    tokens.accent_soft
                } else {
                    tokens.surface
                })
                .border_1()
                .border_color(if active { tokens.accent } else { tokens.border })
                .text_size(px(11.5))
                .text_color(if active {
                    tokens.text_strong
                } else {
                    tokens.text_dim
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_action(GpuiAction::SetTheme(tid), cx);
                }))
                .child(tid.label(french)),
        );
    }

    let renderer_section = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(section_title(
            if french {
                "Moteur de rendu UI"
            } else {
                "UI Renderer"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-rend-gpui",
                    RendererPreference::AutoGpui.label(french).to_owned(),
                    snapshot.renderer == RendererPreference::AutoGpui,
                    tokens,
                    GpuiAction::SetRenderer(RendererPreference::AutoGpui),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-rend-compat",
                    RendererPreference::CompatEgui.label(french).to_owned(),
                    snapshot.renderer == RendererPreference::CompatEgui,
                    tokens,
                    GpuiAction::SetRenderer(RendererPreference::CompatEgui),
                    cx,
                )),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(tokens.text_dim)
                .child(snapshot.renderer.description(french)),
        );

    let font_section = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Taille de police"
            } else {
                "Font size"
            },
            tokens,
        ))
        .child(setting_btn(
            "gpui-st-font-dec",
            "−".to_owned(),
            false,
            tokens,
            GpuiAction::ZoomFont(-1),
            cx,
        ))
        .child(
            div()
                .font_family(mono_font_family())
                .text_size(px(12.0))
                .text_color(tokens.text_strong)
                .child(SharedString::from(format!("{:.0} px", snapshot.font_size))),
        )
        .child(setting_btn(
            "gpui-st-font-inc",
            "+".to_owned(),
            false,
            tokens,
            GpuiAction::ZoomFont(1),
            cx,
        ))
        .child(setting_btn(
            "gpui-st-font-rst",
            "Reset".to_owned(),
            false,
            tokens,
            GpuiAction::ZoomFont(0),
            cx,
        ));

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(section_title(
            if french {
                "Thème de couleurs (12 palettes)"
            } else {
                "Color Theme (12 palettes)"
            },
            tokens,
        ))
        .child(theme_grid)
        .child(renderer_section)
        .child(font_section)
        .into_any_element()
}

fn render_general_page(
    snapshot: &GpuiWorkspaceSnapshot,
    s: &GpuiSettingsSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let french = snapshot.french;
    let gp = &s.general_prefs;

    let mut p_restore = gp.clone();
    p_restore.restore_workspaces = !p_restore.restore_workspaces;
    let mut p_close = gp.clone();
    p_close.confirm_close_agent = !p_close.confirm_close_agent;
    let mut p_quit = gp.clone();
    p_quit.confirm_quit_running = !p_quit.confirm_quit_running;
    let mut p_del = gp.clone();
    p_del.confirm_delete_workspace = !p_del.confirm_delete_workspace;

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french { "Langue" } else { "Language" },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-lang-en",
                    "English".to_owned(),
                    !french,
                    tokens,
                    GpuiAction::SetLanguage { french: false },
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-lang-fr",
                    "Français".to_owned(),
                    french,
                    tokens,
                    GpuiAction::SetLanguage { french: true },
                    cx,
                )),
        )
        .child(toggle_row(
            "gpui-st-gen-restore",
            if french {
                "Restaurer les espaces de travail au démarrage"
            } else {
                "Restore workspaces on startup"
            },
            gp.restore_workspaces,
            tokens,
            GpuiAction::UpdateGeneralPrefs(p_restore),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-gen-close",
            if french {
                "Confirmer avant de fermer un agent actif"
            } else {
                "Confirm before closing a running agent"
            },
            gp.confirm_close_agent,
            tokens,
            GpuiAction::UpdateGeneralPrefs(p_close),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-gen-quit",
            if french {
                "Confirmer avant de quitter l'application"
            } else {
                "Confirm before quitting with active agents"
            },
            gp.confirm_quit_running,
            tokens,
            GpuiAction::UpdateGeneralPrefs(p_quit),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-gen-del",
            if french {
                "Confirmer la suppression d'espace de travail"
            } else {
                "Confirm workspace removal"
            },
            gp.confirm_delete_workspace,
            tokens,
            GpuiAction::UpdateGeneralPrefs(p_del),
            cx,
        ))
        .child(
            div()
                .pt(px(6.0))
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-gen-export",
                    "Export settings".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::ExportSettings),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-gen-import",
                    "Import settings".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::ImportSettings),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-gen-data",
                    "Open data folder".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::OpenDataFolder),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-gen-diag",
                    "Copy diagnostics".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::CopyDiagnostics),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-gen-reset",
                    "Reset all".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::ResetAllSettings),
                    cx,
                )),
        )
        .when(!s.backup_status.is_empty(), |el| {
            el.child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.accent)
                    .child(SharedString::from(s.backup_status.clone())),
            )
        })
        .into_any_element()
}

fn render_terminal_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let tp = s.terminal_prefs;
    let mut tp_block = tp;
    tp_block.cursor_style = TerminalCursorStyle::Block;
    let mut tp_bar = tp;
    tp_bar.cursor_style = TerminalCursorStyle::Bar;
    let mut tp_under = tp;
    tp_under.cursor_style = TerminalCursorStyle::Underline;

    let mut tp_blink = tp;
    tp_blink.cursor_blink = !tp_blink.cursor_blink;
    let mut tp_tile = tp;
    tp_tile.tile_new_panes = !tp_tile.tile_new_panes;
    let mut tp_copy = tp;
    tp_copy.copy_on_select = !tp_copy.copy_on_select;
    let mut tp_paste = tp;
    tp_paste.right_click_paste = !tp_paste.right_click_paste;
    let mut tp_multi = tp;
    tp_multi.confirm_multiline_paste = !tp_multi.confirm_multiline_paste;
    let mut tp_focus = tp;
    tp_focus.focus_follows_mouse = !tp_focus.focus_follows_mouse;

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Shell par défaut"
            } else {
                "Preferred Shell"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-sh-default",
                    "Auto".to_owned(),
                    s.preferred_shell.is_none(),
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::SelectPreferredShell(None)),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-sh-pwsh",
                    "PowerShell".to_owned(),
                    s.preferred_shell.as_deref() == Some("powershell"),
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::SelectPreferredShell(Some(
                        "powershell".into(),
                    ))),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-sh-cmd",
                    "Command Prompt".to_owned(),
                    s.preferred_shell.as_deref() == Some("cmd"),
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::SelectPreferredShell(Some(
                        "cmd".into(),
                    ))),
                    cx,
                ))
                .when(s.wsl_available, |el| {
                    el.child(setting_btn(
                        "gpui-st-sh-wsl",
                        "WSL".to_owned(),
                        s.preferred_shell.as_deref() == Some("wsl"),
                        tokens,
                        GpuiAction::SettingsPage(SettingsPageAction::SelectPreferredShell(Some(
                            "wsl".into(),
                        ))),
                        cx,
                    ))
                }),
        )
        .child(section_title(
            if french {
                "Style du curseur"
            } else {
                "Cursor Style"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-cur-block",
                    "Block".to_owned(),
                    tp.cursor_style == TerminalCursorStyle::Block,
                    tokens,
                    GpuiAction::UpdateTerminalPrefs(tp_block),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-cur-bar",
                    "Bar".to_owned(),
                    tp.cursor_style == TerminalCursorStyle::Bar,
                    tokens,
                    GpuiAction::UpdateTerminalPrefs(tp_bar),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-cur-under",
                    "Underline".to_owned(),
                    tp.cursor_style == TerminalCursorStyle::Underline,
                    tokens,
                    GpuiAction::UpdateTerminalPrefs(tp_under),
                    cx,
                )),
        )
        .child(toggle_row(
            "gpui-st-tp-blink",
            "Cursor blink",
            tp.cursor_blink,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_blink),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-tp-tile",
            "Auto-tile new panes",
            tp.tile_new_panes,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_tile),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-tp-copy",
            "Copy on selection",
            tp.copy_on_select,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_copy),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-tp-paste",
            "Right-click to paste",
            tp.right_click_paste,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_paste),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-tp-multi",
            "Confirm multiline paste",
            tp.confirm_multiline_paste,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_multi),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-tp-focus",
            "Focus follows mouse",
            tp.focus_follows_mouse,
            tokens,
            GpuiAction::UpdateTerminalPrefs(tp_focus),
            cx,
        ))
        .into_any_element()
}

fn render_editor_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let ep = s.editor_prefs;
    let mut e_syn = ep;
    e_syn.syntax_highlighting = !e_syn.syntax_highlighting;
    let mut e_ln = ep;
    e_ln.line_numbers = !e_ln.line_numbers;
    let mut e_wrap = ep;
    e_wrap.word_wrap = !e_wrap.word_wrap;
    let mut e_dwrap = ep;
    e_dwrap.diff_word_wrap = !e_dwrap.diff_word_wrap;
    let mut e_prev = ep;
    e_prev.preview_tabs = !e_prev.preview_tabs;
    let mut e_dot = ep;
    e_dot.show_git_ignored = !e_dot.show_git_ignored;
    let mut e_cmp = ep;
    e_cmp.compact_folders = !e_cmp.compact_folders;
    let mut e_save = ep;
    e_save.auto_save = !e_save.auto_save;
    let mut e_tab2 = ep;
    e_tab2.tab_size = 2;
    let mut e_tab4 = ep;
    e_tab4.tab_size = 4;

    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Éditeur & Explorateur"
            } else {
                "Editor & Explorer"
            },
            tokens,
        ))
        .child(toggle_row(
            "gpui-st-ed-syn",
            "Syntax highlighting",
            ep.syntax_highlighting,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_syn),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-ln",
            "Line numbers",
            ep.line_numbers,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_ln),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-wrap",
            "Word wrap",
            ep.word_wrap,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_wrap),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-dwrap",
            "Diff word wrap",
            ep.diff_word_wrap,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_dwrap),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-prev",
            "Enable preview tabs",
            ep.preview_tabs,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_prev),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-dot",
            "Show dotfiles & git-ignored entries",
            ep.show_git_ignored,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_dot),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-cmp",
            "Compact single-child folders",
            ep.compact_folders,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_cmp),
            cx,
        ))
        .child(toggle_row(
            "gpui-st-ed-save",
            "Auto-save modified files",
            ep.auto_save,
            tokens,
            GpuiAction::UpdateEditorPrefs(e_save),
            cx,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(tokens.text)
                        .child("Tab size:"),
                )
                .child(setting_btn(
                    "gpui-st-ed-t2",
                    "2 spaces".to_owned(),
                    ep.tab_size == 2,
                    tokens,
                    GpuiAction::UpdateEditorPrefs(e_tab2),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-ed-t4",
                    "4 spaces".to_owned(),
                    ep.tab_size == 4,
                    tokens,
                    GpuiAction::UpdateEditorPrefs(e_tab4),
                    cx,
                )),
        )
        .into_any_element()
}

fn render_browser_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let bp = s.browser_prefs;
    let mut bp_builtin = bp;
    bp_builtin.open_links_in = OpenLinksTarget::Builtin;
    let mut bp_sys = bp;
    bp_sys.open_links_in = OpenLinksTarget::System;
    let mut bp_sep = bp;
    bp_sep.separate_agent_session = !bp_sep.separate_agent_session;

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Navigateur intégré"
            } else {
                "Embedded Browser"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-br-builtin",
                    "Built-in browser tab".to_owned(),
                    bp.open_links_in == OpenLinksTarget::Builtin,
                    tokens,
                    GpuiAction::UpdateBrowserPrefs(bp_builtin),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-br-sys",
                    "System browser".to_owned(),
                    bp.open_links_in == OpenLinksTarget::System,
                    tokens,
                    GpuiAction::UpdateBrowserPrefs(bp_sys),
                    cx,
                )),
        )
        .child(toggle_row(
            "gpui-st-br-sep",
            "Isolate agent browser profile from user profile",
            bp.separate_agent_session,
            tokens,
            GpuiAction::UpdateBrowserPrefs(bp_sep),
            cx,
        ))
        .child(setting_btn(
            "gpui-st-br-vault",
            if french {
                "Ouvrir le coffre de mots de passe"
            } else {
                "Open Saved Passwords Vault"
            }
            .to_owned(),
            false,
            tokens,
            GpuiAction::SettingsPage(SettingsPageAction::OpenPasswordsVault),
            cx,
        ))
        .into_any_element()
}

fn render_passwords_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let Some(v) = &s.vault else {
        return div()
            .text_size(px(12.0))
            .text_color(tokens.text_dim)
            .child("Windows DPAPI Saved Passwords vault unavailable on this platform.")
            .into_any_element();
    };

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Coffre de mots de passe chiffré (Windows DPAPI)"
            } else {
                "Saved Passwords Vault (Windows DPAPI)"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-vault-unlock",
                    if v.unlocked {
                        "Vault Unlocked".to_owned()
                    } else {
                        "Unlock Vault".to_owned()
                    },
                    v.unlocked,
                    tokens,
                    GpuiAction::VaultUnlock,
                    cx,
                ))
                .child(setting_btn(
                    "gpui-vault-import",
                    "Import CSV".to_owned(),
                    false,
                    tokens,
                    GpuiAction::VaultImportCsv,
                    cx,
                )),
        );

    if !v.status.is_empty() {
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(if v.warning {
                    tokens.warn
                } else {
                    tokens.accent
                })
                .child(SharedString::from(v.status.clone())),
        );
    }

    for (idx, entry) in v.entries.iter().enumerate() {
        let fill_id = entry.id.clone();
        let del_id = entry.id.clone();
        col = col.child(
            div()
                .p(px(8.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(entry.origin.clone())),
                        )
                        .child(
                            div()
                                .font_family(mono_font_family())
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .child(SharedString::from(entry.username.clone())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .when(v.can_fill_current_page, |el| {
                            el.child(setting_btn(
                                format!("gpui-vault-fill-{idx}"),
                                "Fill".to_owned(),
                                false,
                                tokens,
                                GpuiAction::VaultFillEntry(fill_id),
                                cx,
                            ))
                        })
                        .child(setting_btn(
                            format!("gpui-vault-del-{idx}"),
                            "Delete".to_owned(),
                            false,
                            tokens,
                            GpuiAction::VaultDeleteEntry(del_id),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_git_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let gp = &s.git_prefs;
    let preview = preview_branch_name(
        gp.branch_prefix_mode,
        &gp.branch_prefix_custom,
        s.git_username.as_deref(),
        "fix-login-flow",
    );

    let mut m_custom = gp.clone();
    m_custom.branch_prefix_mode = BranchPrefixMode::Custom;
    let mut m_user = gp.clone();
    m_user.branch_prefix_mode = BranchPrefixMode::GitUsername;
    let mut m_none = gp.clone();
    m_none.branch_prefix_mode = BranchPrefixMode::None;

    let mut o_chg = gp.clone();
    o_chg.scm_group_order = ScmOrderSetting::ChangesFirst;
    let mut o_stg = gp.clone();
    o_stg.scm_group_order = ScmOrderSetting::StagedFirst;

    let mut v_tree = gp.clone();
    v_tree.scm_view_mode = ScmViewModeSetting::Tree;
    let mut v_list = gp.clone();
    v_list.scm_view_mode = ScmViewModeSetting::List;

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Préfixe des branches Worktree"
            } else {
                "Worktree Branch Prefix"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-git-custom",
                    format!("Custom ({})", gp.branch_prefix_custom),
                    gp.branch_prefix_mode == BranchPrefixMode::Custom,
                    tokens,
                    GpuiAction::UpdateGitPrefs(m_custom),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-git-user",
                    "Git user.name".to_owned(),
                    gp.branch_prefix_mode == BranchPrefixMode::GitUsername,
                    tokens,
                    GpuiAction::UpdateGitPrefs(m_user),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-git-none",
                    "No prefix".to_owned(),
                    gp.branch_prefix_mode == BranchPrefixMode::None,
                    tokens,
                    GpuiAction::UpdateGitPrefs(m_none),
                    cx,
                )),
        )
        .child(
            div()
                .font_family(mono_font_family())
                .text_size(px(11.0))
                .text_color(tokens.accent)
                .child(SharedString::from(format!("Preview: {preview}"))),
        )
        .child(section_title("Source Control View", tokens))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-st-git-ochg",
                    "Changes first".to_owned(),
                    gp.scm_group_order == ScmOrderSetting::ChangesFirst,
                    tokens,
                    GpuiAction::UpdateGitPrefs(o_chg),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-git-ostg",
                    "Staged first".to_owned(),
                    gp.scm_group_order == ScmOrderSetting::StagedFirst,
                    tokens,
                    GpuiAction::UpdateGitPrefs(o_stg),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-git-vtree",
                    "Tree".to_owned(),
                    gp.scm_view_mode == ScmViewModeSetting::Tree,
                    tokens,
                    GpuiAction::UpdateGitPrefs(v_tree),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-git-vlist",
                    "List".to_owned(),
                    gp.scm_view_mode == ScmViewModeSetting::List,
                    tokens,
                    GpuiAction::UpdateGitPrefs(v_list),
                    cx,
                )),
        )
        .into_any_element()
}

fn agent_monogram(name: &str, accent: &str) -> AnyElement {
    let (r, g, b) = crate::settings_extra_model::parse_hex_rgb(accent).unwrap_or((138, 147, 166));
    let bg = gpui::rgb(u32::from_be_bytes([0, r, g, b]));
    let light = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) > 150000;
    div()
        .size(px(22.))
        .flex_none()
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .text_color(gpui::rgb(if light { 0x14161b } else { 0xffffff }))
        .child(
            name.trim()
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string(),
        )
        .into_any_element()
}

fn agent_colors(
    handles: &[gpui::FocusHandle; 8],
    selected: usize,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    use crate::settings_extra_model::CUSTOM_AGENT_SWATCHES;
    let label = if french { "Couleur" } else { "Color" };
    let names = if french {
        [
            "Gris", "Orange", "Jaune", "Vert", "Sarcelle", "Bleu", "Violet", "Rose",
        ]
    } else {
        [
            "Gray", "Orange", "Yellow", "Green", "Teal", "Blue", "Purple", "Pink",
        ]
    };
    let mut group = div()
        .id("custom-agent-colors")
        .role(gpui::Role::RadioGroup)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(4.))
        .child(label);
    for (index, hex) in CUSTOM_AGENT_SWATCHES.iter().enumerate() {
        let color = u32::from_str_radix(&hex[1..], 16).expect("constant swatch");
        group = group.child(
            div()
                .id(("agent-color", index))
                .debug_selector(move || format!("agent-color-{index}"))
                .track_focus(&handles[index])
                .tab_stop(index == selected)
                .role(gpui::Role::RadioButton)
                .aria_label(names[index])
                .aria_toggled((index == selected).into())
                .size(px(18.))
                .rounded_full()
                .bg(gpui::rgb(color))
                .border_2()
                .border_color(if index == selected {
                    tokens.text_strong
                } else {
                    gpui::rgba(0)
                })
                .focus_visible(|s| s.border_color(tokens.accent))
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, w, cx| {
                    root.focus_input = FocusInputTarget::None;
                    root.agent_swatches[index].focus(w, cx);
                    root.dispatch_action(GpuiAction::SetCustomAgentAccent(index), cx);
                }))
                .on_key_down(cx.listener(move |root, event: &gpui::KeyDownEvent, w, cx| {
                    let len = CUSTOM_AGENT_SWATCHES.len();
                    let next = match event.keystroke.key.as_str() {
                        "right" | "down" => (index + 1) % len,
                        "left" | "up" => (index + len - 1) % len,
                        "space" => index,
                        _ => return,
                    };
                    cx.stop_propagation();
                    root.focus_input = FocusInputTarget::None;
                    root.agent_swatches[next].focus(w, cx);
                    root.dispatch_action(GpuiAction::SetCustomAgentAccent(next), cx);
                })),
        );
    }
    group.into_any_element()
}

fn render_agents_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    swatches: &[gpui::FocusHandle; 8],
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(8.0)).child(section_title(
        if french {
            "Agents CLI intégrés"
        } else {
            "Built-in Coding Agents"
        },
        tokens,
    ));
    col = col.child(toggle_row_owned(
        "gpui-st-agent-folder-trust".to_owned(),
        crate::agents_panel::agent_folder_trust_label(french).to_owned(),
        s.agent_folder_trust,
        tokens,
        GpuiAction::ToggleAgentFolderTrust,
        cx,
    ));

    for detail in &s.builtin_agent_details {
        let aid = detail.id.clone();
        let exp_id = detail.id.clone();
        let is_expanded = s.expanded_agent_id.as_deref() == Some(detail.id.as_str());
        let label = format!(
            "{} ({})",
            detail.name,
            if detail.installed {
                if french { "installé" } else { "installed" }
            } else if french {
                "non détecté"
            } else {
                "not detected"
            }
        );
        let mut card = div()
            .p(px(8.0))
            .rounded(px(tokens.radius_px))
            .bg(tokens.surface)
            .border_1()
            .border_color(if detail.env_error.is_some() {
                tokens.danger
            } else {
                tokens.border
            })
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(6.0))
                    .child(div().flex_1().child(toggle_row_owned(
                        format!("gpui-st-agent-{}", detail.id),
                        label,
                        detail.enabled,
                        tokens,
                        GpuiAction::ToggleBuiltinAgentEnabled(aid),
                        cx,
                    )))
                    .child(setting_btn(
                        format!("gpui-st-agent-exp-{}", detail.id),
                        if is_expanded {
                            if french {
                                "Replier".into()
                            } else {
                                "Fold".into()
                            }
                        } else if french {
                            "Configurer".into()
                        } else {
                            "Configure".into()
                        },
                        is_expanded,
                        tokens,
                        GpuiAction::ToggleExpandBuiltinAgent(exp_id),
                        cx,
                    )),
            );

        if let Some(err) = &detail.env_error {
            let callout = crate::agents_panel::format_invalid_env_callout(err, french);
            card = card.child(
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(tokens.radius_px - 1.0))
                    .bg(super::theme::rgba_with_alpha(tokens.danger, 0.14))
                    .text_size(px(11.0))
                    .text_color(tokens.danger)
                    .child(SharedString::from(callout)),
            );
        }

        if is_expanded {
            let ref_id = detail.id.clone();
            let rst_id = detail.id.clone();
            let mut details_col = div()
                .pt(px(4.0))
                .border_t_1()
                .border_color(tokens.border)
                .flex()
                .flex_col()
                .gap(px(6.0));
            if let Some(caption) = &detail.listed_models_caption {
                details_col = details_col.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(caption.clone())),
                );
            }
            details_col = details_col
                .child(render_focus_input_box(
                    "gpui-st-ba-cmd",
                    FocusInputTarget::BuiltinAgentCommand,
                    focus_input == FocusInputTarget::BuiltinAgentCommand,
                    &detail.command,
                    if french {
                        "Commande personnalisée…"
                    } else {
                        "Custom command override…"
                    },
                    tokens,
                    cx,
                ))
                .child(render_focus_input_box(
                    "gpui-st-ba-args",
                    FocusInputTarget::BuiltinAgentArgs,
                    focus_input == FocusInputTarget::BuiltinAgentArgs,
                    &detail.args,
                    "Arguments…",
                    tokens,
                    cx,
                ))
                .child(render_focus_input_box(
                    "gpui-st-ba-env",
                    FocusInputTarget::BuiltinAgentEnv,
                    focus_input == FocusInputTarget::BuiltinAgentEnv,
                    &detail.env,
                    if french {
                        "Variables, un NOM=valeur par ligne…"
                    } else {
                        "Variables, one NAME=value per line…"
                    },
                    tokens,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .when(detail.can_refresh_models, |el| {
                            el.child(setting_btn(
                                format!("gpui-st-ba-refresh-{}", detail.id),
                                if french {
                                    "Actualiser les modèles".into()
                                } else {
                                    "Refresh models".into()
                                },
                                false,
                                tokens,
                                GpuiAction::RefreshBuiltinAgentModels(ref_id),
                                cx,
                            ))
                        })
                        .child(setting_btn(
                            format!("gpui-st-ba-reset-{}", detail.id),
                            if french {
                                "Réinitialiser".into()
                            } else {
                                "Reset".into()
                            },
                            false,
                            tokens,
                            GpuiAction::ResetBuiltinAgentPrefs(rst_id),
                            cx,
                        )),
                );
            card = card.child(details_col);
        }

        col = col.child(card);
    }

    col = col
        .child(section_title(
            if french {
                "Agents CLI personnalisés"
            } else {
                "Custom CLI Agents"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .items_center()
                .child(agent_monogram(
                    &s.new_custom_agent_name,
                    crate::settings_extra_model::CUSTOM_AGENT_SWATCHES[s.new_custom_agent_accent],
                ))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-st-ca-name",
                    FocusInputTarget::CustomAgentName,
                    focus_input == FocusInputTarget::CustomAgentName,
                    &s.new_custom_agent_name,
                    if french {
                        "Nom, p. ex. Crush"
                    } else {
                        "Name, e.g. Crush"
                    },
                    tokens,
                    cx,
                )))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-st-ca-cmd",
                    FocusInputTarget::CustomAgentCmd,
                    focus_input == FocusInputTarget::CustomAgentCmd,
                    &s.new_custom_agent_cmd,
                    if french {
                        "Commande, p. ex. crush --yolo"
                    } else {
                        "Command, e.g. crush --yolo"
                    },
                    tokens,
                    cx,
                ))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(agent_colors(
                    swatches,
                    s.new_custom_agent_accent,
                    french,
                    tokens,
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-ca-add",
                    if french {
                        "Ajouter un agent"
                    } else {
                        "Add agent"
                    }
                    .to_owned(),
                    true,
                    tokens,
                    GpuiAction::SubmitNewCustomAgent,
                    cx,
                )),
        );

    if !s.custom_agent_error.is_empty() {
        col = col.child(
            div()
                .text_color(tokens.warn)
                .child(crate::i18n::tr(&s.custom_agent_error).to_owned()),
        );
    }

    for ca in &s.custom_agents {
        let toggle_id = ca.id.clone();
        let next_en = !ca.enabled;
        let del_id = ca.id.clone();
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.5))
                        .text_color(tokens.text_strong)
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(agent_monogram(&ca.name, &ca.accent))
                        .child(SharedString::from(format!("{} — {}", ca.name, ca.command))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(setting_btn(
                            format!("gpui-st-ca-tog-{}", ca.id),
                            if ca.enabled { "Enabled" } else { "Disabled" }.to_owned(),
                            ca.enabled,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::ToggleCustomAgent {
                                id: toggle_id,
                                enabled: next_en,
                            }),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-st-ca-del-{}", ca.id),
                            "Delete".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::DeleteCustomAgent(
                                del_id,
                            )),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_accounts_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let lbl_draft = s.new_account_label.trim().to_owned();
    let add_claude_lbl = if lbl_draft.is_empty() {
        "Work".to_owned()
    } else {
        lbl_draft.clone()
    };
    let add_codex_lbl = if lbl_draft.is_empty() {
        "Work".to_owned()
    } else {
        lbl_draft
    };

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Comptes gérés Claude & Codex"
            } else {
                "Managed Claude & Codex Accounts"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-st-acct-lbl",
                    FocusInputTarget::AccountLabel,
                    focus_input == FocusInputTarget::AccountLabel,
                    &s.new_account_label,
                    "Profile label (e.g. Work, Personal)…",
                    tokens,
                    cx,
                )))
                .child(setting_btn(
                    "gpui-st-acct-add-claude",
                    "+ Claude".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::AddManagedAccount {
                        provider: "claude".into(),
                        label: add_claude_lbl,
                    }),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-st-acct-add-codex",
                    "+ Codex".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::AddManagedAccount {
                        provider: "codex".into(),
                        label: add_codex_lbl,
                    }),
                    cx,
                )),
        );

    for (prov, roster, sys_email) in [
        (
            "claude",
            &s.claude_accounts,
            s.claude_system_email.as_deref(),
        ),
        ("codex", &s.codex_accounts, s.codex_system_email.as_deref()),
    ] {
        if !crate::settings_extra_model::system_account_covered(sys_email, roster) {
            let sys_sel = roster.selected_id.is_none();
            let sys_title = match sys_email.map(str::trim).filter(|e| !e.is_empty()) {
                Some(addr) if french => format!("[{prov}] Par défaut système ({addr})"),
                Some(addr) => format!("[{prov}] System default ({addr})"),
                None if french => format!("[{prov}] Par défaut système"),
                None => format!("[{prov}] System default"),
            };
            col = col.child(
                div()
                    .p(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(if sys_sel {
                        tokens.accent
                    } else {
                        tokens.border
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(sys_title)),
                    )
                    .child(setting_btn(
                        format!("gpui-acct-sys-{prov}"),
                        if french {
                            "Par défaut".into()
                        } else {
                            "Use".into()
                        },
                        sys_sel,
                        tokens,
                        GpuiAction::SettingsExtra(SettingsExtraAction::SelectManagedAccount {
                            provider: prov.to_owned(),
                            account_id: None,
                        }),
                        cx,
                    )),
            );
        }
        for acct in &roster.accounts {
            let is_sel = roster.selected_id.as_deref() == Some(acct.id.as_str());
            let p1 = prov.to_owned();
            let p2 = prov.to_owned();
            let p3 = prov.to_owned();
            let id1 = acct.id.clone();
            let id2 = acct.id.clone();
            let id3 = acct.id.clone();
            let row_label = if acct.email.trim().is_empty() {
                format!("[{prov}] {}", acct.label)
            } else {
                format!("[{prov}] {} ({})", acct.label, acct.email.trim())
            };
            col = col.child(
                div()
                    .p(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.surface)
                    .border_1()
                    .border_color(if is_sel { tokens.accent } else { tokens.border })
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(tokens.text_strong)
                            .child(SharedString::from(row_label)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .child(setting_btn(
                                format!("gpui-acct-sel-{prov}-{}", acct.id),
                                if is_sel {
                                    if french {
                                        "Actif".into()
                                    } else {
                                        "Selected".into()
                                    }
                                } else if french {
                                    "Utiliser".into()
                                } else {
                                    "Use".into()
                                },
                                is_sel,
                                tokens,
                                GpuiAction::SettingsExtra(
                                    SettingsExtraAction::SelectManagedAccount {
                                        provider: p1,
                                        account_id: if is_sel { None } else { Some(id1) },
                                    },
                                ),
                                cx,
                            ))
                            .child(setting_btn(
                                format!("gpui-acct-login-{prov}-{}", acct.id),
                                "Sign in".to_owned(),
                                false,
                                tokens,
                                GpuiAction::SettingsExtra(
                                    SettingsExtraAction::LaunchManagedAccountSignIn {
                                        provider: p2,
                                        account_id: id2,
                                    },
                                ),
                                cx,
                            ))
                            .child(setting_btn(
                                format!("gpui-acct-rm-{prov}-{}", acct.id),
                                "Remove".to_owned(),
                                false,
                                tokens,
                                GpuiAction::SettingsExtra(
                                    SettingsExtraAction::RemoveManagedAccount {
                                        provider: p3,
                                        account_id: id3,
                                    },
                                ),
                                cx,
                            )),
                    ),
            );
        }
    }

    col = col.child(section_title("Encrypted Provider API Keys (DPAPI)", tokens));
    let mut key_chips = div().flex().flex_wrap().gap(px(4.0));
    for (idx, (k, _, stored)) in s.provider_secrets.iter().enumerate() {
        key_chips = key_chips.child(setting_btn(
            format!("gpui-sec-key-{idx}"),
            format!("{k}{}", if *stored { " ✓" } else { "" }),
            s.secret_key_idx == idx,
            tokens,
            GpuiAction::SelectSecretKeyIndex(idx),
            cx,
        ));
    }
    col = col.child(key_chips).child(
        div()
            .flex()
            .gap(px(6.0))
            .child(div().flex_1().child(render_focus_input_box(
                "gpui-st-sec-val",
                FocusInputTarget::ProviderSecretValue,
                focus_input == FocusInputTarget::ProviderSecretValue,
                &s.secret_value_draft,
                "Paste API key or session token…",
                tokens,
                cx,
            )))
            .child(setting_btn(
                "gpui-st-sec-save",
                "Save DPAPI Secret".to_owned(),
                true,
                tokens,
                GpuiAction::SaveSecretFromDraft,
                cx,
            )),
    );

    col.into_any_element()
}

fn render_skills_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let git_url = s.skills_git_url.trim().to_owned();
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_title(
                    if french {
                        "Skills d'agents (SKILL.md)"
                    } else {
                        "Agent Skills (SKILL.md)"
                    },
                    tokens,
                ))
                .child(setting_btn(
                    "gpui-sk-refresh",
                    "Refresh".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::RefreshSkills),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-sk-git-url",
                    FocusInputTarget::SkillsGitUrl,
                    focus_input == FocusInputTarget::SkillsGitUrl,
                    &s.skills_git_url,
                    "https://github.com/org/skill-repo.git",
                    tokens,
                    cx,
                )))
                .child(setting_btn(
                    "gpui-sk-clone",
                    "Clone Skill".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::CloneSkillFromGit {
                        repo_url: git_url,
                        project_scope: false,
                    }),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-sk-pick",
                    "Install Folder".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::PickSkillFolder {
                        project_scope: false,
                    }),
                    cx,
                )),
        );

    for (idx, sk) in s.skills.iter().enumerate() {
        let md_path = sk.skill_md_path.clone();
        let dir_path = sk.dir_path.clone();
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(format!(
                                    "{} ({})",
                                    sk.name, sk.agent_label
                                ))),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .truncate()
                                .child(SharedString::from(sk.description.clone())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(setting_btn(
                            format!("gpui-sk-open-{idx}"),
                            "Open".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::OpenSkillMarkdown(
                                md_path,
                            )),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-sk-rm-{idx}"),
                            "Remove".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::RemoveSkillDir(
                                dir_path,
                            )),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_mcp_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(8.0)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_title(
                if french {
                    "Serveurs MCP installés"
                } else {
                    "Installed MCP Servers"
                },
                tokens,
            ))
            .child(setting_btn(
                "gpui-mcp-refresh",
                "Refresh".to_owned(),
                false,
                tokens,
                GpuiAction::SettingsExtra(SettingsExtraAction::RefreshMcp),
                cx,
            )),
    );

    if !s.mcp_test_status.is_empty() {
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(tokens.text_dim)
                .child(SharedString::from(s.mcp_test_status.clone())),
        );
    }

    let mut shown_bulk_for = std::collections::HashSet::new();
    for (idx, srv) in s.installed_mcp.iter().enumerate() {
        let rm_cfg = srv.config_path.clone();
        let rm_agent = srv.agent_id.clone();
        let rm_name = srv.name.clone();
        let tst_name = srv.name.clone();
        let tst_tr = srv.transport;
        let tst_ep = srv.endpoint_summary.clone();
        let is_remote = srv.transport == crate::settings_extra_model::McpTransport::Http;
        let remote_count = if is_remote {
            s.installed_mcp
                .iter()
                .filter(|m| {
                    m.transport == crate::settings_extra_model::McpTransport::Http
                        && m.name == srv.name
                })
                .count()
        } else {
            0
        };
        let show_bulk_here = remote_count > 1 && shown_bulk_for.insert(srv.name.clone());
        let signs_in_from_agent = s.mcp_probe_outcomes.get(&srv.name)
            == Some(&crate::settings_extra_model::McpProbeOutcome::SignsInFromAgent);
        let card_key = format!("{}:{}", srv.agent_id, srv.name);
        let bulk_key = format!("all:{}", srv.name);
        let inline_msg = s
            .mcp_card_messages
            .get(&card_key)
            .cloned()
            .map(|m| (card_key.clone(), m))
            .or_else(|| {
                show_bulk_here
                    .then(|| {
                        s.mcp_card_messages
                            .get(&bulk_key)
                            .cloned()
                            .map(|m| (bulk_key.clone(), m))
                    })
                    .flatten()
            });

        let sign_name = srv.name.clone();
        let sign_agent = srv.agent_id.clone();
        let sign_label = srv.agent_label.clone();
        let bulk_name = srv.name.clone();

        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(tokens.text_strong)
                                                .child(SharedString::from(format!(
                                                    "{} · {}",
                                                    srv.name, srv.agent_label
                                                ))),
                                        )
                                        .when(signs_in_from_agent, |el| {
                                            el.child(
                                                div()
                                                    .px(px(6.0))
                                                    .py(px(1.5))
                                                    .rounded(px(4.0))
                                                    .bg(super::theme::rgba_with_alpha(
                                                        tokens.warn,
                                                        0.18,
                                                    ))
                                                    .text_size(px(10.0))
                                                    .text_color(tokens.warn)
                                                    .child(
                                                        crate::settings_extra_model::McpProbeOutcome::signs_in_badge(
                                                            french,
                                                        ),
                                                    ),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .font_family(mono_font_family())
                                        .text_size(px(10.0))
                                        .text_color(tokens.text_dim)
                                        .truncate()
                                        .child(SharedString::from(srv.endpoint_summary.clone())),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(4.0))
                                .when(show_bulk_here, |el| {
                                    el.child(setting_btn(
                                        format!("gpui-mcp-signin-all-{idx}"),
                                        if french {
                                            "Tout connecter".into()
                                        } else {
                                            "Sign in all".into()
                                        },
                                        true,
                                        tokens,
                                        GpuiAction::SettingsExtra(
                                            SettingsExtraAction::SignInAllMcpServer {
                                                name: bulk_name,
                                            },
                                        ),
                                        cx,
                                    ))
                                })
                                .when(is_remote, |el| {
                                    el.child(setting_btn(
                                        format!("gpui-mcp-signin-{idx}"),
                                        if french {
                                            "Se connecter".into()
                                        } else {
                                            "Sign in".into()
                                        },
                                        false,
                                        tokens,
                                        GpuiAction::SettingsExtra(
                                            SettingsExtraAction::SignInMcpAgent {
                                                name: sign_name,
                                                agent_id: sign_agent,
                                                agent_label: sign_label,
                                            },
                                        ),
                                        cx,
                                    ))
                                })
                                .child(setting_btn(
                                    format!("gpui-mcp-test-{idx}"),
                                    if french { "Tester" } else { "Test" }.to_owned(),
                                    false,
                                    tokens,
                                    GpuiAction::SettingsExtra(SettingsExtraAction::TestMcpServer {
                                        name: tst_name,
                                        transport: tst_tr,
                                        endpoint: tst_ep,
                                    }),
                                    cx,
                                ))
                                .when(srv.agent_id != "codex", |el| {
                                    el.child(setting_btn(
                                        format!("gpui-mcp-rm-{idx}"),
                                        if french { "Retirer" } else { "Remove" }.to_owned(),
                                        false,
                                        tokens,
                                        GpuiAction::SettingsExtra(
                                            SettingsExtraAction::RemoveInstalledMcp {
                                                config_path: rm_cfg,
                                                agent_id: rm_agent,
                                                name: rm_name,
                                            },
                                        ),
                                        cx,
                                    ))
                                }),
                        ),
                )
                .when_some(inline_msg, |el, (mkey, mtxt)| {
                    el.child(
                        div()
                            .px(px(8.0))
                            .py(px(5.0))
                            .rounded(px(tokens.radius_px - 1.0))
                            .bg(tokens.surface_2)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.0))
                                    .text_color(tokens.text_strong)
                                    .child(SharedString::from(mtxt)),
                            )
                            .child(setting_btn(
                                format!("gpui-mcp-dismiss-{idx}"),
                                "×".to_owned(),
                                false,
                                tokens,
                                GpuiAction::DismissMcpCardMessage(mkey),
                                cx,
                            )),
                    )
                }),
        );
    }

    col = col.child(section_title("Curated MCP Catalog (30 servers)", tokens));
    for entry in MCP_CATALOG.iter().take(15) {
        let cid = entry.id;
        col = col.child(
            div()
                .p(px(6.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(entry.name),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .truncate()
                                .child(if french { entry.desc_fr } else { entry.desc_en }),
                        ),
                )
                .child(setting_btn(
                    format!("gpui-mcp-cat-{cid}"),
                    "Install".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::InstallCatalogMcp {
                        catalog_id: cid,
                        target_agent_idx: 0,
                        env_values: Vec::new(),
                    }),
                    cx,
                )),
        );
    }

    col.into_any_element()
}

fn render_connections_page(
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Connecteurs & Intégrations Tessel"
            } else {
                "Tessel Cloud Connectors & Productivity"
            },
            tokens,
        ))
        .child(setting_btn(
            "gpui-conn-account",
            if french {
                "Gérer le compte Tessel"
            } else {
                "Manage Tessel Account"
            }
            .to_owned(),
            false,
            tokens,
            GpuiAction::SettingsExtra(SettingsExtraAction::OpenTesselAccountSettings),
            cx,
        ));

    for spec in TESSEL_CONNECTORS {
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(spec.name),
                        )
                        .child(div().text_size(px(10.5)).text_color(tokens.text_dim).child(
                            if french {
                                spec.summary_fr
                            } else {
                                spec.summary_en
                            },
                        )),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(if spec.available {
                            tokens.ok_green
                        } else {
                            tokens.text_dim
                        })
                        .child(if spec.available { "Available" } else { "MCP" }),
                ),
        );
    }

    col.into_any_element()
}

fn render_context_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut fam_row = div().flex().flex_wrap().gap(px(5.0));
    for fam in ContextModelFamily::ALL {
        fam_row = fam_row.child(setting_btn(
            format!("gpui-ctx-fam-{fam:?}"),
            fam.label().to_owned(),
            s.context_prefs.model_family == fam,
            tokens,
            GpuiAction::UpdateContextPrefs(crate::settings_extra_model::ContextPreferences {
                model_family: fam,
            }),
            cx,
        ));
    }

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_title(
                    if french {
                        "Budget de contexte initial"
                    } else {
                        "Upfront Context Token Footprint"
                    },
                    tokens,
                ))
                .child(setting_btn(
                    "gpui-ctx-refresh",
                    "Rescan".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::RefreshContext),
                    cx,
                )),
        )
        .child(fam_row)
        .child(
            div()
                .p(px(8.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.accent)
                .font_family(mono_font_family())
                .text_size(px(12.0))
                .text_color(tokens.text_strong)
                .child(SharedString::from(format!(
                    "Estimated upfront context: ~{} tokens (Rules: {}, Skills: {}, MCP: {}, Team rule: {})",
                    s.context_scan.total_tokens,
                    s.context_scan.rules_files.len(),
                    s.context_scan.skills_items.len(),
                    s.context_scan.mcp_items.len(),
                    s.context_scan.team_rule_tokens
                ))),
        );

    for item in s
        .context_scan
        .rules_files
        .iter()
        .chain(s.context_scan.skills_items.iter())
        .chain(s.context_scan.mcp_items.iter())
        .take(24)
    {
        col = col.child(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(tokens.surface)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(tokens.text)
                        .truncate()
                        .child(SharedString::from(item.label.clone())),
                )
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(10.5))
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(format!(
                            "~{} tok",
                            item.estimated_tokens
                        ))),
                ),
        );
    }

    col.into_any_element()
}

fn render_orchestration_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let op = &s.orchestration_prefs;
    let mut o_wake = op.clone();
    o_wake.team_wake_ups = !o_wake.team_wake_ups;
    let mut o_unconf = op.clone();
    o_unconf.team_wake_unconfirmed = !o_unconf.team_wake_unconfirmed;
    let mut o_conf = op.clone();
    o_conf.confirm_workers = !o_conf.confirm_workers;
    let mut o_term = op.clone();
    o_term.worker_mode = WorkerMode::Terminal;
    let mut o_chat = op.clone();
    o_chat.worker_mode = WorkerMode::Chat;

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Orchestration d'équipes & Workers"
            } else {
                "Team Orchestration & Worker Subagents"
            },
            tokens,
        ))
        .child(toggle_row(
            "gpui-orch-wake",
            "Idle team & relay wake-up reminders",
            op.team_wake_ups,
            tokens,
            GpuiAction::UpdateOrchestrationPrefs(o_wake),
            cx,
        ))
        .child(toggle_row(
            "gpui-orch-unconf",
            "Send PTY wake-up without idle confirmation",
            op.team_wake_unconfirmed,
            tokens,
            GpuiAction::UpdateOrchestrationPrefs(o_unconf),
            cx,
        ))
        .child(toggle_row(
            "gpui-orch-confw",
            "Require user confirmation before spawning workers",
            op.confirm_workers,
            tokens,
            GpuiAction::UpdateOrchestrationPrefs(o_conf),
            cx,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-orch-mterm",
                    "Workers in Terminal".to_owned(),
                    op.worker_mode == WorkerMode::Terminal,
                    tokens,
                    GpuiAction::UpdateOrchestrationPrefs(o_term),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-orch-mchat",
                    "Workers in Native Chat".to_owned(),
                    op.worker_mode == WorkerMode::Chat,
                    tokens,
                    GpuiAction::UpdateOrchestrationPrefs(o_chat),
                    cx,
                )),
        )
        .child(section_title("Team MCP Tools Reference", tokens));

    for (tool, desc_en, desc_fr) in TEAM_TOOLS_REFERENCE {
        col = col.child(
            div()
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(tokens.radius_px - 1.0))
                .bg(tokens.surface)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.accent)
                        .child(tool),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .text_color(tokens.text_dim)
                        .truncate()
                        .child(if french { desc_fr } else { desc_en }),
                ),
        );
    }

    col.into_any_element()
}

fn render_alerts_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let np = &s.notification_prefs;
    let mut n_en = np.clone();
    n_en.notifications_enabled = !n_en.notifications_enabled;
    let mut n_desk = np.clone();
    n_desk.desktop_notifications = !n_desk.desktop_notifications;
    let mut n_inapp = np.clone();
    n_inapp.in_app_alerts = !n_inapp.in_app_alerts;
    let mut n_bell = np.clone();
    n_bell.notify_terminal_bell = !n_bell.notify_terminal_bell;
    let mut n_sup = np.clone();
    n_sup.notify_suppress_when_focused = !n_sup.notify_suppress_when_focused;

    let mut s_none = np.clone();
    s_none.alert_sound = AlertSound::None;
    let mut s_chime = np.clone();
    s_chime.alert_sound = AlertSound::Chime;
    let mut s_ping = np.clone();
    s_ping.alert_sound = AlertSound::Ping;

    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Notifications & Alertes"
            } else {
                "Notifications & Alerts"
            },
            tokens,
        ))
        .child(toggle_row(
            "gpui-al-en",
            "Enable notifications",
            np.notifications_enabled,
            tokens,
            GpuiAction::UpdateNotificationPrefs(n_en),
            cx,
        ))
        .child(toggle_row(
            "gpui-al-desk",
            "Desktop OS notifications",
            np.desktop_notifications,
            tokens,
            GpuiAction::UpdateNotificationPrefs(n_desk),
            cx,
        ))
        .child(toggle_row(
            "gpui-al-inapp",
            "In-app toolbar notification center",
            np.in_app_alerts,
            tokens,
            GpuiAction::UpdateNotificationPrefs(n_inapp),
            cx,
        ))
        .child(toggle_row(
            "gpui-al-bell",
            "Alert on terminal BEL (\\x07)",
            np.notify_terminal_bell,
            tokens,
            GpuiAction::UpdateNotificationPrefs(n_bell),
            cx,
        ))
        .child(toggle_row(
            "gpui-al-sup",
            "Suppress when target pane is focused",
            np.notify_suppress_when_focused,
            tokens,
            GpuiAction::UpdateNotificationPrefs(n_sup),
            cx,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-al-snone",
                    "Silent".to_owned(),
                    np.alert_sound == AlertSound::None,
                    tokens,
                    GpuiAction::UpdateNotificationPrefs(s_none),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-al-schime",
                    "Chime".to_owned(),
                    np.alert_sound == AlertSound::Chime,
                    tokens,
                    GpuiAction::UpdateNotificationPrefs(s_chime),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-al-sping",
                    "Ping".to_owned(),
                    np.alert_sound == AlertSound::Ping,
                    tokens,
                    GpuiAction::UpdateNotificationPrefs(s_ping),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-al-test",
                    "Send test alert".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::SendTestNotification),
                    cx,
                )),
        )
        .into_any_element()
}

fn render_voice_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut langs = div().flex().flex_wrap().gap(px(5.0));
    for (idx, (code, label)) in supported_voice_languages(french).into_iter().enumerate() {
        let active = s.voice_prefs.voice_tip == code;
        langs = langs.child(setting_btn(
            format!("gpui-voice-lang-{idx}"),
            label.to_owned(),
            active,
            tokens,
            GpuiAction::UpdateVoicePrefs(crate::settings_extra_model::VoicePreferences {
                voice_tip: code.to_owned(),
                voice_tip_chosen: true,
            }),
            cx,
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Dictée vocale Windows (Win+H)"
            } else {
                "Windows Voice Typing (Win+H)"
            },
            tokens,
        ))
        .child(langs)
        .child(setting_btn(
            "gpui-voice-trigger",
            if french {
                "Lancer la dictée vocale (Win+H)"
            } else {
                "Trigger Windows Voice Typing (Win+H)"
            }
            .to_owned(),
            true,
            tokens,
            GpuiAction::SettingsExtra(SettingsExtraAction::TriggerWindowsVoiceTyping),
            cx,
        ))
        .into_any_element()
}

fn render_devtools_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(8.0)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_title(
                if french {
                    "Outils de développement"
                } else {
                    "Developer Tools"
                },
                tokens,
            ))
            .child(setting_btn(
                "gpui-dt-refresh",
                "Probe PATH".to_owned(),
                false,
                tokens,
                GpuiAction::SettingsExtra(SettingsExtraAction::RefreshDevtools),
                cx,
            )),
    );

    for spec in DEVTOOLS_CATALOG {
        let status = s.devtools.iter().find(|d| d.id == spec.id);
        let installed = status.is_some_and(|d| d.installed);
        let ver = status
            .map(|d| d.version.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "not detected".into());
        let cmd = winget_install_command(spec.winget_id);
        let title = format!("Install {}", spec.name);

        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(format!("{} — {ver}", spec.name))),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .child(if french { spec.desc_fr } else { spec.desc_en }),
                        ),
                )
                .child(setting_btn(
                    format!("gpui-dt-act-{}", spec.id),
                    if installed {
                        "Installed ✓".to_owned()
                    } else {
                        "Install via winget".to_owned()
                    },
                    installed,
                    tokens,
                    GpuiAction::SettingsExtra(SettingsExtraAction::RunTerminalSetupCommand {
                        title,
                        command: cmd,
                    }),
                    cx,
                )),
        );
    }

    col.into_any_element()
}

fn render_automations_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Automatisations planifiées (≥ 15 min)"
            } else {
                "Scheduled Automations (≥ 15 min)"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-auto-name",
                    FocusInputTarget::AutomationName,
                    focus_input == FocusInputTarget::AutomationName,
                    &s.new_auto_name,
                    "Automation name…",
                    tokens,
                    cx,
                )))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-auto-prompt",
                    FocusInputTarget::AutomationPrompt,
                    focus_input == FocusInputTarget::AutomationPrompt,
                    &s.new_auto_prompt,
                    "Agent prompt…",
                    tokens,
                    cx,
                )))
                .child(setting_btn(
                    "gpui-auto-add",
                    "Create".to_owned(),
                    true,
                    tokens,
                    GpuiAction::SubmitNewAutomation,
                    cx,
                )),
        );

    for auto in &s.automations.automations {
        let id_en = auto.id.clone();
        let id_run = auto.id.clone();
        let id_del = auto.id.clone();
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(auto.name.clone())),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(tokens.text_dim)
                                .truncate()
                                .child(SharedString::from(auto.prompt.clone())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(setting_btn(
                            format!("gpui-auto-en-{}", auto.id),
                            if auto.enabled {
                                "Enabled"
                            } else {
                                "Approve & Enable"
                            }
                            .to_owned(),
                            auto.enabled,
                            tokens,
                            GpuiAction::SettingsExtra(
                                SettingsExtraAction::ConfirmAndEnableAutomation(id_en),
                            ),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-auto-run-{}", auto.id),
                            "Run now".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::RunAutomationNow(
                                id_run,
                            )),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-auto-del-{}", auto.id),
                            "Delete".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsExtra(SettingsExtraAction::DeleteAutomation(
                                id_del,
                            )),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_quick_commands_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Commandes rapides"
            } else {
                "Quick Commands"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-qc-name",
                    FocusInputTarget::QuickCommandName,
                    focus_input == FocusInputTarget::QuickCommandName,
                    &s.new_quick_cmd_name,
                    "Label (e.g. Run tests)…",
                    tokens,
                    cx,
                )))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-qc-text",
                    FocusInputTarget::QuickCommandText,
                    focus_input == FocusInputTarget::QuickCommandText,
                    &s.new_quick_cmd_text,
                    "Command (e.g. cargo test)…",
                    tokens,
                    cx,
                )))
                .child(setting_btn(
                    "gpui-qc-add",
                    "Add".to_owned(),
                    true,
                    tokens,
                    GpuiAction::SubmitNewQuickCommand,
                    cx,
                )),
        );

    for qc in &s.quick_commands {
        let qid = qc.id;
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.5))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(format!("{} → {}", qc.name, qc.text))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(setting_btn(
                            format!("gpui-qc-run-{qid}"),
                            "Run".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsPage(SettingsPageAction::RunQuickCommand(qid)),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-qc-rm-{qid}"),
                            "Delete".to_owned(),
                            false,
                            tokens,
                            GpuiAction::SettingsPage(SettingsPageAction::RemoveQuickCommand(qid)),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_ssh_page(
    s: &GpuiSettingsSnapshot,
    focus_input: FocusInputTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_title(
            if french {
                "Hôtes SSH distants"
            } else {
                "Remote SSH Hosts"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-ssh-alias",
                    FocusInputTarget::SshAlias,
                    focus_input == FocusInputTarget::SshAlias,
                    &s.new_ssh_alias,
                    "user@host or SSH config alias…",
                    tokens,
                    cx,
                )))
                .child(div().flex_1().child(render_focus_input_box(
                    "gpui-ssh-folder",
                    FocusInputTarget::SshFolder,
                    focus_input == FocusInputTarget::SshFolder,
                    &s.new_ssh_folder,
                    "Remote folder (~)…",
                    tokens,
                    cx,
                )))
                .child(setting_btn(
                    "gpui-ssh-add",
                    "Add Host".to_owned(),
                    true,
                    tokens,
                    GpuiAction::AddSshHostFromDraft,
                    cx,
                )),
        );

    for (idx, host) in s.ssh_hosts.iter().enumerate() {
        col = col.child(
            div()
                .p(px(7.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(mono_font_family())
                        .text_size(px(11.5))
                        .text_color(tokens.text_strong)
                        .child(SharedString::from(format!(
                            "{} : {}",
                            host.alias, host.folder
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(setting_btn(
                            format!("gpui-ssh-conn-{idx}"),
                            "Open Terminal".to_owned(),
                            false,
                            tokens,
                            GpuiAction::LaunchSshHost(idx),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-ssh-tmux-{idx}"),
                            "tmux".to_owned(),
                            host.persistent_sessions_enabled(),
                            tokens,
                            GpuiAction::ToggleSshHostPersistentSessions(idx),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-ssh-daemon-{idx}"),
                            if french {
                                "Démon".to_owned()
                            } else {
                                "Daemon".to_owned()
                            },
                            host.host_daemon,
                            tokens,
                            GpuiAction::ToggleSshHostDaemon(idx),
                            cx,
                        ))
                        .child(setting_btn(
                            format!("gpui-ssh-rm-{idx}"),
                            "Remove".to_owned(),
                            false,
                            tokens,
                            GpuiAction::RemoveSshHost(idx),
                            cx,
                        )),
                ),
        );
    }

    col.into_any_element()
}

fn render_tessel_account_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(10.0)).child(section_title(
        if french {
            "Compte Tessel & Synchronisation Cloud E2EE"
        } else {
            "Tessel Account & E2EE Cloud Sync (scrypt + AES-256-GCM)"
        },
        tokens,
    ));

    if let Some(acc) = &s.account {
        if acc.signed_in {
            col = col
                .child(
                    div()
                        .p(px(10.0))
                        .rounded(px(tokens.radius_px))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.accent)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(tokens.text_strong)
                                .child(SharedString::from(format!(
                                    "{} ({})",
                                    acc.user_name, acc.user_email
                                ))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(setting_btn(
                                    "gpui-acc-refresh",
                                    if french {
                                        "Actualiser le compte".into()
                                    } else {
                                        "Refresh account".into()
                                    },
                                    false,
                                    tokens,
                                    GpuiAction::AccountRefresh,
                                    cx,
                                ))
                                .when(acc.user_admin, |el| {
                                    el.child(setting_btn(
                                        "gpui-acc-admin",
                                        "Administration".to_owned(),
                                        false,
                                        tokens,
                                        GpuiAction::AccountOpenAdminPortal,
                                        cx,
                                    ))
                                })
                                .child(setting_btn(
                                    "gpui-acc-signout",
                                    if french {
                                        "Se déconnecter".into()
                                    } else {
                                        "Sign out".into()
                                    },
                                    false,
                                    tokens,
                                    GpuiAction::AccountSignOut,
                                    cx,
                                )),
                        ),
                )
                .child(setting_btn(
                    "gpui-acc-suggest",
                    "Generate strong E2EE sync passphrase".to_owned(),
                    false,
                    tokens,
                    GpuiAction::AccountSuggestPhrase,
                    cx,
                ));
        } else {
            col = col.child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(setting_btn(
                        "gpui-acc-open",
                        "Open Local Credential Store".to_owned(),
                        acc.store_open,
                        tokens,
                        GpuiAction::AccountOpenStore,
                        cx,
                    ))
                    .child(setting_btn(
                        "gpui-acc-signin",
                        "Sign in with Browser".to_owned(),
                        true,
                        tokens,
                        GpuiAction::AccountSignInBrowser,
                        cx,
                    )),
            );
        }
        if !acc.status.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.accent)
                    .child(SharedString::from(acc.status.clone())),
            );
        }
    } else {
        col = col.child(setting_btn(
            "gpui-acc-open-init",
            "Initialize Account Store".to_owned(),
            false,
            tokens,
            GpuiAction::AccountOpenStore,
            cx,
        ));
    }

    col.into_any_element()
}

fn render_stats_page(
    s: &GpuiSettingsSnapshot,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let sp = s.stats_prefs;
    let mut sp_used = sp;
    sp_used.usage_percentage_display = UsagePercentDisplay::Used;
    let mut sp_rem = sp;
    sp_rem.usage_percentage_display = UsagePercentDisplay::Remaining;

    let st = s.stats_snapshot;
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title(
            if french {
                "Statistiques & Consommation"
            } else {
                "Workspace Insights & Token Spend"
            },
            tokens,
        ))
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-stats-used",
                    "Show % used".to_owned(),
                    sp.usage_percentage_display == UsagePercentDisplay::Used,
                    tokens,
                    GpuiAction::UpdateStatsPrefs(sp_used),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-stats-rem",
                    "Show % remaining".to_owned(),
                    sp.usage_percentage_display == UsagePercentDisplay::Remaining,
                    tokens,
                    GpuiAction::UpdateStatsPrefs(sp_rem),
                    cx,
                )),
        )
        .child(
            div()
                .p(px(10.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .font_family(mono_font_family())
                .text_size(px(11.5))
                .text_color(tokens.text_strong)
                .child(SharedString::from(format!(
                    "Projects: {} · Panes: {} · Running agents: {} · Today: ${:.2} · 30d: ${:.2}",
                    st.project_count,
                    st.session_count,
                    st.running_agents,
                    s.usage_summary.today_cost_usd,
                    s.usage_summary.month_cost_usd
                ))),
        )
        .into_any_element()
}

fn render_updates_page(
    snapshot: &GpuiWorkspaceSnapshot,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_title("Tessel Rust Edition", tokens))
        .child(
            div()
                .p(px(10.0))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .font_family(mono_font_family())
                .text_size(px(12.0))
                .text_color(tokens.text_strong)
                .child(SharedString::from(format!(
                    "Build: {} · Active Renderer: {}",
                    snapshot.status_bar.version_build,
                    snapshot.renderer.label(snapshot.french)
                ))),
        )
        .child(
            div()
                .flex()
                .gap(px(6.0))
                .child(setting_btn(
                    "gpui-upd-gh",
                    "GitHub Repository".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::OpenExternalUrl(
                        "https://github.com/mindkind1337/tessel-rust".into(),
                    )),
                    cx,
                ))
                .child(setting_btn(
                    "gpui-upd-readme",
                    "Open README.md".to_owned(),
                    false,
                    tokens,
                    GpuiAction::SettingsPage(SettingsPageAction::OpenNoticeFile("README.md")),
                    cx,
                )),
        )
        .into_any_element()
}

fn section_title(text: &str, tokens: &GpuiThemeTokens) -> AnyElement {
    div()
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tokens.text_strong)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

fn toggle_row(
    id: &'static str,
    label: &str,
    checked: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    toggle_row_owned(id.to_owned(), label.to_owned(), checked, tokens, action, cx)
}

fn toggle_row_owned(
    id: String,
    label: String,
    checked: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    div()
        .id(SharedString::from(id))
        .px(px(10.0))
        .py(px(6.5))
        .rounded(px(tokens.radius_px))
        .bg(tokens.surface)
        .border_1()
        .border_color(if checked {
            tokens.accent
        } else {
            tokens.border
        })
        .flex()
        .items_center()
        .justify_between()
        .hover(|st| st.bg(tokens.surface_2))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(
            div()
                .text_size(px(11.5))
                .text_color(tokens.text_strong)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(1.5))
                .rounded(px(8.0))
                .bg(if checked {
                    tokens.accent
                } else {
                    tokens.surface_2
                })
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if checked {
                    tokens.accent_fg
                } else {
                    tokens.text_dim
                })
                .child(if checked { "ON" } else { "OFF" }),
        )
        .into_any_element()
}

fn setting_btn(
    id: impl Into<SharedString>,
    label: String,
    active: bool,
    tokens: &GpuiThemeTokens,
    action: GpuiAction,
    cx: &mut Context<TesselGpuiRoot>,
) -> AnyElement {
    let shared_id: SharedString = id.into();
    div()
        .id(shared_id)
        .px(px(9.0))
        .py(px(4.5))
        .rounded(px(tokens.radius_px))
        .bg(if active {
            tokens.accent_soft
        } else {
            tokens.surface
        })
        .border_1()
        .border_color(if active { tokens.accent } else { tokens.border })
        .text_size(px(11.0))
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active {
            tokens.text_strong
        } else {
            tokens.text_dim
        })
        .hover(|st| st.bg(tokens.surface_2).text_color(tokens.text_strong))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.dispatch_action(action.clone(), cx);
        }))
        .child(SharedString::from(label))
        .into_any_element()
}
