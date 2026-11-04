//! One floating terminal's lifetime, apart from the UI. A frame only asks and looks: starting
//! and stopping the program happen on a worker, because both can block (ending a local terminal
//! waits for its child, also in its `Drop`).
//!
//! - One worker per controller at a time, so never two programs for one floating terminal. A
//!   request during another is refused (`Busy`), except `close`, which joins it as a cancel.
//! - Nothing is retried on its own: a failure is shown (`error`) until the next accepted request.
//! - `close` and `restart` take the program out of the UI at once; it is stopped, then dropped,
//!   on the worker. A restart starts the new program only after that. A stop that fails gives
//!   the program back with the error: it may still run, and it must not disappear.
//! - A cancel is checked under the same lock as a program's publication, so a program created
//!   meanwhile is stopped by the worker that made it: none is left behind.
//! - Whether it is drawn is the UI's own state: hiding it never reaches this file.
//! - Dropping the controller never blocks: a running worker finishes alone and tells nobody, and
//!   a program held here goes to a thread of its own. That thread may not finish before the
//!   process exits: to quit, `close` and wait for `Phase::Closed` first.
//! - Bounded: at most `MAX_WORKERS` workers for requests in the application, each holding its
//!   permit until its thread ends. A controller dropped with a program adds one cleanup thread,
//!   counted (requests wait for it) but never refused, so the threads alive are at most
//!   `MAX_WORKERS` plus the controllers dropped meanwhile. The application has one controller.
//! - `stop` has to bound itself: a stop that never returns leaves its controller in `Closing` or
//!   `Restarting`, refusing further requests, with its worker's permit held.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Workers for requests running at once in the application.
pub const MAX_WORKERS: usize = 8;
/// Longest error kept, in bytes.
pub const MAX_ERROR_BYTES: usize = 1024;

static WORKERS: AtomicUsize = AtomicUsize::new(0);

/// What a floating terminal runs. Real: a local terminal.
pub trait Process: Send + 'static {
    /// On a worker, blocking. `Ok` only once the program is confirmed gone (one that already
    /// ended is `Ok`). `Err`: it may still run, and this object stays usable.
    fn stop(&mut self) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Closed,
    Opening,
    Ready,
    Restarting,
    Closing,
}

/// A request that did not start; nothing changed. Nothing retries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A program is already there: `restart` or `close` it.
    AlreadyOpen,
    /// No program to restart: `open` one.
    NotOpen,
    /// Another request is still running.
    Busy,
    /// Every worker of the application is busy.
    Capacity,
    /// The worker thread could not start.
    NoThread,
}

/// A worker's body, given to the thread that runs it.
pub type Work = Box<dyn FnOnce() + Send>;
type Wake = Arc<dyn Fn() + Send + Sync>;
type Spawn = Box<dyn Fn(Work) -> io::Result<()> + Send>;

/// What a worker ended with.
enum Done<T> {
    Opened(T),
    /// The program is gone and none replaced it.
    Closed,
    OpenFailed(String),
    /// The stop failed: the program comes back.
    StopFailed(T, String),
    Panicked,
}

struct Shared<T> {
    /// Given to the worker through here: a thread that cannot start leaves it for the UI.
    handover: Option<T>,
    /// `close` during an opening or a restart, or the controller gone.
    cancelled: bool,
    /// The controller is gone: tell nobody, keep nothing.
    abandoned: bool,
    done: Option<Done<T>>,
}

