//! Native `egui` renderers for:
//! - `AddProjectDialog` (`512px` modal: Open Folder + nested Git repos, Clone, `git init`, SSH `RemoteFolderBrowser`)
//! - `GitHubDialog` (`760x560px` modal: `gh` Issues, PRs, CI check rollups, worktree launch & `Fix failing checks`)
//! - Toolbar `UsageMenu` (`360px` popover + Speedometer `Icon::Gauge`)
//! - Toolbar `NotificationsMenu` (`360px` popover + `Icon::Bell` unread badge + non-focus-stealing `Agent request` pill)
//! - Sidebar project-group assignment modal

use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, Response, RichText, Sense, Stroke,
    Vec2,
};

use crate::dialogs_toolbar_model::{
    GhIssue, GhPullRequest, MAX_DISCOVERED_REPOS, MAX_NESTED_GIT_DEPTH, NotificationCenter,
    UsageAlertLevel, UsageSummary, default_repo_folder_name, discover_nested_git_repos,
    format_fix_failing_checks_prompt,
};
use crate::icons::{self, Icon};
use crate::theme;
use crate::theme_palettes::UiPalette;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddProjectTab {
    #[default]
    LocalFolder,
    CloneRepo,
    NewProject,
    SshRemote,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitHubTab {
    #[default]
    PullRequests,
    Issues,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogsToolbarAction {
    // AddProjectDialog
    PickLocalProjectFolder,
    PickCloneParentFolder,
    PickInitParentFolder,
    AddProjects(Vec<PathBuf>),
    CloneRepository {
        url: String,
        parent_dir: PathBuf,
        folder_name: String,
    },
    InitNewProject {
        parent_dir: PathBuf,
        folder_name: String,
    },
    BrowseRemoteDirs {
        host_alias: String,
        remote_dir: String,
    },
    OpenSshWorkspace {
        host_alias: String,
        remote_dir: String,
    },
    CloseAddProjectModal,

    // GitHubDialog
    RefreshGitHub,
    StartWorktreeFromSlug(String),
    SendPromptToActiveAgent(String),
    OpenExternalUrl(String),
    CloseGitHubModal,

    // Project group modal
    SaveProjectGroup {
        project_id: u64,
        group: Option<String>,
    },
    CloseProjectGroupModal,

    // Usage & Notifications toolbar popovers
    OpenStatsSettings,
    OpenAlertsSettings,
    FocusPane(u64),
    MarkAllNotificationsRead,
    ClearNotifications,
}

#[derive(Default)]
pub struct DialogsToolbarUiState {
    // AddProjectDialog (`512px`)
    pub add_project_open: bool,
    pub add_tab: AddProjectTab,
    pub picked_folder: Option<PathBuf>,
    pub nested_repos: Vec<PathBuf>,
    pub selected_nested_repos: BTreeSet<PathBuf>,
    pub clone_url: String,
    pub clone_parent: PathBuf,
    pub clone_folder_name: String,
    pub init_parent: PathBuf,
    pub init_folder_name: String,
    pub ssh_selected_alias: String,
    pub ssh_remote_dir: String,
    pub ssh_remote_entries: Vec<String>,
    pub add_error: String,

    // GitHubDialog (`760x560px`)
    pub github_open: bool,
    pub github_tab: GitHubTab,
    pub github_query: String,
    pub github_issues: Vec<GhIssue>,
    pub github_prs: Vec<GhPullRequest>,
    pub github_error: String,

    // Sidebar Project Group modal
    pub group_modal_project: Option<u64>,
    pub group_modal_draft: String,
}

impl DialogsToolbarUiState {
    #[must_use]
    pub fn new() -> Self {
        let default_parent = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            clone_parent: default_parent.clone(),
            init_parent: default_parent,
            ssh_remote_dir: "~".into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn any_modal_open(&self) -> bool {
        self.add_project_open || self.github_open || self.group_modal_project.is_some()
    }

    pub fn set_picked_local_folder(&mut self, folder: PathBuf) {
        self.nested_repos =
            discover_nested_git_repos(&folder, MAX_NESTED_GIT_DEPTH, MAX_DISCOVERED_REPOS);
        self.selected_nested_repos = self.nested_repos.iter().cloned().collect();
        self.picked_folder = Some(folder);
        self.add_error.clear();
    }
}

fn tab_chip(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let pal = &theme::active(ui.ctx()).spec().ui;
    let fill = if active {
        pal.accent.gamma_multiply(0.22_f32)
    } else {
        pal.surface_2
    };
    let border = if active { pal.accent } else { pal.border };
    let fg = if active { pal.text } else { pal.text_dim };
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0_f32).color(fg))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, border))
            .corner_radius(6.0_f32),
    )
    .clicked()
}

