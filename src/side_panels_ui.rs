//! Native `egui` renderers for:
//! - `SideTab::Dashboard` (`AgentDashboard.vue`)
//! - `SideTab::TaskHistory` (`TaskHistoryPanel.vue`)
//! - `SideTab::History` & `SessionsDialog` (`SessionHistoryPanel.vue`)
//! - `SideTab::Conversations` (`ConversationsPanel.vue`)
//! - `ScmHistoryPanel` docked at bottom of `Changes` (`ScmHistoryPanel.vue`)
//! - `NotesPanel` modal (`NotesPanel.vue`)
//! - `ReviewPanel` modal (`ReviewPanel.vue`)

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use eframe::egui::{self, Color32, FontId, Key, Modifiers, Pos2, RichText, Sense, Stroke, Vec2};

use crate::icons::{self, Icon};
use crate::side_panels_model::{
    ActivityMessage, CommitChangedFile, CommitHistoryRow, ConversationMediaKind,
    ConversationWindow, DashboardGroupBy, DashboardItem, DashboardState, InlineReviewComment,
    NoteBlock, NotesModel, ReviewSnapshot, SESSION_LIMITS, SWIMLANE_PALETTE, SessionHistoryEntry,
    SessionHistoryGroupBy, SessionHistoryScope, TaskHistoryFilter, TaskHistoryRecord,
    TaskHistorySort, build_conversation_threads, build_dashboard, filter_and_sort_task_history,
    filter_sessions, format_duration_compact, format_notes_for_agent,
    format_review_request_changes, format_thread_as_text, history_csv, now_unix_ms,
    parse_note_markdown,
};
use crate::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidePanelsAction {
    FocusPane(u64),
    SleepPanes(Vec<u64>),
    RestartPane(u64),
    ClosePane(u64),
    ExportTaskHistoryCsv(String),
    ClearTaskHistory,
    RefreshSessions,
    ResumeSession {
        agent_id: String,
        session_id: String,
        folder: PathBuf,
    },
    DeleteSession(SessionHistoryEntry),
    CopyText(String),
    RefreshCommits,
    ToggleExpandCommit(String),
    OpenCommitDiff {
        commit_hash: String,
        path: String,
        pinned: bool,
    },
    SaveNotes {
        force: bool,
    },
    OpenNotesInEditor,
    SendPromptToActiveAgent(String),
    CloseNotesModal,
    CloseSessionsModal,
    SelectReviewBranch {
        head_branch: String,
        worktree_path: Option<PathBuf>,
    },
    RefreshReview,
    SelectReviewFile(String),
    OpenReviewDiffInTab(String),
    PushReviewBranch(String),
    MergeReviewBranch {
        base_branch: String,
        head_branch: String,
        expect_head_oid: String,
    },
    CloseReviewModal,
}

#[derive(Default)]
pub struct SidePanelsUiState {
    // Dashboard
    pub dashboard_group: DashboardGroupBy,
    pub dashboard_filter: Option<DashboardState>,
    pub dashboard_query: String,
    pub confirm_sleep_idle: bool,

    // Task History
    pub history_window: TaskHistoryFilter,
    pub history_sort: TaskHistorySort,
    pub history_current_project_only: bool,
    pub history_query: String,
    pub confirm_clear_history: bool,

    // Past Sessions
    pub sessions_scope: SessionHistoryScope,
    pub sessions_group: SessionHistoryGroupBy,
    pub sessions_limit: usize,
    pub sessions_agent_filter: String,
    pub sessions_query: String,
    pub sessions_include_transcripts: bool,
    pub sessions_modal_open: bool,

    // Conversations
    pub conv_window: ConversationWindow,
    pub conv_media: Option<ConversationMediaKind>,
    pub conv_query: String,
    pub selected_thread_key: Option<String>,

    // SCM Commit History
    pub commits_open: bool,
    pub expanded_commits: BTreeSet<String>,
    pub commit_files_cache: BTreeMap<String, Vec<CommitChangedFile>>,

    // Project Notes modal
    pub notes_modal_open: bool,
    pub notes: NotesModel,

    // Branch Review modal
    pub review_modal_open: bool,
    pub review_head_branch: String,
    pub review_worktree_path: Option<PathBuf>,
    pub review_snapshot: Option<ReviewSnapshot>,
    pub review_error: Option<String>,
    pub review_selected_file: Option<String>,
    pub review_selected_diff: String,
    pub review_viewed_sigs: BTreeMap<String, String>,
    pub review_general_note: String,
    pub review_inline_line: usize,
    pub review_inline_draft: String,
    pub review_comments: Vec<InlineReviewComment>,
}

impl SidePanelsUiState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions_limit: SESSION_LIMITS[0],
            commits_open: true,
            review_inline_line: 1,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn any_modal_open(&self) -> bool {
        self.notes_modal_open || self.sessions_modal_open || self.review_modal_open
    }
}

fn chip_button(ui: &mut egui::Ui, label: &str, active: bool, accent: Option<Color32>) -> bool {
    let pal = &theme::active(ui.ctx()).spec().ui;
    let fill = if active {
        accent.unwrap_or(pal.accent).gamma_multiply(0.22_f32)
    } else {
        pal.surface_2
    };
    let border = if active {
        accent.unwrap_or(pal.accent)
    } else {
        pal.border
    };
    let fg = if active { pal.text } else { pal.text_dim };
    ui.add(
        egui::Button::new(RichText::new(label).size(11.5_f32).color(fg))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, border))
            .corner_radius(5.0_f32),
    )
    .clicked()
}

