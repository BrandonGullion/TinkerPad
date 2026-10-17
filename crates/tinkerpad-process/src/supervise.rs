//! Keeping a long-lived child alive: state, crash backoff, idle shutdown,
//! memory sampling, and the pool that holds them.
//!
//! Language servers and plugin helpers are the same problem wearing different
//! protocols. Both start lazily, are shared by everything that needs them,
//! outlive the tab that caused them to start, come back after a crash until
//! they have crashed too often, stop when nobody has wanted them for a while,
//! and all stop together at app exit. None of that is about LSP or about
//! `tinkerpad-plugin-v1`, so none of it lives in either.
//!
//! [`Lifecycle`] is the bookkeeping one child needs; [`Pool`] holds many, keyed
//! however the caller likes. A protocol client implements [`Supervised`] and
//! gets [`watch`] for free. The debug adapters of Phase 6 are the third user
//! of this and should need nothing new (`docs/DEBUGGING.md` D5).
//!
//! See `docs/ARCHITECTURE.md` §4.1 and §5.1, and work items 02-02, 02-05,
//! 02-06, 03-02.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tinkerpad_core::ServerState;
use tokio::sync::watch;
use tracing::{debug, info, warn};

/// How long a polite stop is given before the process is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(2);

/// The wait before the first restart. It doubles with each crash in a row.
pub const BACKOFF_START: Duration = Duration::from_secs(1);

/// The longest wait between restarts, however many there have been.
pub const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// Crashes in a row before the pool stops trying. Six starts, five restarts.
pub const MAX_RESTARTS: u32 = 5;

/// A child that has run this long without crashing gets its budget back: a
/// crash after an hour of work is not part of the same run of bad luck.
pub const RESTART_WINDOW: Duration = Duration::from_secs(600);

/// How long a child nobody has ever used is kept. A pre-warmed language the
/// user never opened shouldn't sit there all day (work item 02-06).
pub const IDLE_UNUSED: Duration = Duration::from_secs(3600);

/// How often [`watch`] looks at a child.
pub const WATCH_TICK: Duration = Duration::from_secs(5);

/// How often resident memory is sampled. Cheap, but not free.
pub const RSS_EVERY: Duration = Duration::from_secs(10);

/// How many of a child's last words are kept for the status strip (02-08).
pub const LOG_LINES: usize = 500;

/// The whole of app exit may not take longer than this, however many children
/// there are and however wedged they may be.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);

/// How a pool treats one child: when to give up on it, and when to let it go.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// The first restart wait; it doubles from here, up to [`BACKOFF_CAP`].
    pub backoff: Duration,
    pub backoff_cap: Duration,
    pub max_restarts: u32,
    pub restart_window: Duration,
    /// How long with nothing using it before it is stopped. `None` is the
    /// manifest's `"never"`.
    pub idle_shutdown: Option<Duration>,
    /// The limit for a child nothing has ever used, which also caps
    /// `idle_shutdown` for one that has not been used yet.
    pub idle_unused: Duration,
    pub watch_tick: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            backoff: BACKOFF_START,
            backoff_cap: BACKOFF_CAP,
            max_restarts: MAX_RESTARTS,
            restart_window: RESTART_WINDOW,
            idle_shutdown: None,
            idle_unused: IDLE_UNUSED,
            watch_tick: WATCH_TICK,
        }
    }
}

/// What to do about a child that died without being asked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crash {
    /// Start it again after this wait. `count` is how many crashes in a row.
    Restart { after: Duration, count: u32 },
    /// It has been crashing steadily; something is properly wrong, and
    /// restarting it again would only burn the machine.
    GiveUp { count: u32 },
}

/// How often a child has crashed lately.
#[derive(Debug, Default)]
struct Restarts {
    count: u32,
    last: Option<Instant>,
}

/// One child's supervision state: what it is doing, how often it has crashed,
/// how long it has been doing nothing, and the last few things it said.
///
/// Holds no process and speaks no protocol. The thing that does owns one of
/// these and delegates to it.
#[derive(Debug)]
pub struct Lifecycle {
    /// What the log calls it: a language id, or a plugin id.
    name: Arc<str>,
    /// What it is, for log lines a person reads: "language server", "helper".
    noun: &'static str,
    policy: Policy,
    state: watch::Sender<ServerState>,
    restarts: Mutex<Restarts>,
    /// When the last user let go, or when it started with none. `None` while
    /// something is using it.
    idle_since: Mutex<Option<Instant>>,
    ever_used: AtomicBool,
    /// When the current process started, for uptime.
    started: Mutex<Option<Instant>>,
    log: Mutex<VecDeque<String>>,
}

