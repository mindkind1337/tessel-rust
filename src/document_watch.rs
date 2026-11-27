//! Observe open local text documents without doing filesystem I/O on the UI thread.
use crate::files::{Document, confined};
use std::{
    collections::HashMap,
    fs, io,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, PartialEq, Eq)]
struct Target {
    id: u64,
    root: PathBuf,
    path: PathBuf,
    revision: u64,
}

#[derive(PartialEq, Eq)]
enum Stamp {
    Missing,
    Unsupported,
    Present(u64, SystemTime, Option<SystemTime>),
}

fn stamp(target: &Target) -> io::Result<Stamp> {
    match fs::symlink_metadata(&target.path) {
        Ok(meta) if !meta.is_file() || meta.len() > crate::files::MAX_TEXT as u64 => {
            Ok(Stamp::Unsupported)
        }
        Ok(meta) => Ok(Stamp::Present(
            meta.len(),
            meta.modified()?,
            meta.created().ok(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Stamp::Missing),
        Err(error) => Err(error),
    }
}

enum Event {
    Disk(Target, Option<String>),
    Finished,
}

struct Worker {
    requests: Sender<Vec<Target>>,
    events: Receiver<Event>,
}
impl Worker {
    fn start() -> io::Result<Self> {
        let (requests, batches) = mpsc::channel::<Vec<Target>>();
        let (results, events) = mpsc::sync_channel(2);
        std::thread::Builder::new()
            .name("open-document-watch".into())
            .spawn(move || {
                let mut previous = HashMap::<u64, (Target, Stamp)>::new();
                for batch in batches {
                    let mut next = HashMap::with_capacity(batch.len());
                    for target in batch {
                        let Ok(current) = stamp(&target) else {
                            continue;
                        };
                        if previous
                            .get(&target.id)
                            .is_none_or(|(old, sig)| old != &target || sig != &current)
                        {
                            match &current {
                                Stamp::Missing => {
                                    if results.send(Event::Disk(target.clone(), None)).is_err() {
                                        return;
                                    }
                                }
                                Stamp::Present(..) => {
                                    if confined(&target.root, &target.path).ok().as_ref()
                                        != Some(&target.path)
                                    {
                                        continue;
                                    }
                                    let Ok(bytes) = crate::files::read(&target.path) else {
                                        continue;
                                    };
                                    if stamp(&target).ok().as_ref() != Some(&current) {
                                        continue;
                                    }
                                    if !bytes.contains(&0)
                                        && let Ok(text) = String::from_utf8(bytes)
                                        && results
                                            .send(Event::Disk(target.clone(), Some(text)))
                                            .is_err()
                                    {
                                        return;
                                    }
                                }
                                Stamp::Unsupported => {}
                            }
                        }
                        next.insert(target.id, (target, current));
                    }
                    previous = next;
                    if results.send(Event::Finished).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self { requests, events })
    }
}

#[derive(Default)]
pub(crate) struct DocumentWatch {
    worker: Option<Worker>,
    pending: bool,
    next: Option<Instant>,
}

impl DocumentWatch {
    pub(crate) fn poll(&mut self, docs: &mut [Document], now: Instant) -> bool {
        let mut changed = false;
        if let Some(worker) = &self.worker {
            loop {
                match worker.events.try_recv() {
                    Ok(Event::Disk(target, text)) => {
                        if let Some(doc) = docs.iter_mut().find(|d| {
                            d.instance_id() == target.id
                                && d.path == target.path
                                && d.root == target.root
                                && d.tools.disk_revision == target.revision
                        }) {
                            changed |= doc.observe_disk(text);
                        }
                    }
                    Ok(Event::Finished) => self.pending = false,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.worker = None;
                        self.pending = false;
                        break;
                    }
                }
            }
        }
        if self.pending || self.next.is_some_and(|next| next > now) {
            return changed;
        }
        self.next = Some(now + Duration::from_secs(1));
        let targets: Vec<_> = docs
            .iter()
            .filter(|d| {
                !d.media && d.diff_title.is_none() && d.path.is_absolute() && d.root.is_absolute()
            })
            .map(|d| Target {
                id: d.instance_id(),
                root: d.root.clone(),
                path: d.path.clone(),
                revision: d.tools.disk_revision,
            })
            .collect();
        if targets.is_empty() {
            self.worker = None;
        } else {
            if self.worker.is_none() {
                self.worker = Worker::start().ok();
            }
            if let Some(worker) = &self.worker {
                self.pending = worker.requests.send(targets).is_ok();
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe(watch: &mut DocumentWatch, docs: &mut [Document], now: Instant) -> bool {
        let mut changed = watch.poll(docs, now);
        let deadline = Instant::now() + Duration::from_secs(5);
        while watch.pending {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
            changed |= watch.poll(docs, now);
        }
        changed
    }

    #[test]
    fn document_watch_handles_edits_conflicts_deletion_and_return_without_saving() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "old").unwrap();
        let mut docs = [Document::open(dir.path(), &file).unwrap()];
        let mut watch = DocumentWatch::default();
        let now = Instant::now();
        assert!(!observe(&mut watch, &mut docs, now));
        fs::write(&file, "external update\r\n").unwrap();
        assert!(observe(&mut watch, &mut docs, now + Duration::from_secs(1)));
        assert_eq!(docs[0].text, "external update\r\n");
        assert!(!docs[0].dirty());
        docs[0].text = "unsaved".into();
        fs::write(&file, "another version").unwrap();
        assert!(observe(&mut watch, &mut docs, now + Duration::from_secs(2)));
        assert_eq!(
            docs[0].tools.conflict_disk.as_deref(),
            Some("another version")
        );
        assert_eq!(docs[0].text, "unsaved");
        fs::remove_file(&file).unwrap();
        assert!(observe(&mut watch, &mut docs, now + Duration::from_secs(3)));
        assert!(docs[0].deleted_on_disk && docs[0].tools.conflict_disk.is_none());
        fs::write(&file, "unsaved").unwrap();
        assert!(observe(&mut watch, &mut docs, now + Duration::from_secs(4)));
        assert!(!docs[0].deleted_on_disk && !docs[0].dirty());
    }

    #[test]
    fn document_watch_drops_results_from_before_a_save_or_from_a_closed_tab() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "initial").unwrap();
        let mut docs = [Document::open(dir.path(), &file).unwrap()];
        let mut watch = DocumentWatch::default();
        let now = Instant::now();
        watch.poll(&mut docs, now);
        docs[0].text = "saved".into();
        docs[0].save(None).unwrap();
        assert!(!observe(&mut watch, &mut docs, now));
        assert_eq!(docs[0].text, "saved");
        fs::write(&file, "updated externally").unwrap();
        watch.poll(&mut docs, now + Duration::from_secs(1));
        docs[0] = Document::open(dir.path(), &file).unwrap();
        docs[0].text = "new tab draft".into();
        assert!(!observe(
            &mut watch,
            &mut docs,
            now + Duration::from_secs(1)
        ));
        assert_eq!(docs[0].text, "new tab draft");
        assert!(docs[0].tools.conflict_disk.is_none());
    }
}