// ============================================================================
// 1. AddProjectDialog (`512px` modal — `AddProjectDialog.vue` parity)
// ============================================================================

pub fn show_add_project_modal(
    ctx: &egui::Context,
    state: &mut DialogsToolbarUiState,
    ssh_aliases: &[String],
    french: bool,
) -> Option<DialogsToolbarAction> {
    if !state.add_project_open {
        return None;
    }
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(DialogsToolbarAction::CloseAddProjectModal);
    }

    egui::Area::new(egui::Id::new("add_project_scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            let (_, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            if resp.clicked() {
                out = Some(DialogsToolbarAction::CloseAddProjectModal);
            }
        });

    egui::Window::new("add_project_dialog_512")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(512.0_f32, 430.0_f32))
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
                icons::paint(ui.painter(), ic_rect, Icon::FolderPlus, pal.accent);
                ui.label(
                    RichText::new(if french {
                        "Ajouter un espace de travail"
                    } else {
                        "Add Workspace"
                    })
                    .strong()
                    .size(14.0_f32),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        out = Some(DialogsToolbarAction::CloseAddProjectModal);
                    }
                });
            });

            ui.add_space(6.0_f32);
            ui.horizontal_wrapped(|ui| {
                if tab_chip(
                    ui,
                    if french { "Dossier local" } else { "Open Folder" },
                    state.add_tab == AddProjectTab::LocalFolder,
                ) {
                    state.add_tab = AddProjectTab::LocalFolder;
                    state.add_error.clear();
                }
                if tab_chip(
                    ui,
                    if french { "Cloner Git" } else { "Clone Repo" },
                    state.add_tab == AddProjectTab::CloneRepo,
                ) {
                    state.add_tab = AddProjectTab::CloneRepo;
                    state.add_error.clear();
                }
                if tab_chip(
                    ui,
                    if french {
                        "Nouveau projet"
                    } else {
                        "New Project"
                    },
                    state.add_tab == AddProjectTab::NewProject,
                ) {
                    state.add_tab = AddProjectTab::NewProject;
                    state.add_error.clear();
                }
                if tab_chip(
                    ui,
                    if french { "Dossier SSH" } else { "SSH Folder" },
                    state.add_tab == AddProjectTab::SshRemote,
                ) {
                    state.add_tab = AddProjectTab::SshRemote;
                    state.add_error.clear();
                }
            });

            ui.separator();

            if !state.add_error.is_empty() {
                ui.label(RichText::new(&state.add_error).color(pal.danger).size(12.0_f32));
                ui.add_space(4.0_f32);
            }

            match state.add_tab {
                AddProjectTab::LocalFolder => {
                    ui.horizontal(|ui| {
                        if ui
                            .button(if french {
                                "Parcourir un dossier…"
                            } else {
                                "Browse folder…"
                            })
                            .clicked()
                        {
                            out = Some(DialogsToolbarAction::PickLocalProjectFolder);
                        }
                        if let Some(folder) = &state.picked_folder {
                            ui.label(
                                RichText::new(folder.display().to_string())
                                    .monospace()
                                    .size(11.5_f32)
                                    .color(pal.text_dim),
                            );
                        }
                    });

                    if let Some(folder) = state.picked_folder.clone() {
                        ui.add_space(8.0_f32);
                        if !state.nested_repos.is_empty() {
                            ui.label(
                                RichText::new(if french {
                                    format!(
                                        "{} dépôts Git imbriqués détectés :"
                                        ,
                                        state.nested_repos.len()
                                    )
                                } else {
                                    format!(
                                        "Discovered {} nested Git repositories:",
                                        state.nested_repos.len()
                                    )
                                })
                                .strong()
                                .size(12.0_f32),
                            );
                            egui::ScrollArea::vertical()
                                .id_salt("nested_git_repos_scroll")
                                .max_height(190.0_f32)
                                .show(ui, |ui| {
                                    for repo in &state.nested_repos {
                                        let mut checked =
                                            state.selected_nested_repos.contains(repo);
                                        let rel = repo
                                            .strip_prefix(&folder)
                                            .unwrap_or(repo)
                                            .display()
                                            .to_string();
                                        if ui.checkbox(&mut checked, rel).changed() {
                                            if checked {
                                                state.selected_nested_repos.insert(repo.clone());
                                            } else {
                                                state.selected_nested_repos.remove(repo);
                                            }
                                        }
                                    }
                                });
                            ui.add_space(6.0_f32);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        !state.selected_nested_repos.is_empty(),
                                        egui::Button::new(if french {
                                            format!(
                                                "Ajouter la sélection ({})",
                                                state.selected_nested_repos.len()
                                            )
                                        } else {
                                            format!(
                                                "Add selected repos ({})",
                                                state.selected_nested_repos.len()
                                            )
                                        }),
                                    )
                                    .clicked()
                                {
                                    out = Some(DialogsToolbarAction::AddProjects(
                                        state.selected_nested_repos.iter().cloned().collect(),
                                    ));
                                }
                                if ui
                                    .button(if french {
                                        "Ajouter le dossier parent"
                                    } else {
                                        "Add parent folder instead"
                                    })
                                    .clicked()
                                {
                                    out = Some(DialogsToolbarAction::AddProjects(vec![folder]));
                                }
                            });
                        } else if ui
                            .button(if french {
                                "Ouvrir ce dossier"
                            } else {
                                "Open this folder"
                            })
                            .clicked()
                        {
                            out = Some(DialogsToolbarAction::AddProjects(vec![folder]));
                        }
                    }
                }
                AddProjectTab::CloneRepo => {
                    ui.label(if french {
                        "URL du dépôt Git :"
                    } else {
                        "Git repository URL:"
                    });
                    let url_resp = ui.add(
                        egui::TextEdit::singleline(&mut state.clone_url)
                            .desired_width(f32::INFINITY)
                            .hint_text("https://github.com/org/repo.git"),
                    );
                    if url_resp.changed() && !state.clone_url.trim().is_empty() {
                        state.clone_folder_name = default_repo_folder_name(&state.clone_url);
                    }
                    ui.add_space(6.0_f32);
                    ui.horizontal(|ui| {
                        ui.label(if french {
                            "Dossier parent :"
                        } else {
                            "Destination parent:"
                        });
                        if ui
                            .small_button(if french { "Choisir…" } else { "Browse…" })
                            .clicked()
                        {
                            out = Some(DialogsToolbarAction::PickCloneParentFolder);
                        }
                    });
                    ui.label(
                        RichText::new(state.clone_parent.display().to_string())
                            .monospace()
                            .size(11.5_f32)
                            .color(pal.text_dim),
                    );
                    ui.add_space(6.0_f32);
                    ui.label(if french {
                        "Nom du dossier cible :"
                    } else {
                        "Target folder name:"
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut state.clone_folder_name)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(10.0_f32);
                    if ui
                        .add_enabled(
                            !state.clone_url.trim().is_empty()
                                && !state.clone_folder_name.trim().is_empty(),
                            egui::Button::new(if french {
                                "Cloner et ouvrir"
                            } else {
                                "Clone & open"
                            }),
                        )
                        .clicked()
                    {
                        out = Some(DialogsToolbarAction::CloneRepository {
                            url: state.clone_url.clone(),
                            parent_dir: state.clone_parent.clone(),
                            folder_name: state.clone_folder_name.clone(),
                        });
                    }
                }
                AddProjectTab::NewProject => {
                    ui.horizontal(|ui| {
                        ui.label(if french {
                            "Dossier parent :"
                        } else {
                            "Parent directory:"
                        });
                        if ui
                            .small_button(if french { "Choisir…" } else { "Browse…" })
                            .clicked()
                        {
                            out = Some(DialogsToolbarAction::PickInitParentFolder);
                        }
                    });
                    ui.label(
                        RichText::new(state.init_parent.display().to_string())
                            .monospace()
                            .size(11.5_f32)
                            .color(pal.text_dim),
                    );
                    ui.add_space(6.0_f32);
                    ui.label(if french {
                        "Nom du nouveau projet (initialisé avec `git init`) :"
                    } else {
                        "New project folder name (initialized with `git init`):"
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut state.init_folder_name)
                            .desired_width(f32::INFINITY)
                            .hint_text("my-new-app"),
                    );
                    ui.add_space(10.0_f32);
                    if ui
                        .add_enabled(
                            !state.init_folder_name.trim().is_empty(),
                            egui::Button::new(if french {
                                "Créer et initialiser Git"
                            } else {
                                "Create & git init"
                            }),
                        )
                        .clicked()
                    {
                        out = Some(DialogsToolbarAction::InitNewProject {
                            parent_dir: state.init_parent.clone(),
                            folder_name: state.init_folder_name.clone(),
                        });
                    }
                }
                AddProjectTab::SshRemote => {
                    if ssh_aliases.is_empty() {
                        ui.label(
                            RichText::new(if french {
                                "Aucun hôte SSH configuré. Ajoutez d'abord un hôte dans Paramètres > Hôtes SSH."
                            } else {
                                "No SSH hosts configured yet. Add an SSH host in Settings > SSH Hosts first."
                            })
                            .color(pal.text_dim),
                        );
                    } else {
                        if state.ssh_selected_alias.is_empty()
                            && let Some(first) = ssh_aliases.first()
                        {
                            state.ssh_selected_alias = first.clone();
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if french { "Hôte SSH :" } else { "SSH host:" });
                            for alias in ssh_aliases {
                                if tab_chip(ui, alias, state.ssh_selected_alias == *alias) {
                                    state.ssh_selected_alias = alias.clone();
                                    state.ssh_remote_entries.clear();
                                }
                            }
                        });
                        ui.add_space(6.0_f32);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.ssh_remote_dir)
                                    .desired_width(280.0_f32)
                                    .hint_text("~/projects"),
                            );
                            if ui
                                .button(if french { "Parcourir" } else { "List folders" })
                                .clicked()
                            {
                                out = Some(DialogsToolbarAction::BrowseRemoteDirs {
                                    host_alias: state.ssh_selected_alias.clone(),
                                    remote_dir: state.ssh_remote_dir.clone(),
                                });
                            }
                        });
                        if !state.ssh_remote_entries.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("ssh_remote_dirs_scroll")
                                .max_height(150.0_f32)
                                .show(ui, |ui| {
                                    for sub in state.ssh_remote_entries.clone() {
                                        if ui.button(format!("📁 {sub}/")).clicked() {
                                            let base = state.ssh_remote_dir.trim_end_matches('/');
                                            state.ssh_remote_dir = format!("{base}/{sub}");
                                        }
                                    }
                                });
                        }
                        ui.add_space(8.0_f32);
                        if ui
                            .button(if french {
                                "Ouvrir la session SSH"
                            } else {
                                "Open SSH session here"
                            })
                            .clicked()
                        {
                            out = Some(DialogsToolbarAction::OpenSshWorkspace {
                                host_alias: state.ssh_selected_alias.clone(),
                                remote_dir: state.ssh_remote_dir.clone(),
                            });
                        }
                    }
                }
            }
        });

    out
}