impl Lifecycle {
    /// A child that has not started yet.
    pub fn new(name: impl Into<Arc<str>>, noun: &'static str, policy: Policy) -> Self {
        Self::in_state(name, noun, policy, ServerState::stopped())
    }

    /// A child that could not be started at all, and why.
    pub fn failed(
        name: impl Into<Arc<str>>,
        noun: &'static str,
        policy: Policy,
        reason: String,
        hint: Option<String>,
    ) -> Self {
        Self::in_state(name, noun, policy, ServerState::failed(reason, hint))
    }

    /// A child already in a state of its own: one the pool has prepared but
    /// not started, say.
    pub fn in_state(
        name: impl Into<Arc<str>>,
        noun: &'static str,
        policy: Policy,
        state: ServerState,
    ) -> Self {
        Self {
            name: name.into(),
            noun,
            policy,
            state: watch::Sender::new(state),
            restarts: Mutex::default(),
            idle_since: Mutex::new(Some(Instant::now())),
            ever_used: AtomicBool::new(false),
            started: Mutex::default(),
            log: Mutex::default(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn noun(&self) -> &'static str {
        self.noun
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    pub fn state(&self) -> ServerState {
        self.state.borrow().clone()
    }

    /// State changes, for anything that wants to wait on one.
    pub fn watch_state(&self) -> watch::Receiver<ServerState> {
        self.state.subscribe()
    }

    /// Say what it is doing now. A state that hasn't changed publishes
    /// nothing, so a watcher isn't woken for a repeat.
    pub fn set_state(&self, state: ServerState) {
        self.state.send_if_modified(|current| {
            if *current == state {
                return false;
            }
            debug!(name = %self.name, noun = self.noun, ?state, "state");
            *current = state;
            true
        });
    }

    /// Update the memory shown against a running child, leaving the rest of
    /// its `Ready` alone. Does nothing when it isn't running.
    pub fn set_rss(&self, rss_bytes: Option<u64>) {
        if let ServerState::Ready {
            pid, name, version, ..
        } = self.state()
        {
            self.set_state(ServerState::Ready {
                pid,
                name,
                version,
                rss_bytes,
            });
        }
    }

    /// The process has just started. Resets uptime, and the idle clock if
    /// nothing is using it: a process brought back after an idle stop gets a
    /// whole idle period of its own, rather than the old clock that already
    /// ran out and would stop it again the moment it was ready.
    pub fn starting_now(&self) {
        let now = Instant::now();
        *self.started.lock().expect("started lock") = Some(now);
        let mut since = self.idle_since.lock().expect("idle lock");
        if since.is_some() {
            *since = Some(now);
        }
    }

    /// How long the current process has been up.
    pub fn uptime(&self) -> Option<Duration> {
        self.started
            .lock()
            .expect("started lock")
            .map(|at| at.elapsed())
    }

    /// Count a crash and say what to do about it.
    pub fn note_crash(&self) -> Crash {
        let count = {
            let mut restarts = self.restarts.lock().expect("restarts lock");
            let now = Instant::now();
            // One that has been fine for a while and then dies starts over
            // with a full budget.
            if restarts
                .last
                .is_some_and(|last| now - last > self.policy.restart_window)
            {
                restarts.count = 0;
            }
            restarts.count += 1;
            restarts.last = Some(now);
            restarts.count
        };
        if count > self.policy.max_restarts {
            Crash::GiveUp { count }
        } else {
            Crash::Restart {
                after: backoff(self.policy.backoff, count, self.policy.backoff_cap),
                count,
            }
        }
    }

    /// Forget the crashes so far. What a person asking for a restart means.
    pub fn forget_crashes(&self) {
        self.restarts.lock().expect("restarts lock").count = 0;
    }

    pub fn restarts(&self) -> u32 {
        self.restarts.lock().expect("restarts lock").count
    }

    /// Something is using it now: the idle clock stops, and it counts as used
    /// from here on.
    pub fn busy(&self) {
        self.ever_used.store(true, Ordering::Relaxed);
        *self.idle_since.lock().expect("idle lock") = None;
    }

    /// Nothing is using it any more: the idle clock starts.
    pub fn idle(&self) {
        let mut since = self.idle_since.lock().expect("idle lock");
        if since.is_none() {
            *since = Some(Instant::now());
        }
    }

    /// Whether anything has ever used it, as opposed to it being pre-warmed
    /// and left alone.
    pub fn ever_used(&self) -> bool {
        self.ever_used.load(Ordering::Relaxed)
    }

    /// How long it has had nothing to do, if it has nothing to do.
    pub fn idle_for(&self) -> Option<Duration> {
        self.idle_since
            .lock()
            .expect("idle lock")
            .map(|since| since.elapsed())
    }

    /// How long it may sit idle before it is stopped.
    pub fn idle_limit(&self) -> Option<Duration> {
        match (self.policy.idle_shutdown, self.ever_used()) {
            (limit, true) => limit,
            // Never used: held to the shorter of the two.
            (limit, false) => Some(
                limit
                    .unwrap_or(self.policy.idle_unused)
                    .min(self.policy.idle_unused),
            ),
        }
    }

    /// Whether it has now been idle longer than it is allowed to be.
    pub fn idle_expired(&self) -> bool {
        match (self.idle_limit(), self.idle_for()) {
            (Some(limit), Some(idle)) => idle >= limit,
            _ => false,
        }
    }

    /// Keep a line the child said, for the status strip. Already logged by
    /// whoever read it.
    pub fn note_log(&self, line: String) {
        let mut log = self.log.lock().expect("log lock");
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line);
    }

    /// The last few things it said, oldest first.
    pub fn log(&self) -> Vec<String> {
        self.log.lock().expect("log lock").iter().cloned().collect()
    }
}

/// How long to wait before the nth restart: the base, doubling each time, up
/// to `cap`. With the defaults that is 1s, 2s, 4s, 8s, 16s, 30s.
pub fn backoff(base: Duration, attempt: u32, cap: Duration) -> Duration {
    let doubled = base * 2u32.saturating_pow(attempt.saturating_sub(1));
    doubled.min(cap)
}

/// Resident memory, sampled no more often than it is worth sampling.
pub struct Rss {
    system: sysinfo::System,
    every: Duration,
    last: Instant,
}

impl std::fmt::Debug for Rss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rss").field("every", &self.every).finish()
    }
}