// ============================================================================
// 1. Agent Dashboard (`SideTab::Dashboard`)
// ============================================================================

pub fn show_dashboard_panel(
    ui: &mut egui::Ui,
    state: &mut SidePanelsUiState,
    items: &[DashboardItem],
    french: bool,
) -> Option<SidePanelsAction> {
    let mut out = None;
    let pal = &theme::active(ui.ctx()).spec().ui;
    let vm = build_dashboard(
        items,
        state.dashboard_group,
        state.dashboard_filter,
        &state.dashboard_query,
        french,
    );

    ui.add_space(4.0_f32);
    ui.horizontal_wrapped(|ui| {
        let all_label = if french {
            format!("Tous ({})", vm.total)
        } else {
            format!("All ({})", vm.total)
        };
        if chip_button(ui, &all_label, state.dashboard_filter.is_none(), None) {
            state.dashboard_filter = None;
        }
        for st in DashboardState::ALL {
            let count = vm.counts.get(&st).copied().unwrap_or(0);
            let (r, g, b) = st.rgb();
            let col = Color32::from_rgb(r, g, b);
            let lbl = format!("{} ({count})", st.label(french));
            if chip_button(ui, &lbl, state.dashboard_filter == Some(st), Some(col)) {
                state.dashboard_filter = if state.dashboard_filter == Some(st) {
                    None
                } else {
                    Some(st)
                };
            }
        }
    });

    ui.add_space(4.0_f32);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.dashboard_query)
                .desired_width(150.0_f32)
                .hint_text(if french {
                    "Filtrer les agents…"
                } else {
                    "Filter agents…"
                }),
        );
        let by_status = state.dashboard_group == DashboardGroupBy::Status;
        if chip_button(
            ui,
            if french { "Statut" } else { "Status" },
            by_status,
            None,
        ) {
            state.dashboard_group = DashboardGroupBy::Status;
        }
        if chip_button(
            ui,
            if french { "Projet" } else { "Project" },
            !by_status,
            None,
        ) {
            state.dashboard_group = DashboardGroupBy::Project;
        }
    });

    if !vm.sleepable_pane_ids.is_empty() {
        ui.add_space(4.0_f32);
        ui.horizontal(|ui| {
            let n = vm.sleepable_pane_ids.len();
            if state.confirm_sleep_idle {
                if ui
                    .button(if french {
                        format!("Confirmer la mise en veille ({n})")
                    } else {
                        format!("Confirm sleep ({n})")
                    })
                    .clicked()
                {
                    state.confirm_sleep_idle = false;
                    out = Some(SidePanelsAction::SleepPanes(vm.sleepable_pane_ids.clone()));
                }
                if ui
                    .button(if french { "Annuler" } else { "Cancel" })
                    .clicked()
                {
                    state.confirm_sleep_idle = false;
                }
            } else if ui
                .button(if french {
                    format!("Mettre les inactifs en veille ({n})")
                } else {
                    format!("Sleep idle ({n})")
                })
                .clicked()
            {
                state.confirm_sleep_idle = true;
            }
        });
    } else {
        state.confirm_sleep_idle = false;
    }

    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("side_dashboard_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if vm.groups.is_empty() {
                ui.add_space(24.0_f32);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if french {
                            "Aucun panneau d'agent ouvert."
                        } else {
                            "No open agent panes."
                        })
                        .color(pal.text_dim),
                    );
                });
                return;
            }

            for group in &vm.groups {
                ui.add_space(4.0_f32);
                ui.label(
                    RichText::new(format!("{} ({})", group.heading, group.items.len()))
                        .size(11.5_f32)
                        .strong()
                        .color(pal.text_dim),
                );
                ui.add_space(2.0_f32);
                for item in &group.items {
                    let (r, g, b) = item.state.rgb();
                    let dot_col = Color32::from_rgb(r, g, b);
                    let border_col = if item.is_focused {
                        pal.accent
                    } else {
                        pal.border
                    };
                    let frame = egui::Frame::NONE
                        .fill(pal.surface_2)
                        .stroke(Stroke::new(1.0_f32, border_col))
                        .corner_radius(6.0_f32)
                        .inner_margin(egui::Margin::same(8));
                    let resp = frame
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (dot_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(8.0_f32), Sense::hover());
                                ui.painter()
                                    .circle_filled(dot_rect.center(), 3.8_f32, dot_col);
                                ui.label(RichText::new(&item.title).strong().color(pal.text));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(&item.agent_kind)
                                                .size(11.0_f32)
                                                .color(pal.text_dim),
                                        );
                                    },
                                );
                            });
                            let mut meta = item.project_name.clone();
                            if let Some(br) = &item.branch
                                && !br.is_empty()
                            {
                                if !meta.is_empty() {
                                    meta.push_str(" · ");
                                }
                                meta.push_str(br);
                            }
                            if !meta.is_empty() {
                                ui.label(RichText::new(meta).size(11.0_f32).color(pal.text_dim));
                            }
                            if let Some(tl) = &item.task_line
                                && !tl.is_empty()
                            {
                                ui.label(RichText::new(tl).size(11.5_f32).color(pal.text));
                            }
                        })
                        .response
                        .interact(Sense::click());

                    if resp.clicked() {
                        out = Some(SidePanelsAction::FocusPane(item.pane_id));
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(if french {
                                "Aller au panneau"
                            } else {
                                "Focus pane"
                            })
                            .clicked()
                        {
                            out = Some(SidePanelsAction::FocusPane(item.pane_id));
                            ui.close();
                        }
                        if !item.is_chat
                            && item.state != DashboardState::Sleeping
                            && ui
                                .button(if french {
                                    "Mettre en veille"
                                } else {
                                    "Put to sleep"
                                })
                                .clicked()
                        {
                            out = Some(SidePanelsAction::SleepPanes(vec![item.pane_id]));
                            ui.close();
                        }
                        if !item.is_chat
                            && ui
                                .button(if french { "Redémarrer" } else { "Restart" })
                                .clicked()
                        {
                            out = Some(SidePanelsAction::RestartPane(item.pane_id));
                            ui.close();
                        }
                        if ui.button(if french { "Fermer" } else { "Close" }).clicked() {
                            out = Some(SidePanelsAction::ClosePane(item.pane_id));
                            ui.close();
                        }
                    });
                    ui.add_space(4.0_f32);
                }
            }
        });

    out
}