// ============================================================================
// 2. GitHubDialog (`760x560px` modal — `GitHubDialog.vue` parity)
// ============================================================================

pub fn show_github_modal(
    ctx: &egui::Context,
    state: &mut DialogsToolbarUiState,
    project_name: &str,
    french: bool,
) -> Option<DialogsToolbarAction> {
    if !state.github_open {
        return None;
    }
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(DialogsToolbarAction::CloseGitHubModal);
    }

    egui::Area::new(egui::Id::new("github_modal_scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ctx.content_rect();
            let (_, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0_f32, Color32::from_black_alpha(150));
            if resp.clicked() {
                out = Some(DialogsToolbarAction::CloseGitHubModal);
            }
        });

    egui::Window::new("github_dialog_modal")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(760.0_f32, 540.0_f32))
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
                    RichText::new(format!("GitHub — {project_name}"))
                        .strong()
                        .size(14.0_f32),
                );
                if tab_chip(
                    ui,
                    &format!("Pull Requests ({})", state.github_prs.len()),
                    state.github_tab == GitHubTab::PullRequests,
                ) {
                    state.github_tab = GitHubTab::PullRequests;
                }
                if tab_chip(
                    ui,
                    &format!("Issues ({})", state.github_issues.len()),
                    state.github_tab == GitHubTab::Issues,
                ) {
                    state.github_tab = GitHubTab::Issues;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        out = Some(DialogsToolbarAction::CloseGitHubModal);
                    }
                    if ui
                        .button(if french { "Actualiser" } else { "Refresh" })
                        .clicked()
                    {
                        out = Some(DialogsToolbarAction::RefreshGitHub);
                    }
                });
            });

            ui.add_space(4.0_f32);
            ui.add(
                egui::TextEdit::singleline(&mut state.github_query)
                    .desired_width(f32::INFINITY)
                    .hint_text(if french {
                        "Filtrer par titre, branche ou numéro…"
                    } else {
                        "Filter by title, branch or #number…"
                    }),
            );
            ui.separator();

            if !state.github_error.is_empty() {
                ui.label(RichText::new(&state.github_error).color(pal.danger));
            }

            let q = state.github_query.trim().to_ascii_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("github_modal_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| match state.github_tab {
                    GitHubTab::PullRequests => {
                        for pr in &state.github_prs {
                            if !q.is_empty()
                                && !pr.title.to_ascii_lowercase().contains(&q)
                                && !pr.head_ref.to_ascii_lowercase().contains(&q)
                                && !pr.number.to_string().contains(&q)
                            {
                                continue;
                            }
                            let (cr, cg, cb) = pr.rollup.rgb();
                            let check_col = Color32::from_rgb(cr, cg, cb);
                            egui::Frame::NONE
                                .fill(pal.surface_2)
                                .stroke(Stroke::new(1.0_f32, pal.border))
                                .corner_radius(6.0_f32)
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            RichText::new(format!("#{} {}", pr.number, pr.title))
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(pr.rollup.label(french))
                                                .size(11.0_f32)
                                                .color(check_col),
                                        );
                                    });
                                    ui.label(
                                        RichText::new(format!("{} ← {}", pr.base_ref, pr.head_ref))
                                            .monospace()
                                            .size(11.5_f32)
                                            .color(pal.text_dim),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .small_button(if french {
                                                "Ouvrir dans un worktree"
                                            } else {
                                                "Start worktree"
                                            })
                                            .clicked()
                                        {
                                            out =
                                                Some(DialogsToolbarAction::StartWorktreeFromSlug(
                                                    format!("pr-{}", pr.number),
                                                ));
                                        }
                                        if !pr.failing_checks.is_empty()
                                            && ui
                                                .small_button(if french {
                                                    "Corriger les checks en échec"
                                                } else {
                                                    "Fix failing checks"
                                                })
                                                .clicked()
                                        {
                                            out = Some(
                                                DialogsToolbarAction::SendPromptToActiveAgent(
                                                    format_fix_failing_checks_prompt(pr),
                                                ),
                                            );
                                        }
                                        if !pr.url.is_empty()
                                            && ui
                                                .small_button(if french {
                                                    "Ouvrir sur GitHub"
                                                } else {
                                                    "Open on GitHub"
                                                })
                                                .clicked()
                                        {
                                            out = Some(DialogsToolbarAction::OpenExternalUrl(
                                                pr.url.clone(),
                                            ));
                                        }
                                    });
                                });
                            ui.add_space(4.0_f32);
                        }
                    }
                    GitHubTab::Issues => {
                        for iss in &state.github_issues {
                            if !q.is_empty()
                                && !iss.title.to_ascii_lowercase().contains(&q)
                                && !iss.number.to_string().contains(&q)
                            {
                                continue;
                            }
                            egui::Frame::NONE
                                .fill(pal.surface_2)
                                .stroke(Stroke::new(1.0_f32, pal.border))
                                .corner_radius(6.0_f32)
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("#{} {}", iss.number, iss.title))
                                            .strong(),
                                    );
                                    ui.horizontal(|ui| {
                                        if ui
                                            .small_button(if french {
                                                "Démarrer un worktree"
                                            } else {
                                                "Start worktree"
                                            })
                                            .clicked()
                                        {
                                            out =
                                                Some(DialogsToolbarAction::StartWorktreeFromSlug(
                                                    format!("issue-{}", iss.number),
                                                ));
                                        }
                                        if !iss.url.is_empty()
                                            && ui
                                                .small_button(if french {
                                                    "Ouvrir sur GitHub"
                                                } else {
                                                    "Open on GitHub"
                                                })
                                                .clicked()
                                        {
                                            out = Some(DialogsToolbarAction::OpenExternalUrl(
                                                iss.url.clone(),
                                            ));
                                        }
                                    });
                                });
                            ui.add_space(4.0_f32);
                        }
                    }
                });
        });

    out
}