impl Rss {
    pub fn new(every: Duration) -> Self {
        Self {
            system: sysinfo::System::new(),
            every,
            // Due straight away: the first sample is the interesting one.
            last: Instant::now() - every,
        }
    }

    /// Whether enough time has passed to sample again.
    pub fn due(&self) -> bool {
        self.last.elapsed() >= self.every
    }

    /// Resident bytes of one process, or `None` if it can't be read.
    pub fn sample(&mut self, pid: u32) -> Option<u64> {
        self.last = Instant::now();
        if pid == 0 {
            return None;
        }
        let pid = sysinfo::Pid::from_u32(pid);
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        self.system.process(pid).map(|process| process.memory())
    }
}

impl Default for Rss {
    fn default() -> Self {
        Self::new(RSS_EVERY)
    }
}

/// A long-lived child a [`Pool`] can hold: it has a [`Lifecycle`], a process,
/// and a way to be asked to stop.
pub trait Supervised: Send + Sync + 'static {
    fn lifecycle(&self) -> &Lifecycle;

    /// The process id, or 0 when it isn't running.
    fn pid(&self) -> u32;

    /// Stop politely, within [`STOP_GRACE`] of whatever the protocol's own
    /// goodbye is.
    fn stop(&self) -> impl Future<Output = ()> + Send;

    /// End it now, without asking.
    fn kill(&self);

    fn state(&self) -> ServerState {
        self.lifecycle().state()
    }

    fn name(&self) -> &str {
        self.lifecycle().name()
    }
}

/// Watch one child: stop it when it has been idle too long, and keep its
/// memory figure fresh while it runs.
///
/// Ends when the pool lets go of the child, so a pool that forgets something
/// doesn't leave a task watching it for ever.
pub async fn watch<P: Supervised>(child: Arc<P>) {
    let tick = child.lifecycle().policy().watch_tick;
    let mut rss = Rss::new(RSS_EVERY);

    loop {
        tokio::time::sleep(tick).await;
        // Nobody holds it any more: the pool dropped it, so there is nothing
        // left to watch.
        if Arc::strong_count(&child) == 1 {
            return;
        }
        if !child.state().is_ready() {
            continue;
        }

        if child.lifecycle().idle_expired() {
            let idle = child.lifecycle().idle_for().unwrap_or_default();
            info!(
                name = child.name(),
                idle_s = idle.as_secs(),
                "stopping an idle {}",
                child.lifecycle().noun(),
            );
            // A clean stop, and no toast: nobody asked for this, so nobody
            // needs to be told. It comes back when something wants it.
            child.stop().await;
            continue;
        }

        if rss.due() {
            let bytes = rss.sample(child.pid());
            child.lifecycle().set_rss(bytes);
        }
    }
}