// ============================================================================
// 2. Task History (`SideTab::TaskHistory`)
// ============================================================================

pub fn show_task_history_panel(
    ui: &mut egui::Ui,
    state: &mut SidePanelsUiState,
    records: &[TaskHistoryRecord],
    active_project_id: Option<u64>,
    french: bool,
) -> Option<SidePanelsAction> {
    let mut out = None;
    let pal = &theme::active(ui.ctx()).spec().ui;
    let proj_filter = if state.history_current_project_only {
        active_project_id
    } else {
        None
    };
    let (filtered, totals) = filter_and_sort_task_history(
        records,
        proj_filter,
        state.history_window,
        state.history_sort,
        &state.history_query,
        now_unix_ms(),
    );

    ui.add_space(4.0_f32);
    ui.horizontal_wrapped(|ui| {
        for win in TaskHistoryFilter::ALL {
            if chip_button(ui, win.label(french), state.history_window == win, None) {
                state.history_window = win;
            }
        }
        ui.separator();
        for s in TaskHistorySort::ALL {
            if chip_button(ui, s.label(french), state.history_sort == s, None) {
                state.history_sort = s;
            }
        }
    });

    ui.add_space(4.0_f32);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.history_query)
                .desired_width(140.0_f32)
                .hint_text(if french {
                    "Rechercher une tâche…"
                } else {
                    "Search completed tasks…"
                }),
        );
        ui.checkbox(
            &mut state.history_current_project_only,
            if french {
                "Projet actif"
            } else {
                "Project only"
            },
        );
        if ui
            .button(if french { "Exporter CSV" } else { "Export CSV" })
            .clicked()
        {
            out = Some(SidePanelsAction::ExportTaskHistoryCsv(history_csv(
                &filtered,
            )));
        }
    });

    ui.add_space(4.0_f32);
    egui::Frame::NONE
        .fill(pal.surface_2)
        .stroke(Stroke::new(1.0_f32, pal.border))
        .corner_radius(6.0_f32)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(if french {
                        format!("{} tâches", totals.count)
                    } else {
                        format!("{} tasks", totals.count)
                    })
                    .strong()
                    .size(12.0_f32),
                );
                ui.label(
                    RichText::new(format!(
                        "· {} · {} tok · ${:.4}",
                        format_duration_compact(totals.duration_ms),
                        totals.tokens,
                        totals.cost_usd
                    ))
                    .size(11.5_f32)
                    .color(pal.text_dim),
                );
                if !records.is_empty() {
                    if state.confirm_clear_history {
                        if ui
                            .small_button(if french { "Confirmer" } else { "Confirm" })
                            .clicked()
                        {
                            state.confirm_clear_history = false;
                            out = Some(SidePanelsAction::ClearTaskHistory);
                        }
                        if ui
                            .small_button(if french { "Annuler" } else { "Cancel" })
                            .clicked()
                        {
                            state.confirm_clear_history = false;
                        }
                    } else if ui
                        .small_button(if french { "Effacer" } else { "Clear" })
                        .clicked()
                    {
                        state.confirm_clear_history = true;
                    }
                }
            });
        });

    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("side_task_history_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if filtered.is_empty() {
                ui.add_space(24.0_f32);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if french {
                            "Aucune tâche terminée dans cette période."
                        } else {
                            "No completed tasks in this window."
                        })
                        .color(pal.text_dim),
                    );
                });
                return;
            }
            for rec in &filtered {
                egui::Frame::NONE
                    .fill(pal.surface_2)
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .corner_radius(6.0_f32)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&rec.title).strong().color(pal.text));
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} ({})",
                                    rec.project_name,
                                    if rec.agent_name.is_empty() {
                                        "—"
                                    } else {
                                        &rec.agent_name
                                    },
                                    if rec.agent_kind.is_empty() {
                                        "manual"
                                    } else {
                                        &rec.agent_kind
                                    }
                                ))
                                .size(11.0_f32)
                                .color(pal.text_dim),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "· {} · ${:.4}",
                                    format_duration_compact(rec.duration_ms),
                                    rec.cost_usd
                                ))
                                .size(11.0_f32)
                                .color(pal.text_dim),
                            );
                        });
                    });
                ui.add_space(4.0_f32);
            }
        });

    out
}

// ============================================================================
// 3. Past Conversations / Session History (`SideTab::History` & `SessionsDialog`)
// ============================================================================