// ============================================================================
// 3. Project Group Modal (Sidebar Folder Grouping)
// ============================================================================

pub fn show_project_group_modal(
    ctx: &egui::Context,
    state: &mut DialogsToolbarUiState,
    existing_groups: &[String],
    french: bool,
) -> Option<DialogsToolbarAction> {
    let project_id = state.group_modal_project?;
    let mut out = None;
    let pal = &theme::active(ctx).spec().ui;

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return Some(DialogsToolbarAction::CloseProjectGroupModal);
    }

    egui::Window::new("project_group_modal")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(380.0_f32, 200.0_f32))
        .frame(
            egui::Frame::NONE
                .fill(pal.surface)
                .stroke(Stroke::new(1.0_f32, pal.border_strong))
                .corner_radius(8.0_f32)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.label(
                RichText::new(if french {
                    "Groupe de la barre latérale"
                } else {
                    "Sidebar Workspace Group"
                })
                .strong()
                .size(13.5_f32),
            );
            ui.add_space(6.0_f32);
            ui.add(
                egui::TextEdit::singleline(&mut state.group_modal_draft)
                    .desired_width(f32::INFINITY)
                    .hint_text(if french {
                        "Ex. : Clients, Backend, Personnel…"
                    } else {
                        "e.g. Clients, Backend, Personal…"
                    }),
            );
            if !existing_groups.is_empty() {
                ui.add_space(4.0_f32);
                ui.horizontal_wrapped(|ui| {
                    for g in existing_groups {
                        if tab_chip(ui, g, state.group_modal_draft == *g) {
                            state.group_modal_draft = g.clone();
                        }
                    }
                });
            }
            ui.add_space(10.0_f32);
            ui.horizontal(|ui| {
                if ui
                    .button(if french { "Enregistrer" } else { "Save" })
                    .clicked()
                {
                    let trimmed = state.group_modal_draft.trim().to_owned();
                    out = Some(DialogsToolbarAction::SaveProjectGroup {
                        project_id,
                        group: if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed)
                        },
                    });
                }
                if ui
                    .button(if french { "Aucun groupe" } else { "Ungroup" })
                    .clicked()
                {
                    out = Some(DialogsToolbarAction::SaveProjectGroup {
                        project_id,
                        group: None,
                    });
                }
                if ui
                    .button(if french { "Annuler" } else { "Cancel" })
                    .clicked()
                {
                    out = Some(DialogsToolbarAction::CloseProjectGroupModal);
                }
            });
        });

    out
}

