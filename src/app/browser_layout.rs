use super::{Tessel, View};
use crate::browser::Browser;
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};

const FILE: &str = "owner-browser-tabs.json";
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESTORED_SIDE_PAGES: usize = 50;

#[derive(Serialize, Deserialize)]
struct SavedPage(String, String, #[serde(default)] Option<i8>);

#[derive(Serialize, Deserialize)]
struct Group {
    project: u64,
    root: PathBuf,
    selected: usize,
    pages: Vec<SavedPage>,
}

#[derive(Serialize, Deserialize)]
struct Layout {
    #[serde(default)]
    side_pages: Vec<SavedPage>,
    #[serde(default)]
    selected_side: Option<usize>,
    groups: Vec<Group>,
    visible_project: Option<u64>,
}

impl Tessel {
    fn owner_browser_layout(&self) -> Layout {
        let mut groups = Vec::new();
        for project in &self.state.projects {
            let owner = Some(project.id);
            let current = if owner == self.browser_project {
                (self.browser_tab_open || self.view == View::Browser).then_some(&self.browser)
            } else {
                self.background_browsers
                    .get(&owner)
                    .and_then(|(page, open)| open.then_some(page))
            };
            let Some(current) = current else { continue };
            let mut pages: Vec<_> = self
                .other_browser_pages
                .range((owner, 0)..=(owner, u64::MAX))
                .map(|(_, page)| page)
                .collect();
            pages.push(current);
            pages.sort_unstable_by_key(|page| page.tab_order);
            let selected = pages
                .iter()
                .position(|page| page.tab_id == current.tab_id)
                .unwrap_or_default();
            groups.push(Group {
                project: project.id,
                root: project.path.clone(),
                selected,
                pages: pages
                    .into_iter()
                    .map(|page| {
                        let (url, title) = page.restart_location();
                        SavedPage(url, title, page.zoom_override)
                    })
                    .collect(),
            });
        }
        Layout {
            side_pages: self
                .side_browsers
                .values()
                .map(|page| {
                    let (url, title) = page.restart_location();
                    SavedPage(url, title, page.zoom_override)
                })
                .collect(),
            selected_side: self
                .side_browsers
                .keys()
                .position(|id| Some(*id) == self.selected_side_browser),
            groups,
            visible_project: (self.view == View::Browser)
                .then_some(self.browser_project)
                .flatten(),
        }
    }

    pub(super) fn save_owner_browser_layout(&self) -> Result<(), String> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let path = store.directory().join(FILE);
        let layout = self.owner_browser_layout();
        if (layout.groups.is_empty() && layout.side_pages.is_empty())
            || !self.state.general_prefs.restore_workspaces
        {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let bytes = serde_json::to_vec(&layout).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Browser layout is too large to save.".into());
        }
        crate::workspace::atomic_write(&path, &bytes)
    }

    pub(super) fn restore_owner_browser_layout(&mut self) {
        let Some(store) = &self.store else { return };
        let Ok(file) = std::fs::File::open(store.directory().join(FILE)) else {
            return;
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > MAX_BYTES
        {
            return;
        }
        let Ok(layout) = serde_json::from_slice::<Layout>(&bytes) else {
            return;
        };
        for (index, SavedPage(url, title, zoom)) in layout
            .side_pages
            .into_iter()
            .take(MAX_RESTORED_SIDE_PAGES)
            .enumerate()
        {
            let mut page = Browser::from_restart_location(&url, &title);
            page.zoom_override = zoom.map(|steps| steps.clamp(-6, 10));
            if layout.selected_side == Some(index) {
                self.selected_side_browser = Some(page.tab_id);
            }
            self.side_browsers.insert(page.tab_id, page);
        }
        for group in layout.groups {
            let owner = Some(group.project);
            if !self
                .state
                .projects
                .iter()
                .any(|p| p.id == group.project && p.path == group.root)
                || group.selected >= group.pages.len()
            {
                continue;
            }
            for (index, SavedPage(url, title, zoom)) in group.pages.into_iter().enumerate() {
                let mut page = Browser::from_restart_location(&url, &title);
                page.zoom_override = zoom.map(|steps| steps.clamp(-6, 10));
                if index != group.selected {
                    self.other_browser_pages.insert((owner, page.tab_id), page);
                } else if owner == self.browser_project {
                    self.replace_browser_page(page);
                    self.browser_tab_open = true;
                    if layout.visible_project == owner {
                        self.view = View::Browser;
                    }
                } else {
                    self.background_browsers.insert(owner, (page, true));
                }
            }
        }
    }
}

impl Drop for Tessel {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = self.save_owner_browser_layout();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::gpui_bridge::GpuiAction;

    #[test]
    fn closing_side_pages_restores_last_visit_and_skips_closed_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().to_owned()));
        let ids = [0; 4].map(|_| {
            app.new_browser_page();
            app.dock_browser(app.browser.tab_id);
            app.selected_side_browser.unwrap()
        });
        app.apply_gpui_action(GpuiAction::SelectSideBrowser(ids[0]));
        app.apply_gpui_action(GpuiAction::CloseSideBrowser(ids[2]));
        assert_eq!(app.selected_side_browser, Some(ids[0]));
        app.apply_gpui_action(GpuiAction::CloseSideBrowser(ids[0]));
        assert_eq!(app.selected_side_browser, Some(ids[3]));
        app.apply_gpui_action(GpuiAction::CloseSideBrowser(ids[3]));
        assert_eq!(app.selected_side_browser, Some(ids[1]));
        app.apply_gpui_action(GpuiAction::CloseSideBrowser(ids[1]));
        assert!(app.selected_side_browser.is_none());
        assert!(app.side_browsers.is_empty());
        assert!(app.state.panels.right_open);
        app.new_browser_page();
        app.dock_browser(app.browser.tab_id);
        let side = app.selected_side_browser.unwrap();
        app.execute_palette_command(&eframe::egui::Context::default(), "files");
        assert!(app.right_tool_visible(crate::workspace::SideTab::Files));
        app.select_side_browser(side);
        assert!(app.gpui_explorer_input().is_none());
        app.close_side_browser(side);
        assert!(app.right_tool_visible(crate::workspace::SideTab::Files));
    }

    #[test]
    fn restart_restores_only_first_fifty_side_pages_and_skips_missing_selection() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let layout = Layout {
            side_pages: (0..51)
                .map(|i| SavedPage(format!("https://page.example/{i}"), i.to_string(), None))
                .collect(),
            selected_side: Some(50),
            groups: Vec::new(),
            visible_project: None,
        };
        std::fs::write(state.join(FILE), serde_json::to_vec(&layout).unwrap()).unwrap();
        let app = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(app.side_browsers.len(), 50);
        assert!(app.selected_side_browser.is_none());
        assert_eq!(
            app.side_browsers.values().last().unwrap().address_str(),
            "https://page.example/49"
        );
    }

    #[test]
    fn side_only_layout_restores_selection_without_creating_center_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let start = || Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.new_browser_page();
        app.browser.open_url("https://side.example/");
        app.dock_browser(app.browser.tab_id);
        assert!(app.browser_labels().is_empty());
        drop(app);
        let mut app = start();
        let side = app.selected_side_browser.unwrap();
        assert_eq!(
            app.side_browsers[&side].address_str(),
            "https://side.example/"
        );
        assert!(app.browser_labels().is_empty());
        app.undock_browser(side);
        assert!(app.side_browsers.is_empty());
        assert_eq!(app.browser.address_str(), "https://side.example/");
        assert!(app.view == View::Browser);
    }

    #[test]
    fn side_drop_order_and_selected_neighbor_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let start = || Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().to_owned()));
        let initial = ["a", "b", "c"].map(|name| {
            app.new_browser_page();
            app.browser
                .open_url(&format!("https://fixture.example/{name}"));
            app.browser.tab_id
        });
        let names = |app: &Tessel| {
            app.browser_labels()
                .into_iter()
                .map(|(_, _, url)| url.rsplit('/').next().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        for (name, anchor, before) in [
            ("d", Some(initial[1]), true),
            ("e", Some(initial[0]), false),
            ("f", None, true),
            ("g", None, false),
        ] {
            app.new_side_browser();
            let id = app.selected_side_browser.unwrap();
            app.side_browsers
                .get_mut(&id)
                .unwrap()
                .open_url(&format!("https://fixture.example/{name}"));
            app.undock_browser_at(id, Some(u64::MAX), before);
            assert!(app.side_browsers.contains_key(&id));
            app.apply_gpui_action(GpuiAction::UndockBrowserAt {
                page: id,
                anchor,
                before,
            });
        }
        assert_eq!(names(&app), ["f", "a", "e", "d", "b", "c", "g"]);
        app.select_browser_page(initial[0]);
        app.close_browser_page(initial[0]);
        assert_eq!(app.browser.address_str(), "https://fixture.example/e");
        app.save();
        drop(app);
        let app = start();
        assert_eq!(names(&app), ["f", "e", "d", "b", "c", "g"]);
        assert_eq!(app.browser.address_str(), "https://fixture.example/e");
    }

    #[test]
    fn restart_restores_project_pages_order_selection_and_privacy() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let start = || Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        let mut app = start();
        let mut ids = Vec::new();
        for (index, name) in ["one", "two"].iter().enumerate() {
            let folder = dir.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(folder));
            ids.push(app.state.active_project.unwrap());
            app.new_browser_page();
            app.browser.open_url(&format!("https://{name}.example/"));
            app.execute_palette_command(&eframe::egui::Context::default(), "browser:zoom-in");
            let first = app.browser.tab_id;
            app.new_browser_page();
            app.browser
                .open_url("blob:https://two.example/session-only");
            if index == 1 {
                app.select_browser_page(first);
            }
        }
        app.save();
        drop(app);
        let bytes = std::fs::read_to_string(state.join(FILE)).unwrap();
        assert!(!bytes.contains("session-only"));
        assert!(
            !std::fs::read_to_string(state.join("workspace.json"))
                .unwrap()
                .contains("two.example")
        );
        let mut app = start();
        assert!(app.view == View::Browser);
        assert!(!app.browser.is_started());
        assert_eq!(app.browser.zoom_override, Some(0));
        let legacy: SavedPage =
            serde_json::from_str(r#"["https://example.org/", "Before zoom"]"#).unwrap();
        assert_eq!(legacy.2, None);
        assert_eq!(app.browser.address_str(), "https://two.example/");
        assert_eq!(app.browser_labels().len(), 2);
        app.apply_gpui_action(GpuiAction::SelectProject(ids[0]));
        assert_eq!(app.browser.address_str(), "about:blank");
        let tabs = app.browser_labels();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].2, "https://one.example/");
        app.state.general_prefs.restore_workspaces = false;
        app.save();
        drop(app);
        assert!(!state.join(FILE).exists());
        assert!(start().browser_labels().is_empty());
    }
}