pub fn show_session_history_panel(
    ui: &mut egui::Ui,
    state: &mut SidePanelsUiState,
    entries: &[SessionHistoryEntry],
    active_project_root: Option<&std::path::Path>,
    workspace_roots: &[PathBuf],
    french: bool,
) -> Option<SidePanelsAction> {
    let mut out = None;
    let pal = &theme::active(ui.ctx()).spec().ui;

    ui.add_space(4.0_f32);
    ui.horizontal_wrapped(|ui| {
        for sc in SessionHistoryScope::ALL {
            if chip_button(ui, sc.label(french), state.sessions_scope == sc, None) {
                state.sessions_scope = sc;
            }
        }
        ui.separator();
        for g in [SessionHistoryGroupBy::Folder, SessionHistoryGroupBy::Agent] {
            if chip_button(ui, g.label(french), state.sessions_group == g, None) {
                state.sessions_group = g;
            }
        }
    });

    ui.add_space(4.0_f32);
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.sessions_query)
                .desired_width(150.0_f32)
                .hint_text(if french {
                    "Rechercher une session…"
                } else {
                    "Search past sessions…"
                }),
        );
        for lim in SESSION_LIMITS {
            if chip_button(ui, &lim.to_string(), state.sessions_limit == lim, None) {
                state.sessions_limit = lim;
                out = Some(SidePanelsAction::RefreshSessions);
            }
        }
        if ui
            .button(if french { "Actualiser" } else { "Refresh" })
            .clicked()
        {
            out = Some(SidePanelsAction::RefreshSessions);
        }
    });

    ui.horizontal(|ui| {
        ui.checkbox(
            &mut state.sessions_include_transcripts,
            if french {
                "Rechercher dans le texte des transcriptions (opt-in)"
            } else {
                "Search inside transcript contents (opt-in)"
            },
        );
    });

    let filtered = filter_sessions(
        entries,
        state.sessions_scope,
        active_project_root,
        workspace_roots,
        if state.sessions_agent_filter.is_empty() {
            None
        } else {
            Some(state.sessions_agent_filter.as_str())
        },
        &state.sessions_query,
        state.sessions_include_transcripts,
    );

    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("side_session_history_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if filtered.is_empty() {
                ui.add_space(24.0_f32);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if french {
                            "Aucune conversation passée trouvée."
                        } else {
                            "No past conversations found."
                        })
                        .color(pal.text_dim),
                    );
                });
                return;
            }

            let mut groups: BTreeMap<String, Vec<&SessionHistoryEntry>> = BTreeMap::new();
            for item in &filtered {
                let key = match state.sessions_group {
                    SessionHistoryGroupBy::Folder => item
                        .folder
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("workspace")
                        .to_owned(),
                    SessionHistoryGroupBy::Agent => item.agent_id.clone(),
                };
                groups.entry(key).or_default().push(item);
            }

            for (heading, list) in groups {
                ui.add_space(4.0_f32);
                ui.label(
                    RichText::new(format!("{heading} ({})", list.len()))
                        .size(11.5_f32)
                        .strong()
                        .color(pal.text_dim),
                );
                for entry in list {
                    egui::Frame::NONE
                        .fill(pal.surface_2)
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .corner_radius(6.0_f32)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&entry.title).strong().color(pal.text));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(&entry.agent_id)
                                                .size(11.0_f32)
                                                .color(pal.accent),
                                        );
                                    },
                                );
                            });
                            if let Some(snippet) = &entry.transcript_snippet {
                                ui.label(
                                    RichText::new(format!("…{snippet}…"))
                                        .size(11.0_f32)
                                        .italics()
                                        .color(pal.text_dim),
                                );
                            }
                            ui.horizontal(|ui| {
                                if let Some(pane_id) = entry.open_pane_id {
                                    if ui
                                        .small_button(if french {
                                            "Aller au panneau"
                                        } else {
                                            "Go to pane"
                                        })
                                        .clicked()
                                    {
                                        out = Some(SidePanelsAction::FocusPane(pane_id));
                                    }
                                } else if ui
                                    .small_button(if french { "Reprendre" } else { "Resume" })
                                    .clicked()
                                {
                                    out = Some(SidePanelsAction::ResumeSession {
                                        agent_id: entry.agent_id.clone(),
                                        session_id: entry.id.clone(),
                                        folder: entry.folder.clone(),
                                    });
                                }
                                if entry.file_path.is_some()
                                    && ui
                                        .small_button(if french { "Supprimer" } else { "Delete" })
                                        .clicked()
                                {
                                    out = Some(SidePanelsAction::DeleteSession(entry.clone()));
                                }
                            });
                        });
                    ui.add_space(4.0_f32);
                }
            }
        });

    out
}

// ============================================================================
// 4. Conversations & Activity (`SideTab::Conversations`)
// ============================================================================