// ============================================================================
// 4. Toolbar UsageMenu (`360px`) & NotificationsMenu (`360px` + Agent Request Pill)
// ============================================================================

pub fn paint_usage_toolbar_button(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    level: UsageAlertLevel,
    french: bool,
    open: bool,
) -> Response {
    let tip = if french {
        "Utilisation et budget IA"
    } else {
        "AI usage & budget"
    };
    let resp = ui.add(
        egui::Button::new("")
            .min_size(Vec2::new(28.0_f32, 28.0_f32))
            .frame(false)
            .selected(open),
    );
    let col = match level {
        UsageAlertLevel::Danger => pal.danger,
        UsageAlertLevel::Warn => pal.warn,
        UsageAlertLevel::Normal => {
            if resp.hovered() || open {
                pal.text
            } else {
                pal.text_dim
            }
        }
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(resp.rect.center(), Vec2::splat(16.0_f32)),
        Icon::Gauge,
        col,
    );
    resp.on_hover_text(tip)
}

pub fn show_usage_menu_contents(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    summary: &UsageSummary,
    french: bool,
) -> Option<DialogsToolbarAction> {
    let mut out = None;
    ui.set_min_width(340.0_f32);
    ui.set_max_width(360.0_f32);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if french {
                "Utilisation des fournisseurs"
            } else {
                "Provider Usage"
            })
            .strong()
            .size(13.0_f32),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(if french {
                    "Statistiques…"
                } else {
                    "Stats…"
                })
                .clicked()
            {
                out = Some(DialogsToolbarAction::OpenStatsSettings);
            }
        });
    });

    ui.add_space(4.0_f32);
    ui.label(
        RichText::new(if french {
            format!(
                "Aujourd'hui : ${:.3} / ${:.2} ({:.0}%) · 30 j : ${:.3}",
                summary.today_cost_usd,
                summary.daily_budget_usd,
                summary.overall_ratio * 100.0_f32,
                summary.month_cost_usd
            )
        } else {
            format!(
                "Today: ${:.3} / ${:.2} ({:.0}%) · 30d: ${:.3}",
                summary.today_cost_usd,
                summary.daily_budget_usd,
                summary.overall_ratio * 100.0_f32,
                summary.month_cost_usd
            )
        })
        .size(11.5_f32)
        .color(pal.text_dim),
    );

    let bar_col = match summary.level {
        UsageAlertLevel::Danger => pal.danger,
        UsageAlertLevel::Warn => pal.warn,
        UsageAlertLevel::Normal => pal.accent,
    };
    let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(320.0_f32, 6.0_f32), Sense::hover());
    ui.painter().rect_filled(bar_rect, 3.0_f32, pal.surface_3);
    if summary.overall_ratio > 0.0_f32 {
        let fill_w = (bar_rect.width() * summary.overall_ratio).max(3.0_f32);
        ui.painter().rect_filled(
            Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height())),
            3.0_f32,
            bar_col,
        );
    }

    ui.separator();

    if summary.providers.is_empty() {
        ui.label(
            RichText::new(if french {
                "Aucune consommation enregistrée sur 30 jours."
            } else {
                "No recorded provider usage in the last 30 days."
            })
            .size(11.5_f32)
            .color(pal.text_dim),
        );
    } else {
        for p in &summary.providers {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&p.provider).strong().size(12.0_f32));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("${:.3} ({} tok)", p.cost_30d_usd, p.tokens_30d))
                            .size(11.0_f32)
                            .color(pal.text_dim),
                    );
                });
            });
        }
    }

    out
}