/// Every child of one kind the app has going.
///
/// The pool holds them; nothing else does. A caller gets an `Arc` and may keep
/// it as long as it likes, but only the pool decides when one is started or
/// forgotten.
#[derive(Debug)]
pub struct Pool<K, P> {
    children: Mutex<HashMap<K, Arc<P>>>,
}

impl<K, P> Default for Pool<K, P> {
    fn default() -> Self {
        Self {
            children: Mutex::default(),
        }
    }
}

impl<K: Eq + Hash + Clone, P: Supervised> Pool<K, P> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The child for `key`, if the pool has one.
    pub fn get(&self, key: &K) -> Option<Arc<P>> {
        self.children.lock().expect("pool lock").get(key).cloned()
    }

    /// The child for `key`, made by `make` if the pool hasn't got one.
    ///
    /// The second value says whether this call is what made it. Making happens
    /// under the pool's lock, so two callers racing to be first share one
    /// child rather than starting two; `make` should therefore prepare and
    /// return quickly, and leave anything slow to [`Supervised`] itself.
    ///
    /// `found` is called instead when there already was one, which is where a
    /// caller revives something that had stopped.
    pub fn get_or_make(
        &self,
        key: &K,
        make: impl FnOnce() -> Arc<P>,
        found: impl FnOnce(&Arc<P>),
    ) -> (Arc<P>, bool) {
        let mut children = self.children.lock().expect("pool lock");
        if let Some(child) = children.get(key) {
            found(child);
            return (child.clone(), false);
        }
        let child = make();
        children.insert(key.clone(), child.clone());
        (child, true)
    }

    /// Every child, for the status strip (02-08).
    pub fn all(&self) -> Vec<Arc<P>> {
        self.children
            .lock()
            .expect("pool lock")
            .values()
            .cloned()
            .collect()
    }

    /// Forget one. Whatever holds an `Arc` to it keeps it alive; the pool will
    /// make a new one next time it is asked.
    pub fn forget(&self, key: &K) -> Option<Arc<P>> {
        self.children.lock().expect("pool lock").remove(key)
    }

    pub fn len(&self) -> usize {
        self.children.lock().expect("pool lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Stop everything, politely and all at once, within `budget`. Whatever is
    /// still going when the budget runs out is killed.
    ///
    /// The pool is empty afterwards: this is app exit.
    pub async fn shutdown(&self, budget: Duration) {
        let children = std::mem::take(&mut *self.children.lock().expect("pool lock"));
        if children.is_empty() {
            return;
        }
        let count = children.len();
        let noun = children
            .values()
            .next()
            .map(|c| c.lifecycle().noun())
            .unwrap_or("child");
        let started = Instant::now();

        let mut stopping = tokio::task::JoinSet::new();
        for child in children.values() {
            let child = child.clone();
            stopping.spawn(async move { child.stop().await });
        }
        let all = async { while stopping.join_next().await.is_some() {} };
        if tokio::time::timeout(budget, all).await.is_err() {
            warn!("some {noun}s did not stop in time; killing them");
            for child in children.values() {
                child.kill();
            }
        }
        info!(
            count,
            ms = started.elapsed().as_millis() as u64,
            "{noun}s stopped",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lifecycle(policy: Policy) -> Lifecycle {
        Lifecycle::new("test", "test child", policy)
    }

    #[test]
    fn the_wait_between_restarts_doubles_up_to_the_cap() {
        let base = Duration::from_secs(1);
        let cap = BACKOFF_CAP;
        let waits: Vec<u64> = (1..=7).map(|n| backoff(base, n, cap).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn crashing_steadily_is_given_up_on_but_crashing_once_is_not() {
        let life = lifecycle(Policy {
            max_restarts: 2,
            ..Policy::default()
        });
        assert!(matches!(life.note_crash(), Crash::Restart { count: 1, .. }));
        assert!(matches!(life.note_crash(), Crash::Restart { count: 2, .. }));
        assert_eq!(life.note_crash(), Crash::GiveUp { count: 3 });

        // Someone asks for it again: the count starts over.
        life.forget_crashes();
        assert!(matches!(life.note_crash(), Crash::Restart { count: 1, .. }));
    }

    #[test]
    fn a_crash_after_a_long_quiet_spell_starts_the_count_again() {
        let life = lifecycle(Policy {
            restart_window: Duration::ZERO,
            ..Policy::default()
        });
        life.note_crash();
        life.note_crash();
        // Every crash is "after the window", so none of them is a run of bad
        // luck and the count never climbs.
        assert!(matches!(life.note_crash(), Crash::Restart { count: 1, .. }));
    }

    #[test]
    fn starting_again_restarts_the_idle_clock() {
        let life = lifecycle(Policy {
            idle_shutdown: Some(Duration::from_millis(50)),
            ..Policy::default()
        });
        life.busy();
        life.idle();
        std::thread::sleep(Duration::from_millis(80));
        assert!(life.idle_expired(), "the clock ran out, so it was stopped");

        // Brought back with nothing using it: a fresh period, not the old one.
        life.starting_now();
        assert!(!life.idle_expired());

        // Something using it keeps the clock stopped across a restart.
        life.busy();
        life.starting_now();
        assert_eq!(life.idle_for(), None);
    }

    #[test]
    fn something_never_used_is_held_to_the_shorter_limit() {
        let hour = Duration::from_secs(3600);
        let life = lifecycle(Policy {
            idle_shutdown: None,
            idle_unused: Duration::from_secs(60),
            ..Policy::default()
        });
        assert_eq!(life.idle_limit(), Some(Duration::from_secs(60)));

        life.busy();
        // Used, and the manifest says never: it stays.
        assert_eq!(life.idle_limit(), None);
        assert_eq!(life.idle_for(), None, "it isn't idle at all");

        let life = lifecycle(Policy {
            idle_shutdown: Some(hour * 2),
            idle_unused: hour,
            ..Policy::default()
        });
        assert_eq!(life.idle_limit(), Some(hour), "the shorter of the two");
        life.busy();
        assert_eq!(
            life.idle_limit(),
            Some(hour * 2),
            "its own limit, once used"
        );
    }

    #[test]
    fn the_idle_clock_starts_when_the_last_user_lets_go() {
        let life = lifecycle(Policy {
            idle_shutdown: Some(Duration::ZERO),
            ..Policy::default()
        });
        life.busy();
        assert!(!life.idle_expired(), "something is using it");
        life.idle();
        assert!(life.idle_expired());
        let first = life.idle_for().unwrap();
        life.idle();
        assert!(
            life.idle_for().unwrap() >= first,
            "a second let-go does not restart the clock",
        );
    }

    #[test]
    fn the_log_keeps_the_last_few_hundred_lines() {
        let life = lifecycle(Policy::default());
        for n in 0..LOG_LINES + 10 {
            life.note_log(format!("line {n}"));
        }
        let log = life.log();
        assert_eq!(log.len(), LOG_LINES);
        assert_eq!(log[0], "line 10", "the oldest went");
        assert_eq!(log[LOG_LINES - 1], format!("line {}", LOG_LINES + 9));
    }

    #[test]
    fn a_state_that_has_not_changed_wakes_nobody() {
        let life = lifecycle(Policy::default());
        let mut states = life.watch_state();
        assert!(!states.has_changed().unwrap());
        life.set_state(ServerState::Starting);
        assert!(states.has_changed().unwrap());
        states.mark_unchanged();
        life.set_state(ServerState::Starting);
        assert!(!states.has_changed().unwrap(), "the same state again");
    }

    #[test]
    fn memory_is_shown_against_a_running_child_and_nothing_else() {
        let life = lifecycle(Policy::default());
        life.set_rss(Some(1024));
        assert!(
            matches!(life.state(), ServerState::Stopped { .. }),
            "nothing to show it against",
        );
        life.set_state(ServerState::Ready {
            pid: 7,
            name: Some("thing".to_owned()),
            version: None,
            rss_bytes: None,
        });
        life.set_rss(Some(1024));
        assert!(
            matches!(
                life.state(),
                ServerState::Ready {
                    pid: 7,
                    rss_bytes: Some(1024),
                    ..
                }
            ),
            "{:?}",
            life.state(),
        );
    }
}