pub fn show_conversations_panel(
    ui: &mut egui::Ui,
    state: &mut SidePanelsUiState,
    messages: &[ActivityMessage],
    french: bool,
) -> Option<SidePanelsAction> {
    let mut out = None;
    let pal = &theme::active(ui.ctx()).spec().ui;

    ui.add_space(4.0_f32);
    ui.horizontal_wrapped(|ui| {
        for win in ConversationWindow::ALL {
            if chip_button(ui, win.label(french), state.conv_window == win, None) {
                state.conv_window = win;
            }
        }
        ui.separator();
        if chip_button(
            ui,
            if french { "Tout" } else { "All" },
            state.conv_media.is_none(),
            None,
        ) {
            state.conv_media = None;
        }
        for mk in ConversationMediaKind::ALL {
            if chip_button(ui, mk.label(french), state.conv_media == Some(mk), None) {
                state.conv_media = if state.conv_media == Some(mk) {
                    None
                } else {
                    Some(mk)
                };
            }
        }
    });

    ui.add_space(4.0_f32);
    ui.add(
        egui::TextEdit::singleline(&mut state.conv_query)
            .desired_width(f32::INFINITY)
            .hint_text(if french {
                "Filtrer les fils d'activité…"
            } else {
                "Filter conversation threads…"
            }),
    );

    let threads = build_conversation_threads(
        messages,
        state.conv_window,
        state.conv_media,
        &state.conv_query,
        now_unix_ms(),
    );

    ui.separator();

    if threads.is_empty() {
        ui.add_space(24.0_f32);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(if french {
                    "Aucun fil d'activité récent."
                } else {
                    "No recent conversation threads."
                })
                .color(pal.text_dim),
            );
        });
        return None;
    }

    if state
        .selected_thread_key
        .as_ref()
        .is_none_or(|k| !threads.iter().any(|t| &t.key == k))
    {
        state.selected_thread_key = threads.first().map(|t| t.key.clone());
    }

    egui::ScrollArea::horizontal()
        .id_salt("conv_threads_strip")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for t in &threads {
                    let active = state.selected_thread_key.as_deref() == Some(t.key.as_str());
                    let badge = format!("{} ({})", t.title, t.messages.len());
                    if chip_button(ui, &badge, active, None) {
                        state.selected_thread_key = Some(t.key.clone());
                    }
                }
            });
        });

    if let Some(active_thread) = threads
        .iter()
        .find(|t| state.selected_thread_key.as_deref() == Some(t.key.as_str()))
    {
        ui.add_space(4.0_f32);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&active_thread.title).strong());
            if ui
                .small_button(if french {
                    "Copier le fil"
                } else {
                    "Copy thread"
                })
                .clicked()
            {
                out = Some(SidePanelsAction::CopyText(format_thread_as_text(
                    active_thread,
                )));
            }
            if let Some(pane_id) = active_thread.pane_id
                && ui
                    .small_button(if french {
                        "Aller au panneau"
                    } else {
                        "Go to pane"
                    })
                    .clicked()
            {
                out = Some(SidePanelsAction::FocusPane(pane_id));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("conv_messages_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for msg in &active_thread.messages {
                    egui::Frame::NONE
                        .fill(pal.surface_2)
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .corner_radius(6.0_f32)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("{} → {}", msg.sender, msg.recipient))
                                    .size(11.0_f32)
                                    .strong()
                                    .color(pal.accent),
                            );
                            ui.label(RichText::new(&msg.body).size(12.0_f32).color(pal.text));
                        });
                    ui.add_space(4.0_f32);
                }
            });
    }

    out
}

// ============================================================================
// 5. SCM Commit History (`ScmHistoryPanel` docked at bottom of `Changes`)
// ============================================================================

pub fn show_scm_history_section(
    ui: &mut egui::Ui,
    state: &mut SidePanelsUiState,
    commits: &[CommitHistoryRow],
    french: bool,
) -> Option<SidePanelsAction> {
    let mut out = None;
    let pal = &theme::active(ui.ctx()).spec().ui;

    ui.separator();
    ui.horizontal(|ui| {
        let chevron = if state.commits_open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(14.0_f32), Sense::hover());
        icons::paint(ui.painter(), icon_rect, chevron, pal.text_dim);
        let heading = format!("Commits ({})", commits.len());
        if ui
            .add(egui::Button::new(RichText::new(heading).strong().size(12.0_f32)).frame(false))
            .clicked()
        {
            state.commits_open = !state.commits_open;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(if french { "Actualiser" } else { "Refresh" })
                .clicked()
            {
                out = Some(SidePanelsAction::RefreshCommits);
            }
        });
    });

    if !state.commits_open {
        return out;
    }

    egui::ScrollArea::vertical()
        .id_salt("scm_history_commits_scroll")
        .max_height(240.0_f32)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if commits.is_empty() {
                ui.label(
                    RichText::new(if french {
                        "Aucun commit trouvé."
                    } else {
                        "No commits yet."
                    })
                    .size(11.5_f32)
                    .color(pal.text_dim),
                );
                return;
            }
            for row in commits {
                let expanded = state.expanded_commits.contains(&row.node.hash);
                ui.horizontal(|ui| {
                    // 5-color swimlane graph gutter
                    let lanes_w =
                        (row.active_lanes.max(1) as f32 * 10.0_f32).clamp(14.0_f32, 64.0_f32);
                    let (graph_rect, _) =
                        ui.allocate_exact_size(Vec2::new(lanes_w, 22.0_f32), Sense::hover());
                    for l in 0..row.active_lanes {
                        let (cr, cg, cb) = SWIMLANE_PALETTE[l % SWIMLANE_PALETTE.len()];
                        let col = Color32::from_rgb(cr, cg, cb);
                        let x = graph_rect.min.x + 6.0_f32 + l as f32 * 10.0_f32;
                        ui.painter().line_segment(
                            [
                                Pos2::new(x, graph_rect.min.y),
                                Pos2::new(x, graph_rect.max.y),
                            ],
                            Stroke::new(1.5_f32, col.gamma_multiply(0.65_f32)),
                        );
                        if l == row.lane {
                            let center = Pos2::new(x, graph_rect.center().y);
                            if row.is_merge {
                                ui.painter().circle_filled(center, 4.0_f32, pal.surface);
                                ui.painter().circle_stroke(
                                    center,
                                    3.8_f32,
                                    Stroke::new(1.6_f32, col),
                                );
                            } else {
                                ui.painter().circle_filled(center, 3.5_f32, col);
                            }
                        }
                    }

                    for r in row.node.refs.iter().take(2) {
                        ui.label(
                            RichText::new(format!("[{r}]"))
                                .size(10.5_f32)
                                .color(pal.accent),
                        );
                    }
                    let subject_btn = ui.add(
                        egui::Button::new(
                            RichText::new(&row.node.subject)
                                .size(12.0_f32)
                                .color(pal.text),
                        )
                        .frame(false)
                        .truncate(),
                    );
                    if subject_btn.clicked() {
                        out = Some(SidePanelsAction::ToggleExpandCommit(row.node.hash.clone()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(&row.node.short_hash)
                                .monospace()
                                .size(10.5_f32)
                                .color(pal.text_dim),
                        );
                    });
                });

                if expanded && let Some(files) = state.commit_files_cache.get(&row.node.hash) {
                    for f in files {
                        ui.horizontal(|ui| {
                            ui.add_space(20.0_f32);
                            ui.label(
                                RichText::new(&f.status)
                                    .monospace()
                                    .size(11.0_f32)
                                    .color(pal.accent),
                            );
                            let file_resp = ui.add(
                                egui::Button::new(
                                    RichText::new(&f.path).size(11.5_f32).color(pal.text_dim),
                                )
                                .frame(false),
                            );
                            if file_resp.double_clicked() {
                                out = Some(SidePanelsAction::OpenCommitDiff {
                                    commit_hash: row.node.hash.clone(),
                                    path: f.path.clone(),
                                    pinned: true,
                                });
                            } else if file_resp.clicked() {
                                out = Some(SidePanelsAction::OpenCommitDiff {
                                    commit_hash: row.node.hash.clone(),
                                    path: f.path.clone(),
                                    pinned: false,
                                });
                            }
                        });
                    }
                }
            }
        });

    out
}