pub fn paint_notifications_toolbar_button(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    unread_count: usize,
    french: bool,
    open: bool,
) -> Response {
    let _ = french;
    let tip = "Notifications";
    let resp = ui.add(
        egui::Button::new("")
            .min_size(Vec2::new(28.0_f32, 28.0_f32))
            .frame(false)
            .selected(open),
    );
    let col = if unread_count > 0 {
        pal.accent
    } else if resp.hovered() || open {
        pal.text
    } else {
        pal.text_dim
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(resp.rect.center(), Vec2::splat(16.0_f32)),
        Icon::Bell,
        col,
    );
    if unread_count > 0 {
        let txt = if unread_count > 9 {
            "9+".to_owned()
        } else {
            unread_count.to_string()
        };
        let badge_rect = Rect::from_min_size(
            Pos2::new(resp.rect.right() - 13.0_f32, resp.rect.top() + 2.0_f32),
            Vec2::new(12.0_f32, 12.0_f32),
        );
        ui.painter().rect_filled(badge_rect, 6.0_f32, pal.accent);
        ui.painter().text(
            badge_rect.center(),
            Align2::CENTER_CENTER,
            txt,
            FontId::proportional(8.5_f32),
            Color32::WHITE,
        );
    }
    resp.on_hover_text(tip)
}

