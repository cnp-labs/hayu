//! Owned Hyprland request/event workers.
//!
//! The event socket is authoritative for the active compositor session: it
//! bumps an epoch on connect/disconnect and a revision on every relevant event,
//! and asks for a coalesced snapshot refresh instead of publishing deltas.
//! Only the request worker talks to the UI channel, so a slow UI or a full
//! channel can never stall the event reader.
use super::{Event, Hyprland, Snapshot};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, socket_with};
use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Pending user workspace actions; never coalesced, never silently dropped.
const ACTION_CAP: usize = 16;
/// Total deadline for one request-socket exchange.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum MessageKind {
    Snapshot(Result<Snapshot, String>),
    Error(String),
}

#[derive(Debug)]
pub struct Message {
    pub generation: u64,
    pub epoch: u64,
    pub revision: u64,
    pub kind: MessageKind,
}

/// The bounded action queue is full, or the service was already stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// Why a request exchange ended. Typed so cancellation and deadline handling
/// never depends on matching message text.
#[derive(Debug)]
pub(super) enum Failure {
    /// The service stopped or the compositor connection epoch changed.
    Cancelled,
    /// The exchange exceeded its total deadline.
    TimedOut,
    /// The request socket could not be created, connected, read, or written.
    Io(String),
    /// The peer produced an unusable response.
    Protocol(String),
}

impl Failure {
    /// Cancellation and timeout end the whole refresh; other failures are
    /// tolerable for optional snapshot fields.
    fn fatal(&self) -> bool {
        matches!(self, Self::Cancelled | Self::TimedOut)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Hyprland request cancelled"),
            Self::TimedOut => formatter.write_str("Hyprland request timed out"),
            Self::Io(message) | Self::Protocol(message) => formatter.write_str(message),
        }
    }
}

fn io_error(context: &str, error: impl std::fmt::Display) -> Failure {
    Failure::Io(format!("{context}: {error}"))
}

/// Connection identity guard for one exchange. `Stop` only needs the service
/// to be running; `Epoch` also rejects work started before a reconnect.
#[derive(Clone, Copy)]
enum Guard {
    Stop,
    Epoch(u64),
}

struct State {
    stopped: bool,
    actions: VecDeque<i64>,
    refresh: bool,
    /// Set by a failed discovery/connect attempt; cleared when reported or on
    /// a successful connect, so an unattempted refresh stays quiet.
    failure: bool,
    epoch: u64,
    revision: u64,
    connected: Option<Hyprland>,
    request: Option<UnixStream>,
    events: Option<UnixStream>,
    pending: Option<Message>,
}

pub(super) struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    sender: calloop::channel::SyncSender<Message>,
}