// ============================================================================
// 6. Project Notes Modal (`NotesPanel.vue` — `680x540px`)
// ============================================================================

pub fn show_notes_modal(
    ctx: &egui::Context,
    state: &mut SidePanelsUiState,
    project_name: &str,
    french: bool,
) -> Option<SidePanelsAction> {
    if !state.notes_modal_open {
        return None;
    }
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(SidePanelsAction::CloseNotesModal);
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
        return Some(SidePanelsAction::SaveNotes { force: false });
    }

    egui::Area::new(egui::Id::new("notes_modal_scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            let (_, scrim_resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            if scrim_resp.clicked() {
                out = Some(SidePanelsAction::CloseNotesModal);
            }
        });

    egui::Window::new("project_notes_modal")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(680.0_f32, 520.0_f32))
        .frame(
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .corner_radius(8.0_f32)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (ic_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0_f32), Sense::hover());
                icons::paint(ui.painter(), ic_rect, Icon::NotebookPen, pal.accent);
                let title = if french {
                    format!("Notes du projet — {project_name}")
                } else {
                    format!("Project Notes — {project_name}")
                };
                ui.label(RichText::new(title).strong().size(14.0_f32));
                if state.notes.is_dirty() {
                    ui.label(
                        RichText::new(if french {
                            "● Non enregistré"
                        } else {
                            "● Unsaved"
                        })
                        .size(11.5_f32)
                        .color(Color32::from_rgb(0xf5, 0xa5, 0x24)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        out = Some(SidePanelsAction::CloseNotesModal);
                    }
                    let toggle_lbl = if state.notes.preview_mode {
                        if french { "Éditer" } else { "Edit" }
                    } else if french {
                        "Aperçu"
                    } else {
                        "Preview"
                    };
                    if ui.button(toggle_lbl).clicked() {
                        state.notes.preview_mode = !state.notes.preview_mode;
                    }
                });
            });

            if state.notes.conflict.is_some() {
                ui.add_space(6.0_f32);
                egui::Frame::NONE
                    .fill(Color32::from_rgb(0x42, 0x2c, 0x12))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0xf5, 0xa5, 0x24)))
                    .corner_radius(6.0_f32)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(if french {
                                "`.tessel/notes.md` a changé sur le disque pendant votre édition."
                            } else {
                                "`.tessel/notes.md` changed on disk while you were editing."
                            }));
                            if ui
                                .button(if french {
                                    "Charger la version disque"
                                } else {
                                    "Load their version"
                                })
                                .clicked()
                            {
                                state.notes.accept_disk_version();
                            }
                            if ui
                                .button(if french {
                                    "Garder la mienne"
                                } else {
                                    "Keep mine"
                                })
                                .clicked()
                            {
                                out = Some(SidePanelsAction::SaveNotes { force: true });
                            }
                        });
                    });
            }

            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("notes_modal_body")
                .max_height(390.0_f32)
                .show(ui, |ui| {
                    if state.notes.preview_mode {
                        for block in parse_note_markdown(&state.notes.text) {
                            match block {
                                NoteBlock::Heading(level, text) => {
                                    let sz = match level {
                                        1 => 17.0_f32,
                                        2 => 15.0_f32,
                                        _ => 13.5_f32,
                                    };
                                    ui.label(RichText::new(text).strong().size(sz));
                                }
                                NoteBlock::TaskItem { checked, text } => {
                                    let mark = if checked { "☑" } else { "☐" };
                                    ui.label(format!("{mark} {text}"));
                                }
                                NoteBlock::Bullet(text) => {
                                    ui.label(format!("• {text}"));
                                }
                                NoteBlock::Quote(text) => {
                                    ui.label(
                                        RichText::new(format!("│ {text}"))
                                            .italics()
                                            .color(pal.text_dim),
                                    );
                                }
                                NoteBlock::CodeBlock(code) => {
                                    egui::Frame::NONE
                                        .fill(pal.surface_2)
                                        .corner_radius(4.0_f32)
                                        .inner_margin(egui::Margin::same(8))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(code).monospace().size(12.0_f32),
                                            );
                                        });
                                }
                                NoteBlock::Paragraph(text) => {
                                    ui.label(text);
                                }
                            }
                            ui.add_space(4.0_f32);
                        }
                    } else {
                        ui.add(
                            egui::TextEdit::multiline(&mut state.notes.text)
                                .desired_rows(20)
                                .desired_width(f32::INFINITY)
                                .font(FontId::monospace(12.5_f32))
                                .hint_text(if french {
                                    "Notes Markdown partagées dans `.tessel/notes.md`…"
                                } else {
                                    "Shared Markdown notes saved to `.tessel/notes.md`…"
                                }),
                        );
                    }
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(if french {
                        "Enregistrer (Ctrl+S)"
                    } else {
                        "Save (Ctrl+S)"
                    })
                    .clicked()
                {
                    out = Some(SidePanelsAction::SaveNotes { force: false });
                }
                if ui
                    .button(if french {
                        "Ouvrir dans l'éditeur"
                    } else {
                        "Open in editor"
                    })
                    .clicked()
                {
                    out = Some(SidePanelsAction::OpenNotesInEditor);
                }
                if !state.notes.text.trim().is_empty()
                    && ui
                        .button(if french {
                            "Envoyer à l'agent"
                        } else {
                            "Send notes to agent"
                        })
                        .clicked()
                {
                    out = Some(SidePanelsAction::SendPromptToActiveAgent(
                        format_notes_for_agent(&state.notes.text),
                    ));
                }
            });
        });

    out
}