fn locked<S>(shared: &Mutex<S>) -> MutexGuard<'_, S> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A worker's place in the application's bound, held until its thread ends.
struct Permit(&'static AtomicUsize);

impl Permit {
    fn acquire(pool: &'static AtomicUsize) -> Option<Self> {
        pool.try_update(Ordering::SeqCst, Ordering::SeqCst, |running| {
            (running < MAX_WORKERS).then_some(running + 1)
        })
        .ok()
        .map(|_| Self(pool))
    }

    /// For cleanup after the controller is gone: counted, never refused.
    fn force(pool: &'static AtomicUsize) -> Self {
        pool.fetch_add(1, Ordering::SeqCst);
        Self(pool)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running worker's way back: says how it ended exactly once, also when it panics.
struct Delivery<T> {
    shared: Arc<Mutex<Shared<T>>>,
    wake: Wake,
    /// Set once the thread runs: one that never started has nothing to say.
    armed: bool,
}

impl<T> Delivery<T> {
    fn cancelled(&self) -> bool {
        locked(&self.shared).cancelled
    }

    fn take_handover(&self) -> Option<T> {
        locked(&self.shared).handover.take()
    }

    fn finish(mut self, done: Done<T>) {
        self.armed = false;
        self.publish(done);
    }

    fn publish(&self, done: Done<T>) {
        let unread = {
            let mut shared = locked(&self.shared);
            if shared.abandoned {
                Some(done)
            } else {
                shared.done = Some(done);
                None
            }
        };
        match unread {
            // Here on the worker and outside the lock: dropping a program can block.
            Some(done) => drop(done),
            None => (self.wake)(),
        }
    }

    /// Publishes a new program, or gives it back when a cancel came first. Both under one lock:
    /// a `close` either finds the program published or is seen here.
    fn offer(&mut self, process: T) -> Option<T> {
        {
            let mut shared = locked(&self.shared);
            if shared.cancelled {
                return Some(process);
            }
            shared.done = Some(Done::Opened(process));
        }
        self.armed = false;
        (self.wake)();
        None
    }
}

impl<T> Drop for Delivery<T> {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            self.publish(Done::Panicked);
        }
    }
}

/// Stops `process` and drops it, here on the worker. A failed stop gives it back.
fn stop_and_drop<T: Process>(mut process: T) -> Result<(), (T, String)> {
    match process.stop() {
        Ok(()) => {
            // Can block too: a local terminal waits for its child when dropped.
            drop(process);
            Ok(())
        }
        Err(error) => Err((process, error)),
    }
}

fn finish_stop<T: Process>(delivery: Delivery<T>, process: T) {
    match stop_and_drop(process) {
        Ok(()) => delivery.finish(Done::Closed),
        Err((process, error)) => delivery.finish(Done::StopFailed(process, error)),
    }
}

/// Makes a program and publishes it, unless a cancel came meanwhile: then it is stopped here.
fn open_and_publish<T: Process>(
    mut delivery: Delivery<T>,
    make: impl FnOnce() -> Result<T, String>,
) {
    if delivery.cancelled() {
        delivery.finish(Done::Closed);
        return;
    }
    match make() {
        // A cancel makes the failure moot: nothing is there, as was asked.
        Err(_) if delivery.cancelled() => delivery.finish(Done::Closed),
        Err(error) => delivery.finish(Done::OpenFailed(error)),
        Ok(process) => {
            if let Some(unwanted) = delivery.offer(process) {
                finish_stop(delivery, unwanted);
            }
        }
    }
}

fn spawn_thread(work: Work) -> io::Result<()> {
    std::thread::Builder::new()
        .name("floating-terminal".into())
        .spawn(work)
        .map(drop)
}

/// The UI's side of one floating terminal. Every method returns at once.
pub struct Controller<T: Process> {
    phase: Phase,
    /// Only while `Ready`: no worker runs then, so nothing else holds a program.
    process: Option<T>,
    error: Option<String>,
    shared: Arc<Mutex<Shared<T>>>,
    wake: Wake,
    pool: &'static AtomicUsize,
    spawn: Spawn,
}

impl<T: Process> Controller<T> {
    /// `wake` is called, never under a lock, when a result waits for `poll`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::bounded_by(wake, &WORKERS)
    }

    /// As `new`, counted in `pool` instead of the application's bound (tests).
    pub fn bounded_by(wake: impl Fn() + Send + Sync + 'static, pool: &'static AtomicUsize) -> Self {
        Self::with_spawn(wake, pool, spawn_thread)
    }

    /// As `bounded_by`, with `spawn` starting the threads (tests: a thread that cannot start).
    pub fn with_spawn(
        wake: impl Fn() + Send + Sync + 'static,
        pool: &'static AtomicUsize,
        spawn: impl Fn(Work) -> io::Result<()> + Send + 'static,
    ) -> Self {
        Self {
            phase: Phase::Closed,
            process: None,
            error: None,
            shared: Arc::new(Mutex::new(Shared {
                handover: None,
                cancelled: false,
                abandoned: false,
                done: None,
            })),
            wake: Arc::new(wake),
            pool,
            spawn: Box::new(spawn),
        }
    }

    fn start(
        &self,
        permit: Permit,
        work: impl FnOnce(Delivery<T>) + Send + 'static,
    ) -> Result<(), Refused> {
        let mut delivery = Delivery {
            shared: self.shared.clone(),
            wake: self.wake.clone(),
            armed: false,
        };
        let job: Work = Box::new(move || {
            let _permit = permit;
            delivery.armed = true;
            work(delivery);
        });
        (self.spawn)(job).map_err(|_| Refused::NoThread)
    }

    /// Starts the program off the UI. `make` runs on the worker.
    pub fn open(
        &mut self,
        make: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<(), Refused> {
        self.poll();
        match self.phase {
            Phase::Closed => {}
            Phase::Ready => return Err(Refused::AlreadyOpen),
            Phase::Opening | Phase::Restarting | Phase::Closing => return Err(Refused::Busy),
        }
        let permit = Permit::acquire(self.pool).ok_or(Refused::Capacity)?;
        locked(&self.shared).cancelled = false;
        self.start(permit, move |delivery| open_and_publish(delivery, make))?;
        self.phase = Phase::Opening;
        self.error = None;
        Ok(())
    }

    /// Stops the program off the UI, then starts the one from `make`, only once the first is
    /// confirmed gone. If it cannot be stopped it comes back with the error, and `make` is never
    /// called. Refused: the program stays as it was.
    pub fn restart(
        &mut self,
        make: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<(), Refused> {
        self.poll();
        match self.phase {
            Phase::Ready => {}
            Phase::Closed => return Err(Refused::NotOpen),
            Phase::Opening | Phase::Restarting | Phase::Closing => return Err(Refused::Busy),
        }
        let permit = Permit::acquire(self.pool).ok_or(Refused::Capacity)?;
        self.hand_over();
        let started = self.start(permit, move |delivery| {
            let Some(process) = delivery.take_handover() else {
                return;
            };
            match stop_and_drop(process) {
                Ok(()) => open_and_publish(delivery, make),
                Err((process, error)) => delivery.finish(Done::StopFailed(process, error)),
            }
        });
        if let Err(refused) = started {
            self.take_back();
            return Err(refused);
        }
        self.phase = Phase::Restarting;
        self.error = None;
        Ok(())
    }

    /// Ends the program off the UI, also one still starting: it is stopped as soon as it
    /// exists. If it cannot be stopped it comes back with the error. Refused (only for a program
    /// already there): it stays as it was. With nothing open, only clears the error.
    pub fn close(&mut self) -> Result<(), Refused> {
        self.poll();
        match self.phase {
            Phase::Closed => {}
            Phase::Closing => return Ok(()),
            Phase::Opening | Phase::Restarting => {
                let published = {
                    let mut shared = locked(&self.shared);
                    let done = shared.done.take();
                    if done.is_none() {
                        shared.cancelled = true;
                    }
                    done
                };
                // Published since the poll above: what it says decides, as if polled first.
                if let Some(done) = published {
                    self.apply(done);
                    return self.close();
                }
                self.phase = Phase::Closing;
            }
            Phase::Ready => {
                let permit = Permit::acquire(self.pool).ok_or(Refused::Capacity)?;
                self.hand_over();
                let started = self.start(permit, |delivery| {
                    if let Some(process) = delivery.take_handover() {
                        finish_stop(delivery, process);
                    }
                });
                if let Err(refused) = started {
                    self.take_back();
                    return Err(refused);
                }
                self.phase = Phase::Closing;
            }
        }
        self.error = None;
        Ok(())
    }

    /// On every frame: takes what a worker finished. No I/O, nothing blocks.
    pub fn poll(&mut self) {
        let done = locked(&self.shared).done.take();
        if let Some(done) = done {
            self.apply(done);
        }
    }

    fn apply(&mut self, done: Done<T>) {
        locked(&self.shared).cancelled = false;
        match done {
            Done::Opened(process) => {
                self.process = Some(process);
                self.phase = Phase::Ready;
            }
            Done::Closed => {
                self.phase = Phase::Closed;
            }
            Done::OpenFailed(error) => {
                self.phase = Phase::Closed;
                self.error = Some(bounded(error));
            }
            Done::StopFailed(process, error) => {
                self.process = Some(process);
                self.phase = Phase::Ready;
                self.error = Some(bounded(error));
            }
            Done::Panicked => {
                self.phase = Phase::Closed;
                self.error = Some("The floating terminal's worker stopped unexpectedly".into());
            }
        }
    }

    fn hand_over(&mut self) {
        let mut shared = locked(&self.shared);
        shared.cancelled = false;
        shared.handover = self.process.take();
    }

    /// The thread never started: the program is still in the handover.
    fn take_back(&mut self) {
        self.process = locked(&self.shared).handover.take();
    }

    /// The program, only while it is ready.
    pub fn process(&mut self) -> Option<&mut T> {
        self.process.as_mut()
    }

    /// Read-only view of the program while it is ready.
    pub fn process_ref(&self) -> Option<&T> {
        self.process.as_ref()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The last failure, until the next accepted request.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl<T: Process> Drop for Controller<T> {
    fn drop(&mut self) {
        // A running worker sees this and ends what it holds; what it already published is
        // gathered here, with the program held by the UI. Only one of the two can exist.
        let held = self.process.take();
        let done = {
            let mut shared = locked(&self.shared);
            shared.abandoned = true;
            shared.cancelled = true;
            shared.done.take()
        };
        let published = match done {
            Some(Done::Opened(process) | Done::StopFailed(process, _)) => Some(process),
            _ => None,
        };
        let Some(process) = held.or(published) else {
            return;
        };
        let permit = Permit::force(self.pool);
        let holder = Arc::new(Mutex::new(Some(process)));
        let worker = holder.clone();
        let job: Work = Box::new(move || {
            let _permit = permit;
            let process = locked(&worker).take();
            if let Some(process) = process {
                // Nobody is left to tell. A program that will not stop is still dropped.
                let _ = stop_and_drop(process);
            }
        });
        if (self.spawn)(job).is_err() {
            // Never a blocking drop on the UI thread: without a thread, it stays until the
            // application ends.
            std::mem::forget(locked(&holder).take());
        }
    }
}

fn bounded(mut error: String) -> String {
    if error.len() > MAX_ERROR_BYTES {
        let mut end = MAX_ERROR_BYTES;
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        error.truncate(end);
        error.shrink_to_fit();
    }
    error
}