pub struct Service {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Service {
    pub fn start(
        generation: u64,
        sender: calloop::channel::SyncSender<Message>,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                stopped: false,
                actions: VecDeque::new(),
                refresh: false,
                failure: false,
                epoch: 0,
                revision: 0,
                connected: None,
                request: None,
                events: None,
                pending: None,
            }),
            wake: Condvar::new(),
            sender,
        });
        let requests = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("hayu-hypr-request".into())
                .spawn(move || request_loop(shared, generation))
                .map_err(|e| format!("cannot start Hyprland request worker: {e}"))?
        };
        let events = {
            let shared = shared.clone();
            let on_error = shared.clone();
            match thread::Builder::new()
                .name("hayu-hypr-events".into())
                .spawn(move || event_loop(shared))
            {
                Ok(handle) => handle,
                Err(e) => {
                    stop_shared(&on_error);
                    let _ = requests.join();
                    return Err(format!("cannot start Hyprland event worker: {e}"));
                }
            }
        };
        Ok(Self {
            shared,
            workers: vec![requests, events],
        })
    }

    /// Request a fresh snapshot. Coalesced: many calls collapse into one job.
    pub fn refresh(&self) -> Result<(), QueueFull> {
        let mut state = lock(&self.shared);
        if state.stopped {
            return Err(QueueFull);
        }
        state.refresh = true;
        self.shared.wake.notify_all();
        Ok(())
    }

    pub fn switch_workspace(&self, id: i64) -> Result<(), QueueFull> {
        let mut state = lock(&self.shared);
        if state.stopped || state.actions.len() == ACTION_CAP {
            return Err(QueueFull);
        }
        state.actions.push_back(id);
        self.shared.wake.notify_all();
        Ok(())
    }

    /// `(epoch, revision, connected)` for validating a delivered snapshot.
    pub fn connection_stamp(&self) -> (u64, u64, bool) {
        let state = lock(&self.shared);
        (state.epoch, state.revision, state.connected.is_some())
    }

    /// The UI freed a slot in the result channel; let a blocked publish retry.
    pub fn output_drained(&self) {
        notify_drained(&self.shared);
    }

    pub fn stop(&mut self) {
        stop_shared(&self.shared);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The worker state is plain data, so recovering a poisoned lock cannot expose
/// a broken invariant; aborting the shell instead would be strictly worse.
fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait<'a>(shared: &'a Shared, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
    shared
        .wake
        .wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cancel_request(state: &mut State) {
    if let Some(socket) = state.request.take() {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
}

fn stop_shared(shared: &Shared) {
    let mut state = lock(shared);
    state.stopped = true;
    cancel_request(&mut state);
    if let Some(socket) = state.events.take() {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
    shared.wake.notify_all();
}

/// Publish one result without dropping it. The UI acknowledges each delivery
/// with `output_drained`, so a full channel blocks only this worker.
fn publish(shared: &Shared, message: Message) {
    let mut state = lock(shared);
    if state.stopped {
        return;
    }
    state.pending = Some(message);
    while state.pending.is_some() && !state.stopped {
        flush(shared, &mut state);
        if state.pending.is_some() {
            state = wait(shared, state);
        }
    }
    state.pending = None;
}

fn flush(shared: &Shared, state: &mut State) {
    let Some(message) = state.pending.take() else {
        return;
    };
    match shared.sender.try_send(message) {
        Ok(()) => {}
        Err(std::sync::mpsc::TrySendError::Full(message)) => state.pending = Some(message),
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
    }
}

/// Wake a publisher waiting for room in the result channel. Taking the lock
/// before notifying keeps the wake from being lost between check and wait.
fn notify_drained(shared: &Shared) {
    let _state = lock(shared);
    shared.wake.notify_all();
}

enum Job {
    Refresh {
        hypr: Option<Hyprland>,
        failure: bool,
        epoch: u64,
        revision: u64,
    },
    Switch {
        hypr: Option<Hyprland>,
        id: i64,
        epoch: u64,
        revision: u64,
    },
}

fn request_loop(shared: Arc<Shared>, generation: u64) {
    let mut last_refresh = false;
    loop {
        let job = {
            let mut state = lock(&shared);
            while !state.stopped && state.actions.is_empty() && !state.refresh {
                state = wait(&shared, state);
            }
            if state.stopped {
                return;
            }
            // Alternate so a refresh storm cannot starve queued user actions.
            let refresh = state.refresh && (state.actions.is_empty() || !last_refresh);
            let (epoch, revision) = (state.epoch, state.revision);
            if refresh {
                state.refresh = false;
                let failure = state.failure;
                state.failure = false;
                last_refresh = true;
                Job::Refresh {
                    hypr: state.connected.clone(),
                    failure,
                    epoch,
                    revision,
                }
            } else {
                let Some(id) = state.actions.pop_front() else {
                    continue;
                };
                last_refresh = false;
                Job::Switch {
                    hypr: state.connected.clone(),
                    id,
                    epoch,
                    revision,
                }
            }
        };

        match job {
            Job::Switch {
                hypr,
                id,
                epoch,
                revision,
            } => {
                let result = match hypr {
                    Some(hypr) => switch(&shared, &hypr, id, epoch),
                    None => Err(Failure::Protocol("Hyprland disconnected".into())),
                };
                match result {
                    Ok(()) | Err(Failure::Cancelled) => {}
                    Err(error) => publish(
                        &shared,
                        Message {
                            generation,
                            epoch,
                            revision,
                            kind: MessageKind::Error(format!(
                                "cannot switch to workspace {id}: {error}"
                            )),
                        },
                    ),
                }
            }
            Job::Refresh {
                hypr,
                failure,
                epoch,
                revision,
            } => {
                // A refresh requested while no attempt has failed yet has
                // nothing to report; only a real failed attempt becomes a
                // completed Snapshot(Err) for the current epoch.
                let result = match hypr {
                    Some(hypr) => snapshot(&shared, &hypr, epoch),
                    None if failure => Err(Failure::Protocol("Hyprland IPC unavailable".into())),
                    None => continue,
                };
                // Drop a snapshot overtaken by a stop, a reconnect, or a newer
                // event; a cancelled exchange has no result worth delivering.
                if matches!(result, Err(Failure::Cancelled)) {
                    continue;
                }
                let still_current = {
                    let current = lock(&shared);
                    !current.stopped && current.epoch == epoch && current.revision == revision
                };
                if still_current {
                    publish(
                        &shared,
                        Message {
                            generation,
                            epoch,
                            revision,
                            kind: MessageKind::Snapshot(result.map_err(|e| e.to_string())),
                        },
                    );
                }
            }
        }
    }
}

fn event_loop(shared: Arc<Shared>) {
    event_loop_from(shared, Hyprland::discover);
}

fn event_loop_from(shared: Arc<Shared>, discover: impl Fn() -> Option<Hyprland>) {
    let mut backoff = Duration::from_millis(250);
    loop {
        if lock(&shared).stopped {
            return;
        }
        let Some(hypr) = discover() else {
            fail_attempt(&shared);
            sleep_cancel(&shared, backoff);
            backoff = (backoff * 2).min(Duration::from_secs(5));
            continue;
        };
        let deadline = Instant::now() + TOTAL_TIMEOUT;
        let address = match address(&hypr.event_socket()) {
            Ok(address) => address,
            Err(_) => {
                fail_attempt(&shared);
                sleep_cancel(&shared, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        let stream = match pending_socket() {
            Ok(stream) => stream,
            Err(_) => {
                fail_attempt(&shared);
                sleep_cancel(&shared, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        // Register before connecting so stop can shut the socket down while the
        // non-blocking connect is still waiting for the listener.
        let handle = match stream.try_clone() {
            Ok(handle) => handle,
            Err(_) => {
                fail_attempt(&shared);
                sleep_cancel(&shared, backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        {
            let mut state = lock(&shared);
            if state.stopped {
                return;
            }
            state.events = Some(handle);
        }
        if finish_connect(&stream, &address, &shared, Guard::Stop, deadline).is_err()
            || stream.set_nonblocking(false).is_err()
        {
            fail_attempt(&shared);
            sleep_cancel(&shared, backoff);
            backoff = (backoff * 2).min(Duration::from_secs(5));
            continue;
        }
        {
            let mut state = lock(&shared);
            if state.stopped {
                return;
            }
            cancel_request(&mut state);
            state.epoch += 1;
            state.revision = 0;
            state.connected = Some(hypr);
            state.failure = false;
            state.refresh = true;
            shared.wake.notify_all();
        }
        backoff = Duration::from_millis(250);
        let mut lines = super::LineBuffer::default();
        let mut buf = [0; 8192];
        let mut stream = stream;
        loop {
            if lock(&shared).stopped {
                return;
            }
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for event in lines.feed(&buf[..n]) {
                        if !invalidates(&event) {
                            continue;
                        }
                        let mut state = lock(&shared);
                        state.revision += 1;
                        state.refresh = true;
                        shared.wake.notify_all();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        disconnect(&shared);
        sleep_cancel(&shared, backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

fn invalidates(event: &Event) -> bool {
    matches!(
        event,
        Event::Refresh
            | Event::Workspace { .. }
            | Event::CreateWorkspace { .. }
            | Event::DestroyWorkspace { .. }
            | Event::ActiveWindow { .. }
            | Event::FocusMonitor { .. }
    )
}

/// A failed discovery or connect attempt is a completed UI attempt: wake the
/// request worker so it publishes one coalesced `Snapshot(Err)` for the new
/// epoch and the UI can fall back. A plain event-stream disconnect does not
/// report this, because the last good snapshot stays usable.
fn fail_attempt(shared: &Shared) {
    disconnect(shared);
    let mut state = lock(shared);
    if state.stopped {
        return;
    }
    state.failure = true;
    state.refresh = true;
    shared.wake.notify_all();
}

fn disconnect(shared: &Shared) {
    let mut state = lock(shared);
    if state.stopped {
        return;
    }
    cancel_request(&mut state);
    state.epoch += 1;
    state.revision = 0;
    state.connected = None;
    if let Some(socket) = state.events.take() {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
    state.refresh = false;
    shared.wake.notify_all();
}

/// Reconnect backoff that a stop can interrupt immediately.
fn sleep_cancel(shared: &Shared, delay: Duration) {
    let deadline = Instant::now() + delay;
    let mut state = lock(shared);
    while !state.stopped {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        // Spurious wakes (for example a UI refresh) must not shorten the
        // reconnect backoff into a retry storm; stop still breaks out.
        state = shared
            .wake
            .wait_timeout(state, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
}

fn snapshot(shared: &Shared, hypr: &Hyprland, epoch: u64) -> Result<Snapshot, Failure> {
    let monitors = hypr.worker_request(shared, epoch, "j/monitors")?;
    let workspaces = hypr.worker_request(shared, epoch, "j/workspaces")?;
    // Optional fields may legitimately fail; cancellation and timeouts must not.
    let active_window = match hypr.worker_request(shared, epoch, "j/activewindow") {
        Ok(window) => window,
        Err(error) if error.fatal() => return Err(error),
        Err(_) => Default::default(),
    };
    let active_workspace = match hypr.worker_request(shared, epoch, "j/activeworkspace") {
        Ok(workspace) => workspace,
        Err(error) if error.fatal() => return Err(error),
        Err(_) => Default::default(),
    };
    Ok(Snapshot {
        monitors,
        workspaces,
        active_workspace,
        active_window,
    })
}

fn switch(shared: &Shared, hypr: &Hyprland, id: i64, epoch: u64) -> Result<(), Failure> {
    let legacy = hypr.worker_exchange(shared, epoch, &format!("dispatch workspace {id}"))?;
    if legacy.trim_start().starts_with("ok") {
        return Ok(());
    }
    // Only an explicit rejection justifies the Lua fallback: a transport
    // failure may already have executed the command.
    let lua = hypr.worker_exchange(
        shared,
        epoch,
        &format!("repl hl.dispatch(hl.dsp.focus({{ workspace = {id} }}))"),
    )?;
    if lua.trim_start().starts_with("ok") {
        Ok(())
    } else {
        Err(Failure::Protocol(format!(
            "legacy dispatch failed ({}); Lua dispatch failed ({})",
            legacy.trim(),
            lua.trim()
        )))
    }
}

pub(super) fn request<T: for<'de> serde::Deserialize<'de>>(
    hypr: &Hyprland,
    shared: &Shared,
    epoch: u64,
    command: &str,
) -> Result<T, Failure> {
    let text = exchange(hypr, shared, epoch, command)?;
    serde_json::from_str(&text)
        .map_err(|e| Failure::Protocol(format!("invalid Hyprland response for {command}: {e}")))
}

pub(super) fn exchange(
    hypr: &Hyprland,
    shared: &Shared,
    epoch: u64,
    command: &str,
) -> Result<String, Failure> {
    exchange_within(hypr, shared, epoch, command, TOTAL_TIMEOUT)
}

/// Bounded exchange. The socket is created non-blocking and registered before
/// connecting, so neither a full listen backlog nor a silent peer can outlive
/// the deadline or a stop. Every wait recomputes the remaining time, so a
/// slow-drip peer cannot extend the total.
fn exchange_within(
    hypr: &Hyprland,
    shared: &Shared,
    epoch: u64,
    command: &str,
    timeout: Duration,
) -> Result<String, Failure> {
    let deadline = Instant::now() + timeout;
    let address = address(&hypr.request_socket())?;
    let stream = pending_socket()?;
    {
        let mut state = lock(shared);
        check_locked(&state, Guard::Epoch(epoch), deadline)?;
        let handle = stream
            .try_clone()
            .map_err(|e| io_error("cannot clone Hyprland request socket", e))?;
        state.request = Some(handle);
    }
    let result = finish_connect(&stream, &address, shared, Guard::Epoch(epoch), deadline)
        .and_then(|()| transact(&stream, command, shared, epoch, deadline));
    lock(shared).request = None;
    result
}

fn address(path: &Path) -> Result<SocketAddrUnix, Failure> {
    SocketAddrUnix::new(path).map_err(|e| io_error("cannot address Hyprland socket", e))
}

fn pending_socket() -> Result<UnixStream, Failure> {
    let fd = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|e| io_error("cannot create Hyprland socket", e))?;
    Ok(UnixStream::from(fd))
}

/// Connect within the deadline. A non-blocking AF_UNIX connect reports a full
/// listen backlog as `EAGAIN`/`EWOULDBLOCK` without entering the pending state,
/// so `poll(POLLOUT)` plus `SO_ERROR == 0` would falsely claim success. Retry
/// `connect` on that error instead, waiting cancellably; only a real
/// `EINPROGRESS` waits for readiness.
fn finish_connect(
    stream: &UnixStream,
    address: &SocketAddrUnix,
    shared: &Shared,
    guard: Guard,
    deadline: Instant,
) -> Result<(), Failure> {
    loop {
        check(shared, guard, deadline)?;
        match rustix::net::connect(stream, address) {
            Ok(()) => {}
            Err(Errno::INPROGRESS) => wait_connect(stream, shared, guard, deadline)?,
            Err(Errno::WOULDBLOCK | Errno::ALREADY) => {
                wait_retry(shared, guard, deadline)?;
                continue;
            }
            Err(error) => return Err(io_error("cannot connect to Hyprland socket", error)),
        }
        match rustix::net::sockopt::socket_error(stream) {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(Errno::NOTCONN | Errno::INPROGRESS | Errno::ALREADY | Errno::WOULDBLOCK)) => {
                wait_retry(shared, guard, deadline)?;
            }
            Ok(Err(error)) => return Err(io_error("cannot connect to Hyprland socket", error)),
            Err(error) => return Err(io_error("cannot confirm Hyprland connection", error)),
        }
    }
}

/// Cancellable wait between connect retries. A socket that never entered the
/// pending state is not woken by shutdown, so this waits on the service
/// condition variable instead of polling the descriptor. The short slice only
/// bounds retry latency; stop and reconnect notify immediately.
fn wait_retry(shared: &Shared, guard: Guard, deadline: Instant) -> Result<(), Failure> {
    check(shared, guard, deadline)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Failure::TimedOut);
    }
    let state = lock(shared);
    drop(
        shared
            .wake
            .wait_timeout(state, remaining.min(Duration::from_millis(10)))
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    check(shared, guard, deadline)
}

/// Wait for a pending connect. The socket is not usable yet, and shutdown of
/// an unconnected socket is not guaranteed to wake `poll`, so this re-checks
/// stop, epoch, and the deadline on short slices instead of relying on it.
/// Only the bounded connect window is sliced; steady-state IO uses
/// [`wait_ready`].
fn wait_connect(
    stream: &UnixStream,
    shared: &Shared,
    guard: Guard,
    deadline: Instant,
) -> Result<(), Failure> {
    loop {
        check(shared, guard, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Failure::TimedOut);
        }
        let slice = remaining.min(Duration::from_millis(20));
        let timeout = Timespec {
            tv_sec: i64::try_from(slice.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: slice.subsec_nanos().into(),
        };
        let mut fds = [PollFd::new(stream, PollFlags::OUT)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => {}
            Ok(_) => return Ok(()),
            Err(Errno::INTR) => {}
            Err(error) => return Err(io_error("cannot wait on Hyprland socket", error)),
        }
    }
}

/// Wait for `events` with only the remaining time left before the deadline.
/// Stop and reconnect wake the wait by shutting the registered socket down, so
/// the timeout is not sliced into idle polling.
fn wait_ready(
    stream: &UnixStream,
    events: PollFlags,
    shared: &Shared,
    guard: Guard,
    deadline: Instant,
) -> Result<(), Failure> {
    loop {
        check(shared, guard, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Failure::TimedOut);
        }
        let timeout = Timespec {
            tv_sec: i64::try_from(remaining.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: remaining.subsec_nanos().into(),
        };
        let mut fds = [PollFd::new(stream, events)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Err(Failure::TimedOut),
            Ok(_) => {
                if fds[0].revents().contains(PollFlags::NVAL) {
                    return Err(Failure::Io(
                        "Hyprland socket became invalid while waiting".into(),
                    ));
                }
                return Ok(());
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(io_error("cannot wait on Hyprland socket", error)),
        }
    }
}

fn transact(
    stream: &UnixStream,
    command: &str,
    shared: &Shared,
    epoch: u64,
    deadline: Instant,
) -> Result<String, Failure> {
    check(shared, Guard::Epoch(epoch), deadline)?;
    let bytes = command.as_bytes();
    let mut sent = 0;
    while sent < bytes.len() {
        check(shared, Guard::Epoch(epoch), deadline)?;
        match rustix::io::write(stream, &bytes[sent..]) {
            Ok(0) => {
                return Err(Failure::Protocol(
                    "Hyprland request write returned zero".into(),
                ));
            }
            Ok(n) => sent += n,
            Err(Errno::WOULDBLOCK) => {
                wait_ready(
                    stream,
                    PollFlags::OUT,
                    shared,
                    Guard::Epoch(epoch),
                    deadline,
                )?;
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(io_error("cannot write Hyprland request", error)),
        }
    }
    let _ = rustix::net::shutdown(stream, rustix::net::Shutdown::Write);
    let mut reply = Vec::new();
    let mut buf = [0; 8192];
    loop {
        check(shared, Guard::Epoch(epoch), deadline)?;
        match rustix::io::read(stream, &mut buf[..]) {
            Ok(0) => {
                // A stop or reconnect can surface as EOF; report it as
                // cancellation instead of an empty successful reply.
                check_alive(shared, Guard::Epoch(epoch))?;
                break;
            }
            Ok(n) => {
                if reply.len() + n > super::MAX_RESPONSE {
                    return Err(Failure::Protocol("Hyprland response exceeded 4 MiB".into()));
                }
                reply.extend_from_slice(&buf[..n]);
            }
            Err(Errno::WOULDBLOCK) => {
                wait_ready(stream, PollFlags::IN, shared, Guard::Epoch(epoch), deadline)?;
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(io_error("Hyprland read failed", error)),
        }
    }
    String::from_utf8(reply)
        .map_err(|e| Failure::Protocol(format!("invalid Hyprland reply for {command}: {e}")))
}

fn check(shared: &Shared, guard: Guard, deadline: Instant) -> Result<(), Failure> {
    check_locked(&lock(shared), guard, deadline)
}

fn check_locked(state: &State, guard: Guard, deadline: Instant) -> Result<(), Failure> {
    if state.stopped {
        return Err(Failure::Cancelled);
    }
    if let Guard::Epoch(epoch) = guard
        && state.epoch != epoch
    {
        return Err(Failure::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Failure::TimedOut);
    }
    Ok(())
}

/// Stop/reconnect check without a deadline, for an EOF that may be legitimate.
fn check_alive(shared: &Shared, guard: Guard) -> Result<(), Failure> {
    let state = lock(shared);
    if state.stopped {
        return Err(Failure::Cancelled);
    }
    if let Guard::Epoch(epoch) = guard
        && state.epoch != epoch
    {
        return Err(Failure::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    fn shared(capacity: usize) -> (Arc<Shared>, calloop::channel::Channel<Message>) {
        let (sender, receiver) = calloop::channel::sync_channel(capacity);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                stopped: false,
                actions: VecDeque::new(),
                refresh: false,
                failure: false,
                epoch: 0,
                revision: 0,
                connected: None,
                request: None,
                events: None,
                pending: None,
            }),
            wake: Condvar::new(),
            sender,
        });
        (shared, receiver)
    }

    fn service_without_workers() -> Service {
        let (shared, _receiver) = shared(8);
        Service {
            shared,
            workers: Vec::new(),
        }
    }

    /// Minimal temporary directory so tests do not need an env var or a crate.
    mod tempdir {
        use std::path::PathBuf;

        pub struct Dir(pub PathBuf);
        impl Dir {
            pub fn new(tag: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "hayu-{tag}-{}-{:?}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn read_request(stream: &mut UnixStream) -> Option<String> {
        let mut request = Vec::new();
        let mut buf = [0; 256];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => request.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
        String::from_utf8(request).ok()
    }

    /// Fake compositor: accepts `connections` clients, records each full
    /// request, waits `delay`, then writes `reply`.
    fn recording_socket(
        connections: usize,
        delay: Duration,
        reply: &'static [u8],
    ) -> (tempdir::Dir, Arc<Mutex<Vec<String>>>) {
        let dir = tempdir::Dir::new("hypr-record");
        let path = dir.0.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&commands);
        thread::spawn(move || {
            for _ in 0..connections {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                if let Some(request) = read_request(&mut stream)
                    && let Ok(mut commands) = seen.lock()
                {
                    commands.push(request);
                }
                thread::sleep(delay);
                let _ = stream.write_all(reply);
            }
        });
        (dir, commands)
    }

    /// Fake compositor that accepts and never replies until the client leaves.
    fn silent_socket() -> tempdir::Dir {
        let dir = tempdir::Dir::new("hypr-silent");
        let path = dir.0.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = read_request(&mut stream);
            // The client half-closes after its request, so any further read
            // sees EOF. Holding the socket open without reading is what makes
            // this peer silent until the client gives up and leaves.
            thread::sleep(Duration::from_secs(2));
        });
        dir
    }

    /// Fake compositor that trickles bytes until the client gives up.
    fn drip_socket(interval: Duration) -> tempdir::Dir {
        let dir = tempdir::Dir::new("hypr-drip");
        let path = dir.0.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = read_request(&mut stream);
            for _ in 0..64 {
                if stream.write_all(b" ").is_err() {
                    return;
                }
                thread::sleep(interval);
            }
        });
        dir
    }

    /// Fill a fake listener's backlog so further non-blocking connects report
    /// `EAGAIN` until the queue drains. The listener and fillers stay alive.
    fn saturated_socket() -> (tempdir::Dir, std::os::fd::OwnedFd, Vec<UnixStream>) {
        let dir = tempdir::Dir::new("hypr-saturated");
        let path = dir.0.join(".socket.sock");
        let address = address(&path).unwrap();
        let listener = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        rustix::net::bind(&listener, &address).unwrap();
        rustix::net::listen(&listener, 1).unwrap();
        let mut fillers = Vec::new();
        for _ in 0..8 {
            let stream = pending_socket().unwrap();
            match rustix::net::connect(&stream, &address) {
                Ok(()) => fillers.push(stream),
                Err(Errno::WOULDBLOCK) => break,
                Err(error) => panic!("unexpected connect error: {error:?}"),
            }
        }
        assert!(!fillers.is_empty(), "fake listener queue never filled");
        let probe = pending_socket().unwrap();
        assert_eq!(
            rustix::net::connect(&probe, &address).unwrap_err(),
            Errno::WOULDBLOCK,
            "fake listener backlog must be saturated"
        );
        (dir, listener, fillers)
    }

    #[test]
    fn action_queue_is_bounded_and_fifo() {
        let service = service_without_workers();
        for id in 0..ACTION_CAP {
            assert_eq!(service.switch_workspace(id as i64), Ok(()));
        }
        assert_eq!(service.switch_workspace(99), Err(QueueFull));
        let state = lock(&service.shared);
        assert_eq!(
            state.actions.iter().copied().collect::<Vec<_>>(),
            (0..16).collect::<Vec<_>>()
        );
    }

    #[test]
    fn refresh_coalesces_into_one_bit() {
        let service = service_without_workers();
        for _ in 0..100 {
            assert_eq!(service.refresh(), Ok(()));
        }
        let state = lock(&service.shared);
        assert!(state.refresh);
        assert!(state.actions.is_empty());
    }

    #[test]
    fn stale_snapshot_stamp_is_rejected() {
        let service = service_without_workers();
        let (epoch, revision, connected) = service.connection_stamp();
        assert!(!connected);
        {
            let mut state = lock(&service.shared);
            state.epoch = epoch + 1;
            state.revision = revision;
        }
        let (now_epoch, now_revision, _) = service.connection_stamp();
        assert_ne!((epoch, revision), (now_epoch, now_revision));
    }

    #[test]
    fn stop_wakes_idle_worker_and_joins_quickly() {
        let (shared, _receiver) = shared(1);
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || request_loop(shared, 1))
        };
        thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        stop_shared(&shared);
        worker.join().unwrap();
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn full_output_keeps_message_until_drained() {
        let (shared, receiver) = shared(1);
        // Fill the channel with an older result the UI has not drained yet.
        shared
            .sender
            .try_send(Message {
                generation: 1,
                epoch: 0,
                revision: 0,
                kind: MessageKind::Error("queued".into()),
            })
            .unwrap();
        let publisher = {
            let shared = shared.clone();
            thread::spawn(move || {
                publish(
                    &shared,
                    Message {
                        generation: 1,
                        epoch: 0,
                        revision: 0,
                        kind: MessageKind::Error("retained".into()),
                    },
                )
            })
        };
        thread::sleep(Duration::from_millis(50));
        assert!(!publisher.is_finished(), "publisher must wait, not drop");

        let first = receiver.recv().unwrap();
        assert!(matches!(first.kind, MessageKind::Error(ref e) if e == "queued"));
        notify_drained(&shared);
        let second = receiver.recv().unwrap();
        assert!(matches!(second.kind, MessageKind::Error(ref e) if e == "retained"));
        publisher.join().unwrap();

        let started = Instant::now();
        stop_shared(&shared);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn stop_while_output_is_full_is_prompt() {
        let (shared, receiver) = shared(1);
        shared
            .sender
            .try_send(Message {
                generation: 1,
                epoch: 0,
                revision: 0,
                kind: MessageKind::Error("queued".into()),
            })
            .unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let publisher = {
            let shared = shared.clone();
            thread::spawn(move || {
                publish(
                    &shared,
                    Message {
                        generation: 1,
                        epoch: 0,
                        revision: 0,
                        kind: MessageKind::Error("blocked".into()),
                    },
                );
                done_tx.send(()).ok();
            })
        };
        thread::sleep(Duration::from_millis(50));
        assert!(!publisher.is_finished(), "publisher must block, not drop");

        let started = Instant::now();
        stop_shared(&shared);
        done_rx
            .recv_timeout(Duration::from_millis(300))
            .expect("publisher did not wake on stop");
        assert!(started.elapsed() < Duration::from_millis(300));
        publisher.join().unwrap();
        // The already queued result survives; the blocked one is discarded.
        assert!(
            matches!(receiver.recv().unwrap().kind, MessageKind::Error(ref e) if e == "queued")
        );
    }

    #[test]
    fn slow_reply_keeps_calloop_responsive() {
        let (dir, _commands) = recording_socket(4, Duration::from_millis(60), b"[]");
        let (shared, receiver) = shared(8);
        {
            let mut state = lock(&shared);
            state.epoch = 1;
            state.connected = Some(Hyprland::at(dir.0.clone()));
            state.refresh = true;
        }
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || request_loop(shared, 1))
        };

        struct LoopData {
            ticks: u32,
            ticks_at_message: Option<u32>,
            messages: Vec<Message>,
        }
        let mut event_loop: calloop::EventLoop<'static, LoopData> =
            calloop::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        handle
            .insert_source(
                calloop::timer::Timer::from_duration(Duration::from_millis(20)),
                |_, _, data: &mut LoopData| {
                    data.ticks += 1;
                    calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(20))
                },
            )
            .unwrap();
        handle
            .insert_source(receiver, |event, _, data: &mut LoopData| {
                if let calloop::channel::Event::Msg(message) = event {
                    data.ticks_at_message.get_or_insert(data.ticks);
                    data.messages.push(message);
                }
            })
            .unwrap();

        let mut data = LoopData {
            ticks: 0,
            ticks_at_message: None,
            messages: Vec::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        // Bounded dispatches: `EventLoop::run` would loop forever because its
        // timeout is the per-dispatch wait, not an overall deadline.
        while data.messages.is_empty() && Instant::now() < deadline {
            event_loop
                .dispatch(Duration::from_millis(30), &mut data)
                .unwrap();
        }
        assert_eq!(data.messages.len(), 1, "one coalesced snapshot arrives");
        assert!(matches!(
            data.messages[0].kind,
            MessageKind::Snapshot(Ok(_))
        ));
        assert!(
            data.ticks_at_message.unwrap_or(0) >= 1,
            "timer fired while the slow request was in flight"
        );

        stop_shared(&shared);
        worker.join().unwrap();
    }

    #[test]
    fn accepted_actions_run_fifo_under_saturation() {
        let (dir, commands) = recording_socket(ACTION_CAP, Duration::ZERO, b"ok");
        let service = service_without_workers();
        {
            let mut state = lock(&service.shared);
            state.epoch = 1;
            state.connected = Some(Hyprland::at(dir.0.clone()));
        }
        for id in 0..ACTION_CAP {
            assert_eq!(service.switch_workspace(id as i64), Ok(()));
        }
        assert_eq!(service.switch_workspace(99), Err(QueueFull));
        let worker = {
            let shared = service.shared.clone();
            thread::spawn(move || request_loop(shared, 1))
        };

        let deadline = Instant::now() + Duration::from_secs(3);
        let executed = loop {
            let executed = commands.lock().unwrap().clone();
            if executed.len() == ACTION_CAP || Instant::now() >= deadline {
                break executed;
            }
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(executed.len(), ACTION_CAP, "every accepted action ran");
        for (index, command) in executed.iter().enumerate() {
            assert_eq!(command, &format!("dispatch workspace {index}"));
        }

        stop_shared(&service.shared);
        worker.join().unwrap();
    }

    #[test]
    fn reconnect_rejects_stale_in_flight_snapshot() {
        let (dir, commands) = recording_socket(1, Duration::from_millis(250), b"[]");
        let (shared, receiver) = shared(8);
        {
            let mut state = lock(&shared);
            state.epoch = 1;
            state.connected = Some(Hyprland::at(dir.0.clone()));
            state.refresh = true;
        }
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || request_loop(shared, 1))
        };
        // Wait until the first request actually reached the fake compositor.
        let deadline = Instant::now() + Duration::from_secs(2);
        while commands.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "request never started");
            thread::sleep(Duration::from_millis(5));
        }
        // Simulate a reconnect: new epoch, old request cancelled.
        {
            let mut state = lock(&shared);
            state.epoch += 1;
            cancel_request(&mut state);
        }
        thread::sleep(Duration::from_millis(400));
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "stale snapshot must not reach the UI"
        );

        stop_shared(&shared);
        worker.join().unwrap();
    }

    #[test]
    fn exchange_enforces_total_deadline() {
        let dir = silent_socket();
        let (shared, _receiver) = shared(8);
        let hypr = Hyprland::at(dir.0.clone());
        let started = Instant::now();
        let error = exchange_within(&hypr, &shared, 0, "j/monitors", Duration::from_millis(120))
            .unwrap_err();
        assert!(matches!(error, Failure::TimedOut));
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(100),
            "returned early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "deadline overshoot: {elapsed:?}"
        );
    }

    #[test]
    fn slow_drip_cannot_extend_the_deadline() {
        let dir = drip_socket(Duration::from_millis(80));
        let (shared, _receiver) = shared(8);
        let hypr = Hyprland::at(dir.0.clone());
        let started = Instant::now();
        let error = exchange_within(&hypr, &shared, 0, "j/monitors", Duration::from_millis(250))
            .unwrap_err();
        assert!(matches!(error, Failure::TimedOut));
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "drip exceeded the total deadline: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn saturated_backlog_times_out_instead_of_false_connect() {
        let (dir, _listener, _fillers) = saturated_socket();
        let (shared, _receiver) = shared(8);
        let hypr = Hyprland::at(dir.0.clone());
        let started = Instant::now();
        let error = exchange_within(&hypr, &shared, 0, "j/monitors", Duration::from_millis(150))
            .unwrap_err();
        // A full backlog reports EAGAIN, not a pending connect. Treating that
        // as POLLOUT + SO_ERROR == 0 would produce a bogus empty reply.
        assert!(
            matches!(error, Failure::TimedOut),
            "expected timeout, got {error:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn saturated_backlog_stop_is_prompt() {
        let (dir, _listener, _fillers) = saturated_socket();
        let (shared, _receiver) = shared(8);
        let hypr = Hyprland::at(dir.0.clone());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || {
                let result =
                    exchange_within(&hypr, &shared, 0, "j/monitors", Duration::from_secs(5));
                done_tx.send(result).ok();
            })
        };
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        stop_shared(&shared);
        let result = done_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("connect retry did not stop");
        assert!(
            matches!(result, Err(Failure::Cancelled)),
            "expected cancellation, got {result:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        worker.join().unwrap();
    }

    #[test]
    fn idle_event_connection_does_not_reconnect() {
        let dir = tempdir::Dir::new("hypr-events");
        let path = dir.0.join(".socket2.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let accepts = Arc::new(Mutex::new(0u32));
        let seen = Arc::clone(&accepts);
        thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                if let Ok(mut count) = seen.lock() {
                    *count += 1;
                }
                let _ = stream.read(&mut [0; 8]);
                thread::sleep(Duration::from_secs(2));
            }
        });

        let (shared, _receiver) = shared(8);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = {
            let shared = shared.clone();
            let event_dir = dir.0.clone();
            thread::spawn(move || {
                event_loop_from(shared, move || Some(Hyprland::at(event_dir.clone())));
                done_tx.send(()).ok();
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while lock(&shared).connected.is_none() {
            assert!(Instant::now() < deadline, "event loop never connected");
            thread::sleep(Duration::from_millis(5));
        }
        // Longer than the reconnect backoff: a non-blocking event stream that
        // read as WouldBlock would drop and accept a second connection here.
        thread::sleep(Duration::from_millis(400));
        assert_eq!(*accepts.lock().unwrap(), 1, "idle event stream reconnected");
        assert!(lock(&shared).connected.is_some());

        stop_shared(&shared);
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("event worker did not stop");
        worker.join().unwrap();
    }

    #[test]
    fn discovery_failure_reports_once_per_attempt() {
        let (shared, receiver) = shared(8);
        let request_worker = {
            let shared = shared.clone();
            thread::spawn(move || request_loop(shared, 1))
        };
        // An app-issued refresh before any discovery attempt has nothing to
        // report; it must not become an unattempted Snapshot(Err).
        {
            let mut state = lock(&shared);
            state.refresh = true;
            shared.wake.notify_all();
        }
        thread::sleep(Duration::from_millis(30));
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "unattempted refresh must stay quiet"
        );

        let attempts = Arc::new(Mutex::new(0u32));
        let seen = Arc::clone(&attempts);
        let event_worker = {
            let shared = shared.clone();
            thread::spawn(move || {
                event_loop_from(shared, move || {
                    if let Ok(mut count) = seen.lock() {
                        *count += 1;
                    }
                    None
                })
            })
        };

        let deadline = Instant::now() + Duration::from_secs(2);
        let message = loop {
            match receiver.try_recv() {
                Ok(message) => break message,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "failed attempt never reported");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("result channel closed")
                }
            }
        };
        assert_eq!(message.generation, 1);
        assert!(matches!(message.kind, MessageKind::Snapshot(Err(_))));
        let (epoch, _, connected) = {
            let state = lock(&shared);
            (state.epoch, state.revision, state.connected.is_some())
        };
        assert_eq!(message.epoch, epoch, "error must carry the current epoch");
        assert!(!connected);
        assert_eq!(*attempts.lock().unwrap(), 1);

        // A later refresh while still disconnected has no new failed attempt,
        // so it must not emit a duplicate Err.
        {
            let mut state = lock(&shared);
            state.refresh = true;
            shared.wake.notify_all();
        }
        thread::sleep(Duration::from_millis(80));
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "at most one error per failed attempt"
        );
        assert_eq!(*attempts.lock().unwrap(), 1);

        stop_shared(&shared);
        request_worker.join().unwrap();
        event_worker.join().unwrap();
    }

    #[test]
    fn stop_cancels_a_connected_request_promptly() {
        let dir = silent_socket();
        let (shared, _receiver) = shared(8);
        {
            let mut state = lock(&shared);
            state.epoch = 1;
            state.connected = Some(Hyprland::at(dir.0.clone()));
            state.refresh = true;
        }
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || {
                request_loop(shared, 1);
                done_tx.send(()).ok();
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while lock(&shared).request.is_none() {
            assert!(Instant::now() < deadline, "request socket never registered");
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(30));

        let started = Instant::now();
        stop_shared(&shared);
        done_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("request worker did not stop");
        assert!(started.elapsed() < Duration::from_millis(500));
        worker.join().unwrap();
    }
}