// ============================================================================
// 7. Past Sessions Modal (`SessionsDialog` — `720x680px`)
// ============================================================================

pub fn show_sessions_modal(
    ctx: &egui::Context,
    state: &mut SidePanelsUiState,
    entries: &[SessionHistoryEntry],
    active_project_root: Option<&std::path::Path>,
    workspace_roots: &[PathBuf],
    french: bool,
) -> Option<SidePanelsAction> {
    if !state.sessions_modal_open {
        return None;
    }
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(SidePanelsAction::CloseSessionsModal);
    }

    egui::Area::new(egui::Id::new("sessions_modal_scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            let (_, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            if resp.clicked() {
                out = Some(SidePanelsAction::CloseSessionsModal);
            }
        });

    egui::Window::new("past_sessions_modal")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(720.0_f32, 620.0_f32))
        .frame(
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .corner_radius(8.0_f32)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (ic_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0_f32), Sense::hover());
                icons::paint(ui.painter(), ic_rect, Icon::History, pal.accent);
                ui.label(
                    RichText::new(if french {
                        "Conversations passées"
                    } else {
                        "Past Conversations"
                    })
                    .strong()
                    .size(14.0_f32),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        out = Some(SidePanelsAction::CloseSessionsModal);
                    }
                });
            });
            ui.separator();
            if let Some(act) = show_session_history_panel(
                ui,
                state,
                entries,
                active_project_root,
                workspace_roots,
                french,
            ) {
                out = Some(act);
            }
        });

    out
}

// ============================================================================
// 8. Task Branch Review Modal (`ReviewPanel.vue` — `840x600px`)
// ============================================================================

