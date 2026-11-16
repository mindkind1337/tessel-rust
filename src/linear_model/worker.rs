//! Dedicated, bounded Linear worker. The renderer only enqueues and polls.
use super::{
    request::Generation,
    service::{Action, Delivery, Reply, Service},
};
use tokio::{sync::mpsc, task::JoinSet};
const CAPACITY: usize = 32;
struct Command {
    id: u64,
    action: Action,
}
pub struct Event {
    id: u64,
    delivery: Delivery,
}
impl Event {
    pub fn id(&self) -> u64 {
        self.id
    }
    /// UI must additionally match this id to its current request slot.
    pub fn apply<T>(
        self,
        apply: impl FnOnce(u64, Result<Reply, String>) -> T,
    ) -> Result<T, String> {
        self.delivery.apply(|reply| apply(self.id, reply))
    }
}
pub struct Worker {
    // First field closes authority before dropping channels. Joining on UI is forbidden.
    _owner: Generation,
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    sequence: u64,
    _thread: std::thread::JoinHandle<()>,
}
impl Worker {
    pub fn new_at(path: std::path::PathBuf) -> Result<Self, String> {
        let owner = Generation::default();
        let service = Service::new_at(owner.scope(), path);
        Self::with_service(owner, service)
    }
    pub fn new() -> Result<Self, String> {
        let owner = Generation::default();
        let service = Service::new(owner.scope())?;
        Self::with_service(owner, service)
    }
    pub(super) fn with_service(owner: Generation, service: Service) -> Result<Self, String> {
        if !service.accepts_owner(&owner.scope()) {
            return Err("Wrong Linear worker owner".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Cannot start Linear runtime")?;
        let (commands, receiver) = mpsc::channel(CAPACITY);
        let (sender, events) = mpsc::channel(CAPACITY);
        let thread = std::thread::Builder::new()
            .name("tessel-linear".into())
            .spawn(move || runtime.block_on(run(receiver, sender, service)))
            .map_err(|_| "Cannot start Linear worker")?;
        Ok(Self {
            _owner: owner,
            commands,
            events,
            sequence: 0,
            _thread: thread,
        })
    }
    pub fn submit(&mut self, action: Action) -> Result<u64, String> {
        let id = self
            .sequence
            .checked_add(1)
            .ok_or("Linear request identity exhausted")?;
        self.commands
            .try_send(Command { id, action })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "Linear worker is busy; try again",
                mpsc::error::TrySendError::Closed(_) => "Linear worker is unavailable",
            })?;
        self.sequence = id;
        Ok(id)
    }
    pub fn poll(&mut self) -> Result<Option<Event>, String> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err("Linear worker stopped".into()),
        }
    }
}
async fn run(mut commands: mpsc::Receiver<Command>, events: mpsc::Sender<Event>, service: Service) {
    let mut jobs = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            command=commands.recv(), if jobs.len()<CAPACITY => {
                let Some(command)=command else {break;};
                // Prepare account changes in dequeue order, before spawning network work.
                // Otherwise queued connects could capture one old scope and cancel the latest intent.
                let scope=service.current_scope();
                let prepared=service.prepare(&scope,&command.action);
                let service=service.clone();let events=events.clone();
                jobs.spawn(async move {
                    let delivery=match prepared {
                        Ok(scope)=>service.execute_prepared(scope,command.action).await,
                        Err(error)=>service.failure(scope,error),
                    };
                    let _=events.send(Event{id:command.id,delivery}).await;
                });
            },
            result=jobs.join_next(), if !jobs.is_empty()=>{
                // A panicked task closes this worker instead of leaving an unanswerable UI request.
                if matches!(result,Some(Err(_))){break;}
            },
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
}
