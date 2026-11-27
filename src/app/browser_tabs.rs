use super::{Tessel, View};
use crate::browser::{Browser, editor_page::EditorPage};

impl Tessel {
    pub(super) fn browser_labels(&self) -> Vec<(u64, String, String)> {
        let mut pages: Vec<_> = self
            .other_browser_pages
            .iter()
            .filter(|((owner, _), _)| *owner == self.state.active_project)
            .map(|(_, page)| page)
            .collect();
        if self.browser_tab_open || self.view == View::Browser {
            pages.push(&self.browser);
        }
        pages.sort_unstable_by_key(|page| page.tab_id);
        pages
            .into_iter()
            .map(|page| {
                let (title, hint) = page.tab_label();
                (page.tab_id, title, hint)
            })
            .collect()
    }

    pub(super) fn replace_browser_page(&mut self, next: Browser) -> Browser {
        self.browser.visible(false);
        self.browser.close_popups();
        self.browser.cancel_native_address_edit();
        let previous = std::mem::replace(&mut self.browser, next);
        self.browser
            .set_default_zoom(self.state.browser_prefs.zoom_half_steps);
        self.browser.invalidate_address_input();
        self.editor_page = EditorPage::new(&self.browser);
        self.show_agent_browser = false;
        previous
    }

    pub(super) fn new_browser_page(&mut self) {
        let previous = self.replace_browser_page(Browser::default());
        if self.browser_tab_open || self.view == View::Browser {
            self.other_browser_pages
                .insert((self.state.active_project, previous.tab_id), previous);
        }
        self.browser.open_url("about:blank");
        self.browser_tab_open = true;
        self.view = View::Browser;
    }

    pub(super) fn browser_speaker(&self, id: u64) -> Option<bool> {
        if id == self.browser.tab_id {
            self.browser.speaker_state()
        } else {
            self.other_browser_pages
                .get(&(self.state.active_project, id))?
                .speaker_state()
        }
    }

    pub(super) fn toggle_browser_sound(&mut self, id: u64) {
        if id == self.browser.tab_id {
            self.browser.toggle_mute();
        } else if let Some(page) = self
            .other_browser_pages
            .get_mut(&(self.state.active_project, id))
        {
            page.toggle_mute();
        }
    }

    pub(super) fn select_browser_page(&mut self, id: u64) {
        if id != self.browser.tab_id {
            let Some(next) = self
                .other_browser_pages
                .remove(&(self.state.active_project, id))
            else {
                return;
            };
            let previous = self.replace_browser_page(next);
            self.other_browser_pages
                .insert((self.state.active_project, previous.tab_id), previous);
        }
        self.browser_tab_open = true;
        self.view = View::Browser;
    }

    fn take_center_browser(&mut self, id: u64) -> Option<Browser> {
        if id == self.browser.tab_id && !self.browser_tab_open && self.view != View::Browser {
            return None;
        }
        let page = if id == self.browser.tab_id {
            let owner = self.state.active_project;
            let key = self
                .other_browser_pages
                .range((owner, id)..=(owner, u64::MAX))
                .next()
                .or_else(|| {
                    self.other_browser_pages
                        .range((owner, 0)..=(owner, id))
                        .next_back()
                })
                .map(|(key, _)| *key);
            let next = key.and_then(|key| self.other_browser_pages.remove(&key));
            self.browser_tab_open = next.is_some();
            if !self.browser_tab_open && self.view == View::Browser {
                self.view = View::Files;
                self.reconcile_editor_selection();
            }
            let next = next.unwrap_or_else(|| {
                let mut dormant = Browser::default();
                dormant.set_address_str(self.browser.address_str().to_owned());
                dormant
            });
            self.replace_browser_page(next)
        } else {
            self.other_browser_pages
                .remove(&(self.state.active_project, id))?
        };
        Some(page)
    }

    pub(super) fn dock_browser(&mut self, id: u64) {
        if self.gpui_has_open_modal() {
            return;
        }
        if let Some(mut page) = self.take_center_browser(id) {
            page.change_presentation();
            let id = page.tab_id;
            self.side_browsers.insert(id, page);
            self.selected_side_browser = Some(id);
            self.state.panels.right_open = true;
            self.save();
        }
    }

    pub(super) fn undock_browser(&mut self, id: u64) {
        if self.state.active_project.is_none()
            || self.view == View::GuestFiles
            || self.gpui_has_open_modal()
        {
            return;
        }
        if let Some(mut page) = self.side_browsers.remove(&id) {
            if self.selected_side_browser == Some(id) {
                self.selected_side_browser = None;
            }
            page.change_presentation();
            let was_open = self.browser_tab_open || self.view == View::Browser;
            let previous = self.replace_browser_page(page);
            if was_open {
                self.other_browser_pages
                    .insert((self.state.active_project, previous.tab_id), previous);
            }
            self.browser_tab_open = true;
            self.right_fullscreen = false;
            self.view = View::Browser;
            self.save();
        }
    }

    pub(super) fn close_browser_page(&mut self, id: u64) {
        let Some(mut closed) = self.take_center_browser(id) else {
            return;
        };
        closed.poll();
        if closed.downloading() {
            self.finishing_browser_pages
                .push((self.state.active_project, closed));
        }
    }
}
