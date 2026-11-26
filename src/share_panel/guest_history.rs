use crate::guest_file_editor::Key;
use crate::share::{GuestRpcError, GuestSession, guest_text};
use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
pub(crate) struct Commit {
    pub(crate) id: String,
    pub(crate) time: i64,
    pub(crate) author: String,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    rel: String,
    pub(crate) items: Vec<Commit>,
    pub(crate) uncommitted: bool,
    pub(crate) has_more: bool,
}

struct Pending {
    reply: Receiver<Result<Value, GuestRpcError>>,
    deadline: Instant,
    limit: usize,
}

#[derive(Default)]
pub(super) struct Timeline {
    key: Option<Key>,
    pub(super) page: Option<std::sync::Arc<Page>>,
    pending: Option<Pending>,
    limit: usize,
    pub(super) error: Option<String>,
}

impl Timeline {
    pub(super) fn bind(&mut self, key: Option<&Key>) -> bool {
        if self.key.as_ref() == key {
            return false;
        }
        *self = Self {
            key: key.cloned(),
            limit: 50,
            ..Default::default()
        };
        true
    }

    pub(super) fn next_limit(&self, more: bool) -> usize {
        if more {
            (self.limit + 50).min(200)
        } else {
            self.limit.max(50)
        }
    }
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn request(&mut self, session: &GuestSession, limit: usize) {
        let Some(key) = &self.key else { return };
        if self.busy() || key.scope.session != session.generation() {
            return;
        }
        let payload = json!({"op":"scm.fileHistory","projectId":key.scope.project,"path":key.path,"limit":limit});
        self.error = None;
        match session.request(payload) {
            Ok(reply) => {
                self.pending = Some(Pending {
                    reply,
                    limit,
                    deadline: Instant::now() + Duration::from_secs(25),
                })
            }
            Err(_) => self.error = Some(String::new()),
        }
    }

    pub(super) fn poll(&mut self, now: Instant) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.reply.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) if now < pending.deadline => return false,
            Err(_) => None,
        };
        let limit = pending.limit;
        self.pending = None;
        let page = match result {
            Some(Ok(value)) => serde_json::from_value::<Page>(value).ok(),
            Some(Err(GuestRpcError::Host(error))) => {
                self.error = Some(guest_text::task_title(&error, "", 200));
                None
            }
            _ => None,
        };
        if let Some(mut page) = page.filter(|page| {
            self.key.as_ref().is_some_and(|k| k.path == page.rel)
                && page.items.len() <= limit
                && page.items.iter().all(|row| {
                    matches!(row.id.len(), 40 | 64)
                        && row.id.bytes().all(|b| b.is_ascii_hexdigit())
                        && row.author.len() <= 800
                        && row.message.len() <= 80000
                })
        }) {
            for row in &mut page.items {
                row.author = guest_text::task_title(&row.author, "", 200);
                row.message = guest_text::guest_text(&row.message, &[], 4096);
            }
            self.limit = limit;
            self.page = Some(std::sync::Arc::new(page));
        } else {
            self.page = None;
            self.error.get_or_insert_with(String::new);
        }
        true
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, fr: bool, session: &GuestSession) {
        egui::CollapsingHeader::new(if fr { "Chronologie" } else { "Timeline" })
            .id_salt("guest-file-timeline")
            .show(ui, |ui| {
                if self.page.is_none() && !self.busy() && self.error.is_none() {
                    self.request(session, 50);
                }
                let mut request = None;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.busy(),
                            egui::Button::new(if fr { "Actualiser" } else { "Refresh" }),
                        )
                        .clicked()
                    {
                        request = Some(self.limit);
                    }
                    if self.busy() {
                        ui.spinner();
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                    }
                });
                if let Some(error) = &self.error {
                    ui.label(if error.is_empty() {
                        if fr {
                            "Historique indisponible."
                        } else {
                            "History unavailable."
                        }
                    } else {
                        error
                    });
                }
                if let Some(page) = &self.page {
                    if page.uncommitted {
                        ui.label(if fr {
                            "Modifications non commitées"
                        } else {
                            "Uncommitted Changes"
                        });
                    }
                    if page.items.is_empty() && !page.uncommitted {
                        ui.label(if fr {
                            "Aucun historique."
                        } else {
                            "No history."
                        });
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("guest-history-list")
                        .max_height(220.)
                        .show(ui, |ui| {
                            for row in &page.items {
                                let date = chrono::DateTime::from_timestamp_millis(row.time)
                                    .map(|dt| {
                                        dt.with_timezone(&chrono::Local)
                                            .format("%Y-%m-%d %H:%M")
                                            .to_string()
                                    })
                                    .unwrap_or_default();
                                ui.label(row.message.lines().next().unwrap_or(""))
                                    .on_hover_ui(|ui| {
                                        ui.strong(&row.author);
                                        ui.label(date);
                                        ui.label(&row.message);
                                        ui.monospace(&row.id);
                                    });
                            }
                        });
                    if page.has_more
                        && self.limit < 200
                        && ui
                            .add_enabled(
                                !self.busy(),
                                egui::Button::new(if fr {
                                    "Charger la suite"
                                } else {
                                    "Load More"
                                }),
                            )
                            .clicked()
                    {
                        request = Some((self.limit + 50).min(200));
                    }
                }
                if let Some(n) = request {
                    self.request(session, n);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_task_board::Scope;
    #[test]
    fn guest_timeline_drops_late_replies_on_key_change() {
        let original = Key {
            scope: Scope {
                session: 1,
                project: 1,
            },
            path: "a.txt".into(),
        };
        for other in [
            None,
            Some(Key {
                path: "b.txt".into(),
                ..original.clone()
            }),
            Some(Key {
                scope: Scope {
                    session: 2,
                    project: 1,
                },
                ..original.clone()
            }),
            Some(Key {
                scope: Scope {
                    session: 1,
                    project: 2,
                },
                ..original.clone()
            }),
        ] {
            let mut timeline = Timeline::default();
            timeline.bind(Some(&original));
            let (tx, reply) = std::sync::mpsc::channel();
            timeline.pending = Some(Pending {
                reply,
                limit: 50,
                deadline: Instant::now() + Duration::from_secs(25),
            });
            assert!(timeline.bind(other.as_ref()));
            assert!(tx.send(Ok(Value::Null)).is_err());
            assert!(!timeline.poll(Instant::now()));
            assert!(timeline.page.is_none());
        }
    }
}