pub fn paint_agent_request_pill(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    waiting_count: usize,
    french: bool,
) -> Option<Response> {
    if waiting_count == 0 {
        return None;
    }
    let label = if french {
        format!("● Demande ({waiting_count})")
    } else {
        format!("● Agent request ({waiting_count})")
    };
    let resp = ui.add(
        egui::Button::new(RichText::new(label).size(11.0_f32).color(pal.warn))
            .fill(pal.warn.gamma_multiply(0.16_f32))
            .stroke(Stroke::new(1.0_f32, pal.warn))
            .corner_radius(10.0_f32),
    );
    Some(resp)
}

pub fn show_notifications_menu_contents(
    ui: &mut egui::Ui,
    pal: &UiPalette,
    center: &NotificationCenter,
    french: bool,
) -> Option<DialogsToolbarAction> {
    let mut out = None;
    ui.set_min_width(340.0_f32);
    ui.set_max_width(360.0_f32);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if french {
                "Centre de notifications"
            } else {
                "Notifications"
            })
            .strong()
            .size(13.0_f32),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(if french {
                    "Réglages…"
                } else {
                    "Settings…"
                })
                .clicked()
            {
                out = Some(DialogsToolbarAction::OpenAlertsSettings);
            }
            if !center.items.is_empty() {
                if ui
                    .small_button(if french { "Effacer" } else { "Clear" })
                    .clicked()
                {
                    out = Some(DialogsToolbarAction::ClearNotifications);
                }
                if center.unread_count() > 0
                    && ui
                        .small_button(if french {
                            "Tout marquer lu"
                        } else {
                            "Mark read"
                        })
                        .clicked()
                {
                    out = Some(DialogsToolbarAction::MarkAllNotificationsRead);
                }
            }
        });
    });

    ui.separator();

    if center.items.is_empty() {
        ui.label(
            RichText::new(if french {
                "Aucune notification récente."
            } else {
                "No recent notifications."
            })
            .size(11.5_f32)
            .color(pal.text_dim),
        );
        return out;
    }

    egui::ScrollArea::vertical()
        .id_salt("notifications_menu_scroll")
        .max_height(280.0_f32)
        .show(ui, |ui| {
            for item in center.items.iter().rev() {
                let border = if item.unread { pal.accent } else { pal.border };
                let resp = egui::Frame::NONE
                    .fill(pal.surface_2)
                    .stroke(Stroke::new(1.0_f32, border))
                    .corner_radius(6.0_f32)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        let title = if item.count > 1 {
                            format!("{} (×{})", item.title, item.count)
                        } else {
                            item.title.clone()
                        };
                        ui.label(RichText::new(title).strong().size(12.0_f32));
                        if !item.detail.is_empty() {
                            ui.label(
                                RichText::new(&item.detail)
                                    .size(11.0_f32)
                                    .color(pal.text_dim),
                            );
                        }
                    })
                    .response
                    .interact(Sense::click());
                if resp.clicked()
                    && let Some(pane_id) = item.pane_id
                {
                    out = Some(DialogsToolbarAction::FocusPane(pane_id));
                }
                ui.add_space(4.0_f32);
            }
        });

    out
}
