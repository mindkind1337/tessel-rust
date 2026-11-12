//! Reuse grouped conversations until their sources, filters, or time window change.
use crate::side_panels_model::{
    ActivityMessage, ActivityStore, ConversationMediaKind, ConversationThread, ConversationWindow,
    build_conversation_threads,
};
use std::sync::Arc;

#[derive(PartialEq, Eq)]
struct Key {
    activity_id: u64,
    activity_len: usize,
    preview_revision: u64,
    window: ConversationWindow,
    media: Option<ConversationMediaKind>,
    query: String,
}

#[derive(Default)]
pub(crate) struct Model {
    key: Option<Key>,
    threads: Arc<[ConversationThread]>,
    expires_at: Option<u64>,
    last_now: u64,
}

impl Model {
    pub(crate) fn needs_frame(&self, now: u64) -> bool {
        self.key.is_some() && (now < self.last_now || self.expires_at.is_some_and(|end| now >= end))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        activity: &ActivityStore,
        preview: &[ActivityMessage],
        preview_revision: u64,
        window: ConversationWindow,
        media: Option<ConversationMediaKind>,
        query: &str,
        now: u64,
    ) -> Arc<[ConversationThread]> {
        let unchanged = self.key.as_ref().is_some_and(|key| {
            key.activity_id == activity.next_id
                && key.activity_len == activity.messages.len()
                && key.preview_revision == preview_revision
                && key.window == window
                && key.media == media
                && key.query == query
        });
        if unchanged && now >= self.last_now && self.expires_at.is_none_or(|end| now < end) {
            self.last_now = now;
            return self.threads.clone();
        }
        let messages: Vec<_> = activity.messages.iter().chain(preview).cloned().collect();
        self.expires_at = messages
            .iter()
            .filter_map(|message| {
                message
                    .timestamp_ms
                    .checked_add(window.window_ms())?
                    .checked_add(1)
            })
            .filter(|end| *end > now)
            .min();
        self.threads = build_conversation_threads(&messages, window, media, query, now).into();
        self.key = Some(Key {
            activity_id: activity.next_id,
            activity_len: activity.messages.len(),
            preview_revision,
            window,
            media,
            query: query.to_owned(),
        });
        self.last_now = now;
        self.threads.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_frames_reuse_threads_until_actual_expiry() {
        let mut activity = ActivityStore::default();
        activity.push(ActivityMessage {
            timestamp_ms: 100,
            body: "synthetic".into(),
            thread_key: "one".into(),
            ..Default::default()
        });
        let mut model = Model::default();
        let first = model.update(
            &activity,
            &[],
            0,
            ConversationWindow::Hours24,
            None,
            "",
            100,
        );
        let again = model.update(
            &activity,
            &[],
            0,
            ConversationWindow::Hours24,
            None,
            "",
            200,
        );
        assert!(Arc::ptr_eq(&first, &again));
        let boundary = 100 + ConversationWindow::Hours24.window_ms();
        assert!(!model.needs_frame(boundary));
        assert!(model.needs_frame(boundary + 1));
        assert_eq!(
            model
                .update(
                    &activity,
                    &[],
                    0,
                    ConversationWindow::Hours24,
                    None,
                    "",
                    boundary
                )
                .len(),
            1
        );
        assert!(
            model
                .update(
                    &activity,
                    &[],
                    0,
                    ConversationWindow::Hours24,
                    None,
                    "",
                    boundary + 1
                )
                .is_empty()
        );
        assert_eq!(
            model
                .update(
                    &activity,
                    &[],
                    0,
                    ConversationWindow::Hours24,
                    None,
                    "",
                    100
                )
                .len(),
            1
        );
    }
    #[test]
    fn source_revision_and_filter_changes_invalidate_model() {
        let activity = ActivityStore::default();
        let preview = [ActivityMessage {
            timestamp_ms: 100,
            body: "first".into(),
            thread_key: "one".into(),
            ..Default::default()
        }];
        let mut model = Model::default();
        let first = model.update(
            &activity,
            &preview,
            1,
            ConversationWindow::Hours24,
            None,
            "",
            100,
        );
        assert_eq!(first.len(), 1);
        assert!(
            model
                .update(
                    &activity,
                    &preview,
                    1,
                    ConversationWindow::Hours24,
                    None,
                    "absent",
                    100
                )
                .is_empty()
        );
        assert!(
            model
                .update(
                    &activity,
                    &[],
                    2,
                    ConversationWindow::Hours24,
                    None,
                    "",
                    100
                )
                .is_empty()
        );
    }
}