pub fn show_review_modal(
    ctx: &egui::Context,
    state: &mut SidePanelsUiState,
    candidate_branches: &[(String, Option<PathBuf>)],
    french: bool,
) -> Option<SidePanelsAction> {
    if !state.review_modal_open {
        return None;
    }
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(SidePanelsAction::CloseReviewModal);
    }

    egui::Area::new(egui::Id::new("review_modal_scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            let (_, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            if resp.clicked() {
                out = Some(SidePanelsAction::CloseReviewModal);
            }
        });

    egui::Window::new("branch_review_modal")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(840.0_f32, 600.0_f32))
        .frame(
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .corner_radius(8.0_f32)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (ic_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0_f32), Sense::hover());
                icons::paint(ui.painter(), ic_rect, Icon::GitPullRequest, pal.accent);
                ui.label(
                    RichText::new(if french {
                        "Revue de branche"
                    } else {
                        "Review Task Branch"
                    })
                    .strong()
                    .size(14.0_f32),
                );
                for (br, wt) in candidate_branches {
                    if chip_button(ui, br, state.review_head_branch == *br, None) {
                        out = Some(SidePanelsAction::SelectReviewBranch {
                            head_branch: br.clone(),
                            worktree_path: wt.clone(),
                        });
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        out = Some(SidePanelsAction::CloseReviewModal);
                    }
                    if ui
                        .button(if french { "Actualiser" } else { "Refresh" })
                        .clicked()
                    {
                        out = Some(SidePanelsAction::RefreshReview);
                    }
                });
            });

            ui.separator();

            if let Some(err) = &state.review_error {
                ui.label(RichText::new(err).color(Color32::from_rgb(0xf8, 0x71, 0x71)));
            }

            let Some(snap) = state.review_snapshot.clone() else {
                ui.label(
                    RichText::new(if french {
                        "Sélectionnez un worktree ou une branche de tâche à examiner."
                    } else {
                        "Select a worktree or task branch above to review its changes."
                    })
                    .color(pal.text_dim),
                );
                return;
            };

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} ← {} (+{} / -{} commits)",
                        snap.base_branch, snap.head_branch, snap.ahead, snap.behind
                    ))
                    .strong(),
                );
                if !snap.conflicts.is_empty() {
                    ui.label(
                        RichText::new(if french {
                            format!("⚠ {} conflits prévus", snap.conflicts.len())
                        } else {
                            format!("⚠ {} merge conflicts detected", snap.conflicts.len())
                        })
                        .color(Color32::from_rgb(0xf8, 0x71, 0x71)),
                    );
                }
                if snap.worktree_dirty {
                    ui.label(
                        RichText::new(if french {
                            "⚠ Modifications non commises dans le worktree"
                        } else {
                            "⚠ Uncommitted changes in worktree"
                        })
                        .color(Color32::from_rgb(0xf5, 0xa5, 0x24)),
                    );
                }
                if !snap.base_dirty_overlap.is_empty() {
                    ui.label(
                        RichText::new(if french {
                            "⚠ Fichiers modifiés localement sur la branche cible"
                        } else {
                            "⚠ Dirty overlap on target checkout"
                        })
                        .color(Color32::from_rgb(0xf5, 0xa5, 0x24)),
                    );
                }
            });

            ui.add_space(6.0_f32);

            ui.columns(2, |cols| {
                // Left column: files + Viewed blob-signature checkboxes
                cols[0].label(
                    RichText::new(if french {
                        format!("Fichiers ({})", snap.files.len())
                    } else {
                        format!("Changed files ({})", snap.files.len())
                    })
                    .strong()
                    .size(12.0_f32),
                );
                egui::ScrollArea::vertical()
                    .id_salt("review_files_scroll")
                    .max_height(330.0_f32)
                    .show(&mut cols[0], |ui| {
                        for f in &snap.files {
                            let mut viewed = state
                                .review_viewed_sigs
                                .get(&f.path)
                                .is_some_and(|sig| sig == &f.blob_sig);
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut viewed, "").changed() {
                                    if viewed {
                                        state
                                            .review_viewed_sigs
                                            .insert(f.path.clone(), f.blob_sig.clone());
                                    } else {
                                        state.review_viewed_sigs.remove(&f.path);
                                    }
                                }
                                let active =
                                    state.review_selected_file.as_deref() == Some(f.path.as_str());
                                let label = format!(
                                    "{}  {} (+{} -{})",
                                    f.status, f.path, f.added, f.removed
                                );
                                if chip_button(ui, &label, active, None) {
                                    out = Some(SidePanelsAction::SelectReviewFile(f.path.clone()));
                                }
                            });
                        }
                    });

                // Right column: diff preview + inline comment input
                if let Some(sel_file) = state.review_selected_file.clone() {
                    cols[1].horizontal(|ui| {
                        ui.label(RichText::new(&sel_file).strong().size(12.0_f32));
                        if ui
                            .small_button(if french {
                                "Ouvrir l'onglet diff"
                            } else {
                                "Open full diff"
                            })
                            .clicked()
                        {
                            out = Some(SidePanelsAction::OpenReviewDiffInTab(sel_file.clone()));
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("review_diff_scroll")
                        .max_height(230.0_f32)
                        .show(&mut cols[1], |ui| {
                            for line in state.review_selected_diff.lines().take(250) {
                                let col = if line.starts_with('+') {
                                    Color32::from_rgb(0x81, 0xb8, 0x8b)
                                } else if line.starts_with('-') {
                                    Color32::from_rgb(0xf8, 0x71, 0x71)
                                } else {
                                    pal.text_dim
                                };
                                ui.label(RichText::new(line).monospace().size(11.0_f32).color(col));
                            }
                        });
                    cols[1].horizontal(|ui| {
                        ui.label(if french { "Ligne :" } else { "Line:" });
                        ui.add(
                            egui::DragValue::new(&mut state.review_inline_line).range(1..=99_999),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut state.review_inline_draft)
                                .desired_width(180.0_f32)
                                .hint_text(if french {
                                    "Commentaire en ligne…"
                                } else {
                                    "Inline comment…"
                                }),
                        );
                        if ui
                            .small_button(if french { "Ajouter" } else { "Add" })
                            .clicked()
                            && !state.review_inline_draft.trim().is_empty()
                        {
                            state.review_comments.push(InlineReviewComment {
                                file: sel_file,
                                line: state.review_inline_line.max(1),
                                body: std::mem::take(&mut state.review_inline_draft),
                            });
                        }
                    });
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.review_general_note)
                        .desired_width(300.0_f32)
                        .hint_text(if french {
                            "Note globale pour demander des modifications…"
                        } else {
                            "Overall note for Request changes…"
                        }),
                );
                let has_feedback = !state.review_general_note.trim().is_empty()
                    || !state.review_comments.is_empty();
                if ui
                    .add_enabled(
                        has_feedback,
                        egui::Button::new(if french {
                            format!("Demander des modifs ({})", state.review_comments.len())
                        } else {
                            format!("Request changes ({})", state.review_comments.len())
                        }),
                    )
                    .clicked()
                {
                    let msg = format_review_request_changes(
                        &snap.head_branch,
                        &state.review_general_note,
                        &state.review_comments,
                    );
                    state.review_general_note.clear();
                    state.review_comments.clear();
                    out = Some(SidePanelsAction::SendPromptToActiveAgent(msg));
                }
                if ui.button(if french { "Pousser" } else { "Push" }).clicked() {
                    out = Some(SidePanelsAction::PushReviewBranch(snap.head_branch.clone()));
                }
                if ui
                    .add_enabled(
                        snap.can_clean_merge(),
                        egui::Button::new(if french {
                            "Fusionner (--no-ff)"
                        } else {
                            "Merge (--no-ff)"
                        }),
                    )
                    .clicked()
                {
                    out = Some(SidePanelsAction::MergeReviewBranch {
                        base_branch: snap.base_branch.clone(),
                        head_branch: snap.head_branch.clone(),
                        expect_head_oid: snap.head_oid.clone(),
                    });
                }
            });
        });

    out
}
