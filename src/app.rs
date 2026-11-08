use std::collections::HashMap;
use std::os::raw::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use calloop::channel::Event as ChannelEvent;
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken};
use chrono::{Local, Timelike};
use cosmic_text::FontSystem;
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::WaylandSurface,
    shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        LayerSurfaceConfigure,
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ZwpTextInputV3},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use crate::a11y::A11y;
use crate::bar::{Bar, NoEvents, ScaleTarget};
use crate::config::{self, Config};
use crate::desktop;
use crate::hyprland::worker::{Message, MessageKind, Service as HyprService};
use crate::hyprland::{Snapshot, Workspace};
use crate::ipc::{self, Command as IpcCommand};
use crate::launcher::{self, Launcher};
use crate::render::{self, Theme};
use crate::tray::{self, Message as TrayMessage, Service as TrayService};

pub fn run(cfg: Config) -> Result<(), String> {
    let conn =
        Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&conn)
        .map_err(|e| format!("cannot enumerate Wayland globals: {e}"))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| format!("wl_compositor unavailable: {e}"))?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|e| format!("wlr-layer-shell unavailable: {e}"))?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);
    let seat_state = SeatState::new(&globals, &qh);
    let ime_manager = globals.bind(&qh, 1..=1, ()).ok();
    if ime_manager.is_none() {
        eprintln!("hayu: text-input-v3 is not offered; IME support is off");
    }
    let viewporter: Option<WpViewporter> = globals.bind(&qh, 1..=1, NoEvents).ok();
    let fractional: Option<WpFractionalScaleManagerV1> = globals.bind(&qh, 1..=1, NoEvents).ok();
    let fractional = fractional.filter(|_| viewporter.is_some());
    if fractional.is_none() {
        eprintln!("hayu: fractional-scale/viewporter not offered; using integer buffer scale");
    }

    let (tray_tx, tray_rx) = calloop::channel::channel::<TrayMessage>();
    let tray_sender = tray_tx.clone();
    let (hypr_tx, hypr_rx) = calloop::channel::sync_channel::<Message>(64);
    let hypr_sender = hypr_tx.clone();
    let hypr_generation = 1;
    let theme = Theme::from_config(&cfg.appearance.mode, &cfg.appearance.accent);
    let mut fonts = FontSystem::new();
    if fonts.db_mut().is_empty() {
        eprintln!("hayu: no system fonts found; text will not be drawn");
    }
    let clock = render::format_clock(&cfg.clock.format, Local::now().time());
    let launcher_enabled = cfg.launcher.enabled;

    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().map_err(|e| format!("cannot create event loop: {e}"))?;
    let loop_handle = event_loop.handle();

    let mut state = State {
        registry_state,
        output_state,
        compositor,
        layer_shell,
        qh: qh.clone(),
        seat_state,
        keyboard: None,
        pointer: None,
        ime_manager,
        viewporter,
        fractional,
        ime: None,
        loop_handle: loop_handle.clone(),
        cfg,
        theme,
        fonts,
        display_ptr: conn.display().id().as_ptr() as *mut c_void,
        hypr_service: None,
        hypr_generation,
        hypr_sender,
        snapshot: None,
        snapshot_stamp: None,
        snapshot_attempt_stamp: None,
        active_workspace: 0,
        title: String::new(),
        clock,
        bars: HashMap::new(),
        bars_dirty: false,
        deferred_surfaces: Vec::new(),
        clock_timer: None,
        tray_service: None,
        tray_sender: tray_tx,
        tray_generation: 0,
        tray_items: Vec::new(),
        tray_version: 0,
        launcher: Launcher::new(Vec::new()),
        a11y: launcher_enabled.then(A11y::new),
        ipc_path: None,
        loop_signal: Some(event_loop.get_signal()),
    };
    state.resync();
    // Publish the hidden window before any launcher open, so a screen reader
    // that connects early sees a stable tree instead of an empty application.
    state.sync_a11y();

    // Populate outputs (and create their bars) before entering the event loop.
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("Wayland roundtrip failed: {e}"))?;
    // Bind only once startup work is done, so a CLI client never waits on a half-ready shell.
    let server = ipc::Server::bind()?;
    let (listener, ipc_path) = server.into_parts();
    state.ipc_path = Some(ipc_path);

    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| format!("cannot insert Wayland source: {e}"))?;

    if state.clock_timer.is_none() {
        state.clock_timer = schedule_clock(&mut state)?;
    }

    let signals = calloop::signals::Signals::new(&[
        calloop::signals::Signal::SIGINT,
        calloop::signals::Signal::SIGTERM,
    ])
    .map_err(|e| format!("cannot install signal handler: {e}"))?;
    loop_handle
        .insert_source(signals, |_, _, state| {
            if let Some(signal) = &state.loop_signal {
                signal.stop();
            }
        })
        .map_err(|e| format!("cannot insert signal source: {e}"))?;

    loop_handle
        .insert_source(hypr_rx, |event, _, state| {
            if let ChannelEvent::Msg(message) = event {
                state.handle_hypr_message(message);
            }
        })
        .map_err(|e| format!("cannot insert Hyprland channel: {e}"))?;

    state.sync_tray(tray_sender);
    loop_handle
        .insert_source(tray_rx, |event, _, state| {
            if let ChannelEvent::Msg(message) = event {
                state.handle_tray_message(message);
            }
        })
        .map_err(|e| format!("cannot insert tray channel: {e}"))?;
    let (ipc_tx, ipc_rx) = calloop::channel::channel::<ipc::Job>();
    loop_handle
        .insert_source(ipc_rx, |event, _, state| {
            if let ChannelEvent::Msg(job) = event {
                state.handle_ipc(job);
            }
        })
        .map_err(|e| format!("cannot insert control channel: {e}"))?;

    let control = Generic::new(listener, Interest::READ, Mode::Level);
    loop_handle
        .insert_source(control, move |_, listener, _| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let tx = ipc_tx.clone();
                        std::thread::spawn(move || ipc::handle_connection(stream, tx));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        eprintln!("hayu: control socket accept failed: {error}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| format!("cannot insert control socket: {e}"))?;

    let result = event_loop
        .run(None, &mut state, |state| state.flush_invalidated())
        .map_err(|e| format!("event loop failed: {e}"));
    if let Some(path) = &state.ipc_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockStep {
    Second,
    Minute,
}

fn generation_matches(message: u64, current: u64) -> bool {
    message == current
}

/// The mapping decision a snapshot error produces, or `None` when the error is
/// stale (other service generation or dropped connection) and must not touch
/// the overlay. A current error answered the attempt, so the pending launcher
/// falls back to the Wayland outputs instead of waiting for a snapshot.
fn snapshot_error_map(
    message_generation: u64,
    message_epoch: u64,
    current_generation: u64,
    current_epoch: u64,
    snapshot_current: bool,
) -> Option<LauncherMap> {
    (message_generation == current_generation && message_epoch == current_epoch).then(|| {
        launcher_map_decision(
            true,
            snapshot_current,
            needs_fresh_snapshot(
                true,
                Some((current_generation, current_epoch)),
                current_generation,
                current_epoch,
            ),
        )
    })
}
fn snapshot_stamp_matches(
    message_epoch: u64,
    message_revision: u64,
    current_epoch: u64,
    current_revision: u64,
    connected: bool,
) -> bool {
    connected && message_epoch == current_epoch && message_revision == current_revision
}

fn hypr_wanted(cfg: &Config, has_bars: bool, launcher_open: bool) -> bool {
    launcher_open
        || (has_bars
            && cfg.bar.enabled
            && [&cfg.bar.left, &cfg.bar.center, &cfg.bar.right]
                .iter()
                .any(|modules| {
                    modules
                        .iter()
                        .any(|module| module == "workspaces" || module == "active_window")
                }))
}

fn bar_uses_tray(cfg: &Config, output: Option<&str>) -> bool {
    cfg.bar.enabled
        && cfg
            .bar
            .outputs
            .iter()
            .any(|pattern| pattern == "*" || Some(pattern.as_str()) == output)
        && [&cfg.bar.left, &cfg.bar.center, &cfg.bar.right]
            .iter()
            .any(|modules| modules.iter().any(|module| module == "tray"))
}

/// A `%S` needs a per-second timer; `%%S` is a literal percent and does not.
fn clock_step(format: &str) -> ClockStep {
    let bytes = format.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'%' => i += 2,
            b'S' => return ClockStep::Second,
            _ => i += 2,
        }
    }
    ClockStep::Minute
}

/// No timer when no bar draws a clock. The config is re-read on reload, so
/// this also decides whether a timer exists after a reload.
fn clock_wanted(cfg: &Config, has_bars: bool) -> bool {
    has_bars
        && cfg.bar.enabled
        && [&cfg.bar.left, &cfg.bar.center, &cfg.bar.right]
            .iter()
            .any(|modules| modules.iter().any(|module| module == "clock"))
}

fn next_clock_delay(second: u32, millis: u32, step: ClockStep) -> Duration {
    let (period, elapsed) = match step {
        ClockStep::Second => (1_000, millis),
        ClockStep::Minute => (60_000, second * 1_000 + millis),
    };
    debug_assert!(elapsed < period);
    Duration::from_millis((period - elapsed) as u64 + 20)
}

fn duration_to_next_clock(format: &str) -> Duration {
    let now = Local::now();
    next_clock_delay(
        now.second(),
        now.timestamp_subsec_millis(),
        clock_step(format),
    )
}

/// Insert the clock timer, or nothing when the bar shows no clock.
fn schedule_clock(state: &mut State) -> Result<Option<RegistrationToken>, String> {
    if !clock_wanted(&state.cfg, !state.bars.is_empty()) {
        return Ok(None);
    }
    let timer = Timer::from_duration(duration_to_next_clock(&state.cfg.clock.format));
    let handle = state.loop_handle.clone();
    let token = handle
        .insert_source(timer, |_, _, state| {
            state.tick();
            TimeoutAction::ToInstant(
                Instant::now() + duration_to_next_clock(&state.cfg.clock.format),
            )
        })
        .map_err(|e| format!("cannot insert clock timer: {e}"))?;
    Ok(Some(token))
}

fn take_dirty(dirty: &mut bool) -> bool {
    std::mem::take(dirty)
}

struct Ime {
    object: ZwpTextInputV3,
    enabled: bool,
    /// Commit requests issued so far; `done` carries the matching serial.
    serial: u32,
    pending_preedit: Option<String>,
    pending_commit: Option<String>,
    pending_delete: u32,
}
impl Ime {
    /// Commit the pending requests and count them, because `done` echoes it.
    fn commit(&mut self) {
        self.object.commit();
        self.serial = self.serial.wrapping_add(1);
    }
}
/// How `map_launcher` resolves its output.
#[derive(Debug, PartialEq, Eq)]
enum LauncherMap {
    /// Use `launcher_output` now.
    Now,
    /// Hyprland owes this connection a snapshot; map from it instead of
    /// guessing a monitor.
    Wait,
}

/// True while a snapshot is still owed. Connected: the attempt for the current
/// epoch (success or failure) must have resolved, so a reconnect waits for the
/// new epoch. Disconnected: only a generation that never answered waits; once
/// it answered, the overlay falls back instead of waiting for a reconnect.
fn needs_fresh_snapshot(
    connected: bool,
    attempt: Option<(u64, u64)>,
    generation: u64,
    epoch: u64,
) -> bool {
    if connected {
        attempt != Some((generation, epoch))
    } else {
        attempt.is_none_or(|(answered, _)| answered != generation)
    }
}

/// `needs_fresh_snapshot` is true while the connection is up or while no
/// snapshot from the current service generation exists yet. A service that
/// already delivered a snapshot and then lost the compositor falls back to
/// the Wayland outputs instead of waiting forever.
fn launcher_map_decision(
    service_running: bool,
    snapshot_current: bool,
    needs_fresh_snapshot: bool,
) -> LauncherMap {
    if service_running && !snapshot_current && needs_fresh_snapshot {
        LauncherMap::Wait
    } else {
        LauncherMap::Now
    }
}

pub struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    qh: QueueHandle<State>,
    seat_state: SeatState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    ime_manager: Option<ZwpTextInputManagerV3>,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
    ime: Option<Ime>,
    loop_handle: LoopHandle<'static, State>,
    cfg: Config,
    theme: Theme,
    fonts: FontSystem,
    display_ptr: *mut c_void,
    hypr_service: Option<HyprService>,
    hypr_generation: u64,
    hypr_sender: calloop::channel::SyncSender<Message>,
    snapshot: Option<Snapshot>,
    /// Service generation and connection epoch of `snapshot`; a snapshot from
    /// another connection cannot name the focused monitor for this open.
    snapshot_stamp: Option<(u64, u64)>,
    /// Service generation and connection epoch of the last resolved discovery
    /// attempt, successful or not. An attempt answered for the current epoch
    /// ends a pending launcher wait; the epoch keeps a previous connection's
    /// answer from covering a reconnect.
    snapshot_attempt_stamp: Option<(u64, u64)>,
    active_workspace: i64,
    title: String,
    clock: String,
    bars: HashMap<u32, Bar>,
    /// Set by any state change that affects the bar; one flush per loop cycle.
    bars_dirty: bool,
    /// Removed layer surfaces whose native object must not be destroyed while
    /// the dispatch that dropped the role still holds it. Drained once the
    /// dispatch returns, before the next draw.
    deferred_surfaces: Vec<wl_surface::WlSurface>,
    clock_timer: Option<RegistrationToken>,
    tray_service: Option<TrayService>,
    tray_sender: calloop::channel::Sender<TrayMessage>,
    tray_generation: u64,
    tray_items: Vec<tray::TrayItem>,
    tray_version: u32,
    launcher: Launcher,
    a11y: Option<A11y>,
    ipc_path: Option<PathBuf>,
    loop_signal: Option<LoopSignal>,
}

impl State {
    fn handle_tray_message(&mut self, message: TrayMessage) {
        if self.tray_service.is_none() || message.generation != self.tray_generation {
            return;
        }
        self.handle_tray_event(message.event);
    }

    fn sync_tray(&mut self, tx: calloop::channel::Sender<TrayMessage>) {
        let has_consumer = self.bars.values().any(|bar| {
            let name = self
                .output_state
                .info(&bar.output)
                .and_then(|info| info.name);
            bar_uses_tray(&self.cfg, name.as_deref())
        });
        if !self.cfg.tray.enabled || !has_consumer {
            let had_service = self.tray_service.take().is_some();
            if had_service {
                self.tray_generation = self.tray_generation.wrapping_add(1);
            }
            // Idempotent: output updates call this often and must not churn
            // the icon strip version when nothing changed.
            if had_service || !self.tray_items.is_empty() {
                self.tray_items.clear();
                self.tray_version = self.tray_version.wrapping_add(1);
                self.invalidate_bars();
            }
            return;
        }
        if self.tray_service.is_none() {
            self.tray_generation = self.tray_generation.wrapping_add(1);
            match TrayService::start(self.tray_generation, tx) {
                Ok(service) => self.tray_service = Some(service),
                Err(error) => eprintln!("hayu: cannot start tray service: {error}"),
            }
        }
    }

    fn handle_tray_event(&mut self, event: tray::Event) {
        match event {
            tray::Event::Items(items) => {
                self.tray_items = items;
                self.tray_version = self.tray_version.wrapping_add(1);
                self.invalidate_bars();
            }
            tray::Event::Disabled(reason) => {
                eprintln!("hayu: {reason}");
                // The worker has exited; drop the inert service so a later
                // sync (reload or output change) can retry once instead of
                // holding a dead worker forever.
                if self.tray_service.take().is_some() {
                    self.tray_generation = self.tray_generation.wrapping_add(1);
                }
                self.tray_items.clear();
                self.tray_version = self.tray_version.wrapping_add(1);
                self.invalidate_bars();
            }
        }
    }
    fn tick(&mut self) {
        let clock = render::format_clock(&self.cfg.clock.format, Local::now().time());
        if clock != self.clock {
            self.clock = clock;
            self.invalidate_bars();
        }
    }

    fn invalidate_bars(&mut self) {
        self.bars_dirty = true;
    }

    /// Called once per event-loop cycle, after the dispatch returned: destroys
    /// the surfaces queued by `remove_bar` and friends, then collapses many
    /// invalidations into one draw.
    fn flush_invalidated(&mut self) {
        // SCTK still holds the layer role across the callback that removed it;
        // destroying the native surface earlier makes Hyprland kill the role
        // and SCTK then sends role.destroy to an invalid object (error 42).
        for surface in self.deferred_surfaces.drain(..) {
            surface.destroy();
        }
        if take_dirty(&mut self.bars_dirty) {
            self.render_bars();
        }
    }

    /// Reload can change `clock.format` or the widget list, so the timer's
    /// cadence is rebuilt immediately instead of waiting for the old deadline.
    fn reschedule_clock(&mut self) {
        if let Some(token) = self.clock_timer.take() {
            self.loop_handle.remove(token);
        }
        match schedule_clock(self) {
            Ok(token) => self.clock_timer = token,
            Err(error) => eprintln!("hayu: {error}"),
        }
    }

    fn request_hypr_refresh(&mut self) {
        if self.hypr_service.is_none() {
            if !hypr_wanted(&self.cfg, !self.bars.is_empty(), self.launcher.is_open()) {
                return;
            }
            self.hypr_generation = self.hypr_generation.wrapping_add(1);
            match HyprService::start(self.hypr_generation, self.hypr_sender.clone()) {
                Ok(service) => self.hypr_service = Some(service),
                Err(error) => {
                    eprintln!("hayu: cannot start Hyprland service: {error}");
                    return;
                }
            }
        }
        if self
            .hypr_service
            .as_ref()
            .is_some_and(|service| service.refresh().is_err())
        {
            eprintln!("hayu: Hyprland action queue full; refresh rejected");
        }
    }

    fn handle_hypr_message(&mut self, message: Message) {
        if let Some(service) = &self.hypr_service {
            service.output_drained();
        }
        if self.hypr_service.is_none()
            || !generation_matches(message.generation, self.hypr_generation)
        {
            return;
        }
        match message.kind {
            MessageKind::Snapshot(result) => {
                let (epoch, revision, connected) = self
                    .hypr_service
                    .as_ref()
                    .map_or((0, 0, false), HyprService::connection_stamp);
                match result {
                    Ok(snapshot) => {
                        if !snapshot_stamp_matches(
                            message.epoch,
                            message.revision,
                            epoch,
                            revision,
                            connected,
                        ) {
                            return;
                        }
                        self.apply_snapshot(snapshot, message.epoch);
                        // A launcher opened before the first snapshot of this
                        // connection maps only now.
                        if let Err(error) = self.map_launcher() {
                            eprintln!("hayu: {error}");
                        }
                    }
                    Err(error) => {
                        eprintln!("hayu: Hyprland snapshot failed: {error}");
                        // A failed discovery answers the attempt, so a pending
                        // launcher falls back instead of waiting forever. Only
                        // the current generation and connection may answer; a
                        // stale error must not move the overlay.
                        if snapshot_error_map(
                            message.generation,
                            message.epoch,
                            self.hypr_generation,
                            epoch,
                            self.snapshot_current(),
                        )
                        .is_some()
                        {
                            self.snapshot_attempt_stamp = Some((self.hypr_generation, epoch));
                            if let Err(error) = self.map_launcher() {
                                eprintln!("hayu: {error}");
                            }
                        }
                    }
                }
            }
            MessageKind::Error(error) => {
                eprintln!("hayu: Hyprland request failed: {error}");
            }
        }
    }

    fn apply_snapshot(&mut self, snapshot: Snapshot, epoch: u64) {
        self.active_workspace = snapshot.active_workspace.id;
        self.title = snapshot.active_window.title.clone();
        self.snapshot = Some(snapshot);
        self.snapshot_stamp = Some((self.hypr_generation, epoch));
        self.snapshot_attempt_stamp = Some((self.hypr_generation, epoch));
        self.invalidate_bars();
    }

    /// True only when `snapshot` came from the live connection. A reopen after
    /// the service stopped must not trust the saved monitor layout.
    fn snapshot_current(&self) -> bool {
        let Some(service) = &self.hypr_service else {
            return false;
        };
        let (epoch, _, connected) = service.connection_stamp();
        connected && self.snapshot_stamp == Some((self.hypr_generation, epoch))
    }

    fn hypr_connected(&self) -> bool {
        self.hypr_service
            .as_ref()
            .is_some_and(|service| service.connection_stamp().2)
    }

    fn needs_fresh_snapshot(&self) -> bool {
        let epoch = self
            .hypr_service
            .as_ref()
            .map_or(0, |service| service.connection_stamp().0);
        needs_fresh_snapshot(
            self.hypr_connected(),
            self.snapshot_attempt_stamp,
            self.hypr_generation,
            epoch,
        )
    }

    fn resync(&mut self) {
        self.request_hypr_refresh();
    }

    fn sync_hypr(&mut self) {
        let wanted = hypr_wanted(&self.cfg, !self.bars.is_empty(), self.launcher.is_open());
        if !wanted {
            if self.hypr_service.is_some() {
                self.hypr_generation = self.hypr_generation.wrapping_add(1);
                self.hypr_service = None;
            }
            self.snapshot = None;
            self.snapshot_stamp = None;
            self.snapshot_attempt_stamp = None;
            self.active_workspace = 0;
            self.title.clear();
            return;
        }
        if self.hypr_service.is_none() {
            self.request_hypr_refresh();
        }
    }

    fn handle_ipc(&mut self, job: ipc::Job) {
        let ipc::Job {
            request,
            mut stream,
        } = job;
        let (id, command) = match request {
            Ok(request) => {
                let id = request.id;
                (id, request.command())
            }
            Err(error) => (0, Err(error)),
        };
        let (response, quit) = match command {
            Ok(command) => {
                let result = match command {
                    IpcCommand::LauncherToggle => self.toggle_launcher().map(|()| None),
                    IpcCommand::LauncherOpen => self.open_launcher().map(|()| None),
                    IpcCommand::LauncherClose => {
                        self.close_launcher();
                        Ok(None)
                    }
                    IpcCommand::Quit => Ok(None),
                    IpcCommand::Reload => self.reload_config(),
                };
                match result {
                    Ok(Some(notice)) => (ipc::Response::ok_with_notice(id, notice), false),
                    Ok(None) => (ipc::Response::ok(id), command == IpcCommand::Quit),
                    Err(error) => (ipc::Response::error(id, error), false),
                }
            }
            Err(error) => (ipc::Response::error(id, error), false),
        };
        if let Err(error) = ipc::write_response(&mut stream, &response) {
            eprintln!("hayu: cannot answer control request: {error}");
        }
        if quit {
            // Text input must be disabled and committed before the surface and
            // the object go away, not just dropped with the connection.
            self.close_launcher();
            if let Some(signal) = &self.loop_signal {
                signal.stop();
            }
        }
    }

    /// Re-read the config file. A parse or validation error keeps the running
    /// state untouched, so a bad edit cannot take the shell down.
    fn reload_config(&mut self) -> Result<Option<String>, String> {
        let path = config::default_path();
        let cfg = if path.exists() {
            config::load(&path).map_err(|error| format!("{}: {error}", path.display()))?
        } else {
            config::Config::default()
        };
        let restart = config::restart_required(&self.cfg, &cfg);
        let theme = Theme::from_config(&cfg.appearance.mode, &cfg.appearance.accent);
        self.cfg = cfg;
        self.theme = theme;
        self.clock = render::format_clock(&self.cfg.clock.format, Local::now().time());
        self.reschedule_clock();
        if !self.cfg.launcher.enabled {
            self.release_launcher();
        } else if self.a11y.is_none() {
            self.a11y = Some(A11y::new());
            self.sync_a11y();
        }
        self.sync_bars();
        self.sync_hypr();
        self.sync_tray(self.tray_sender.clone());
        self.apply_bar_geometry();
        self.invalidate_bars();
        Ok((!restart.is_empty()).then(|| format!("restart required for: {}", restart.join(", "))))
    }

    /// Height and edge are layer-shell state, so a change needs a new configure
    /// before the next buffer.
    fn apply_bar_geometry(&mut self) {
        let height = self.cfg.bar.height.clamp(1, 4096) as u32;
        let bottom = self.cfg.bar.position == "bottom";
        for bar in self.bars.values_mut() {
            if bar.height == height && bar.bottom == bottom {
                continue;
            }
            let mut anchor = Anchor::LEFT | Anchor::RIGHT;
            anchor |= if bottom { Anchor::BOTTOM } else { Anchor::TOP };
            bar.layer.set_anchor(anchor);
            bar.layer.set_size(0, height);
            bar.layer.set_exclusive_zone(height as i32);
            bar.height = height;
            bar.bottom = bottom;
            bar.configured = false;
            bar.layer.commit();
        }
    }

    fn toggle_launcher(&mut self) -> Result<(), String> {
        if self.launcher.is_open() {
            self.close_launcher();
            Ok(())
        } else {
            self.open_launcher()
        }
    }

    fn open_launcher(&mut self) -> Result<(), String> {
        if !self.cfg.launcher.enabled {
            return Err("launcher is disabled in config".into());
        }
        if self.launcher.is_open() {
            return Ok(());
        }
        // Refresh on every open so an application installed since startup can
        // be launched without restarting the shell.
        self.launcher.set_entries(desktop::scan());
        self.launcher.begin_open();
        self.sync_hypr();
        self.request_hypr_refresh();
        // The overlay answers the request immediately; with a live Hyprland
        // service but no snapshot for this connection it stays pending and
        // maps from `apply_snapshot` instead of guessing a monitor.
        self.map_launcher()
    }

    /// Create the overlay on its target output. Called on open and again on
    /// the first snapshot of the current connection when the open was earlier.
    fn map_launcher(&mut self) -> Result<(), String> {
        if !self.launcher.is_open() || self.launcher.surface.is_some() {
            return Ok(());
        }
        if launcher_map_decision(
            self.hypr_service.is_some(),
            self.snapshot_current(),
            self.needs_fresh_snapshot(),
        ) == LauncherMap::Wait
        {
            return Ok(());
        }
        let output = match self.launcher_output() {
            Some(output) => output,
            None => {
                self.close_launcher();
                return Err("no output is available for the launcher".into());
            }
        };
        let scale = self
            .output_state
            .info(&output)
            .map(|info| info.scale_factor)
            .unwrap_or(1) as f32;
        let created = launcher::Surface::create(
            &self.compositor,
            &self.layer_shell,
            &self.qh,
            &output,
            scale,
            self.fractional.as_ref(),
            self.viewporter.as_ref(),
        );
        match created {
            Ok(surface) => {
                self.launcher.show(surface);
                // The draw happens on the configure event for this open;
                // painting here would attach a buffer before a reconfigure.
                Ok(())
            }
            Err(error) => {
                self.close_launcher();
                Err(error)
            }
        }
    }

    fn close_launcher(&mut self) {
        self.ime_disable();
        self.launcher.close();
        self.sync_a11y();
        self.sync_hypr();
    }

    /// A disabled launcher must not keep a surface, GPU/atlas resources, a
    /// scanned catalogue or an accessibility adapter alive.
    fn release_launcher(&mut self) {
        self.ime_disable();
        if let Some(surface) = self.launcher.release() {
            self.deferred_surfaces.push(surface);
        }
        self.a11y = None;
        self.sync_hypr();
    }
    fn ime_attach_seat(&mut self, qh: &QueueHandle<Self>, seat: &wl_seat::WlSeat) {
        if self.ime.is_some() {
            return;
        }
        let Some(manager) = &self.ime_manager else {
            return;
        };
        let object = manager.get_text_input(seat, qh, ());
        self.ime = Some(Ime {
            object,
            enabled: false,
            serial: 0,
            pending_preedit: None,
            pending_commit: None,
            pending_delete: 0,
        });
    }
    fn ime_is_launcher(&self, surface: &wl_surface::WlSurface) -> bool {
        self.launcher
            .surface
            .as_ref()
            .is_some_and(|launcher| launcher.layer.wl_surface() == surface)
    }
    fn ime_enable(&mut self) {
        if self.ime.as_ref().is_none_or(|ime| ime.enabled) {
            return;
        }
        if let Some(ime) = self.ime.as_mut() {
            ime.object.enable();
            ime.enabled = true;
        }
        if std::env::var_os("HAYU_DEBUG_IME").is_some() {
            eprintln!("hayu: ime enabled");
        }
        self.ime_sync(zwp_text_input_v3::ChangeCause::Other);
    }
    fn ime_disable(&mut self) {
        let Some(ime) = self.ime.as_mut() else {
            return;
        };
        if !ime.enabled {
            return;
        }
        ime.object.disable();
        ime.commit();
        ime.enabled = false;
        ime.pending_preedit = None;
        ime.pending_commit = None;
        ime.pending_delete = 0;
        if std::env::var_os("HAYU_DEBUG_IME").is_some() {
            eprintln!("hayu: ime disabled");
        }
        self.launcher.set_preedit(String::new());
    }
    fn ime_cursor_rect(&mut self, query: &str) -> (i32, i32, i32, i32) {
        let scale = self
            .launcher
            .surface
            .as_ref()
            .map_or(1.0, |surface| surface.scale);
        let x = (launcher::PAD + render::text_width(&mut self.fonts, query, launcher::INPUT_SIZE))
            / scale;
        (
            x as i32,
            (launcher::INPUT_Y / scale) as i32,
            2,
            (launcher::INPUT_SIZE / scale) as i32,
        )
    }
    /// Re-send the field state the protocol expects after the query changed.
    fn ime_sync(&mut self, cause: zwp_text_input_v3::ChangeCause) {
        if !self.ime.as_ref().is_some_and(|ime| ime.enabled) {
            return;
        }
        let query = self.launcher.query().to_string();
        let cursor = query.len() as i32;
        let (x, y, width, height) = self.ime_cursor_rect(&query);
        let Some(ime) = self.ime.as_mut() else {
            return;
        };
        ime.object.set_surrounding_text(query, cursor, cursor);
        ime.object.set_text_change_cause(cause);
        ime.object.set_content_type(
            zwp_text_input_v3::ContentHint::None,
            zwp_text_input_v3::ContentPurpose::Normal,
        );
        ime.object.set_cursor_rectangle(x, y, width, height);
        ime.commit();
    }
    fn ime_enter(&mut self, surface: &wl_surface::WlSurface) {
        if self.launcher.is_open() && self.ime_is_launcher(surface) {
            self.ime_enable();
        }
    }
    fn ime_leave(&mut self, surface: &wl_surface::WlSurface) {
        if self.ime_is_launcher(surface) {
            self.ime_disable();
        }
    }
    /// Apply the pending input-method state in the order the protocol requires.
    ///
    /// The compositor answers every commit request with `done`, so an empty
    /// one must not trigger a draw and sync of its own: that would echo
    /// forever.
    fn ime_done(&mut self, serial: u32) {
        let Some(ime) = self.ime.as_mut() else {
            return;
        };
        let current = serial == ime.serial;
        let preedit = ime.pending_preedit.take();
        let commit = ime.pending_commit.take();
        let delete = std::mem::take(&mut ime.pending_delete);
        let preedit_changed = preedit
            .as_deref()
            .is_some_and(|text| text != self.launcher.preedit());
        let query_changed = commit.is_some() || delete > 0;
        if !preedit_changed && !query_changed {
            return;
        }
        // Protocol order: drop the old preedit, delete, insert the commit,
        // then install the new preedit.
        self.launcher.set_preedit(String::new());
        if delete > 0 {
            self.launcher.delete_before(delete);
        }
        if let Some(text) = commit {
            self.launcher.insert_text(&text);
        }
        if let Some(text) = preedit {
            self.launcher.set_preedit(text);
        }
        if std::env::var_os("HAYU_DEBUG_IME").is_some() {
            eprintln!(
                "hayu: ime done delete={delete} query_len={} preedit_len={}",
                self.launcher.query().len(),
                self.launcher.preedit().len()
            );
        }
        self.draw_launcher();
        // Only a changed query needs new surrounding text. Re-sending it for a
        // preedit-only update makes the input method repeat the same preedit,
        // which would echo forever.
        if current && query_changed {
            self.ime_sync(zwp_text_input_v3::ChangeCause::InputMethod);
        }
    }

    /// Whether the config asks for a bar on this output name. `*` matches all.
    fn bar_wanted(&self, name: Option<&str>) -> bool {
        if !self.cfg.bar.enabled {
            return false;
        }
        self.cfg
            .bar
            .outputs
            .iter()
            .any(|pattern| pattern == "*" || Some(pattern.as_str()) == name)
    }

    /// Add bars for wanted outputs and destroy bars the config no longer asks
    /// for. Hotplug goes through `new_output`; this is for reload.
    fn sync_bars(&mut self) {
        let outputs: Vec<(wl_output::WlOutput, Option<String>)> = self
            .output_state
            .outputs()
            .map(|output| {
                let name = self.output_state.info(&output).and_then(|info| info.name);
                (output, name)
            })
            .collect();
        let mut keep = std::collections::HashSet::new();
        for (output, name) in outputs {
            let id = output.id().protocol_id();
            if !self.bar_wanted(name.as_deref()) {
                continue;
            }
            keep.insert(id);
            if !self.bars.contains_key(&id) {
                self.create_bar(output);
            }
        }
        let stale: Vec<u32> = self
            .bars
            .keys()
            .copied()
            .filter(|key| !keep.contains(key))
            .collect();
        for key in stale {
            self.remove_bar(key);
        }
        self.reschedule_clock();
        self.sync_tray(self.tray_sender.clone());
    }

    /// Drop a bar and queue its native surface; the destroy happens once the
    /// layer role is gone, in `flush_invalidated`.
    fn remove_bar(&mut self, key: u32) {
        if let Some(bar) = self.bars.remove(&key) {
            let surface = bar.layer.wl_surface().clone();
            drop(bar);
            self.deferred_surfaces.push(surface);
        }
    }

    fn launcher_output(&self) -> Option<wl_output::WlOutput> {
        // A snapshot from a stopped connection names a monitor that may no
        // longer be focused; fall back to the outputs instead of trusting it.
        let focused = self
            .snapshot
            .as_ref()
            .filter(|_| self.snapshot_current())
            .and_then(|snapshot| {
                snapshot
                    .monitors
                    .iter()
                    .find(|monitor| monitor.focused)
                    .map(|monitor| monitor.name.clone())
            });
        if let Some(name) = focused {
            for bar in self.bars.values() {
                if self
                    .output_state
                    .info(&bar.output)
                    .and_then(|info| info.name)
                    .as_deref()
                    == Some(name.as_str())
                {
                    return Some(bar.output.clone());
                }
            }
            // The focused monitor may run no bar (bar.outputs filter or
            // bar.enabled = false); the launcher still belongs there.
            if let Some(output) = self.output_state.outputs().find(|output| {
                self.output_state
                    .info(output)
                    .and_then(|info| info.name)
                    .as_deref()
                    == Some(name.as_str())
            }) {
                return Some(output);
            }
        }
        self.bars
            .values()
            .next()
            .map(|bar| bar.output.clone())
            .or_else(|| self.output_state.outputs().next())
    }

    fn handle_launcher_key(&mut self, event: KeyEvent) {
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            let (query, selected, results) = self.launcher.debug_state();
            eprintln!(
                "hayu: key keysym=0x{:x} utf8={:?} open={} before query={query:?} sel={selected} preedit={} results={results}",
                event.keysym.raw(),
                event.utf8,
                self.launcher.is_open(),
                self.launcher.preedit().len(),
            );
        }
        let action = self.launcher.key(&event);
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            let (query, selected, results) = self.launcher.debug_state();
            eprintln!(
                "hayu: key -> {action:?} query={query:?} sel={selected} preedit={} results={results} top={:?}",
                self.launcher.preedit().len(),
                self.launcher.debug_results()
            );
        }
        match action {
            launcher::Action::None => {}
            launcher::Action::Changed => {
                self.ime_sync(zwp_text_input_v3::ChangeCause::Other);
                self.draw_launcher();
            }
            launcher::Action::Close => self.close_launcher(),
            launcher::Action::Launch(entry) => match entry.launch() {
                Ok(()) => self.close_launcher(),
                Err(error) => {
                    eprintln!("hayu: {error}");
                    self.launcher.set_error(error);
                    self.draw_launcher();
                }
            },
        }
    }

    fn draw_launcher(&mut self) {
        let started = std::time::Instant::now();
        self.launcher
            .draw(&self.theme, &mut self.fonts, self.display_ptr);
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            eprintln!(
                "hayu: launcher frame {:.2} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        self.sync_a11y();
    }

    fn sync_a11y(&mut self) {
        if let Some(a11y) = self.a11y.as_mut() {
            a11y.sync(&self.launcher);
        }
    }

    fn create_bar(&mut self, output: wl_output::WlOutput) {
        let height = self.cfg.bar.height.clamp(1, 4096) as u32;
        let scale = self
            .output_state
            .info(&output)
            .map(|info| info.scale_factor)
            .unwrap_or(1)
            .max(1);
        let surface = self.compositor.create_surface(&self.qh);
        let output_id = output.id().protocol_id();
        // Both objects must exist before the surface's first commit.
        let fractional = self.fractional.as_ref().map(|manager| {
            manager.get_fractional_scale(&surface, &self.qh, ScaleTarget::Bar(output_id))
        });
        let viewport = self
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, &self.qh, NoEvents));
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Top,
            Some("hayu"),
            Some(&output),
        );
        let mut anchor = Anchor::LEFT | Anchor::RIGHT;
        anchor |= if self.cfg.bar.position == "bottom" {
            Anchor::BOTTOM
        } else {
            Anchor::TOP
        };
        layer.set_anchor(anchor);
        layer.set_size(0, height);
        layer.set_exclusive_zone(height as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.bars.insert(
            output_id,
            Bar::new(
                output,
                layer,
                scale as f32,
                scale as u32 * 256,
                height,
                self.cfg.bar.position == "bottom",
                fractional,
                viewport,
            ),
        );
    }

    fn render_bars(&mut self) {
        let tray_images: Vec<render::TrayImage> = self
            .tray_items
            .iter()
            .filter_map(|item| {
                item.icon.as_ref().map(|icon| render::TrayImage {
                    id: item.id,
                    width: icon.width,
                    height: icon.height,
                    rgba: icon.rgba.clone(),
                })
            })
            .collect();
        let tray_version = self.tray_version;
        let keys: Vec<u32> = self.bars.keys().copied().collect();
        for key in keys {
            let Some(output) = self.bars.get(&key).map(|bar| bar.output.clone()) else {
                continue;
            };
            let output_name = self
                .output_state
                .info(&output)
                .and_then(|info| info.name.clone());
            let (workspaces, title) = self.scene_for(output_name.as_deref());
            let Some(bar) = self.bars.get_mut(&key) else {
                continue;
            };
            bar.render(
                &self.cfg,
                &self.theme,
                &self.clock,
                self.active_workspace,
                &workspaces,
                &title,
                &tray_images,
                tray_version,
                &mut self.fonts,
                self.display_ptr,
            );
        }
    }

    fn scene_for(&self, output_name: Option<&str>) -> (Vec<Workspace>, String) {
        let Some(snapshot) = &self.snapshot else {
            return (Vec::new(), String::new());
        };
        let mut workspaces: Vec<Workspace> = snapshot
            .workspaces
            .iter()
            .filter(|ws| output_name.is_none_or(|name| ws.monitor == name))
            .cloned()
            .collect();
        workspaces.sort_by_key(|ws| ws.id);
        let focused = snapshot
            .monitors
            .iter()
            .find(|m| m.focused)
            .map(|m| m.name.as_str());
        let title = match (output_name, focused) {
            (Some(name), Some(focused)) if name == focused => snapshot.active_window.title.clone(),
            (None, _) => snapshot.active_window.title.clone(),
            _ => String::new(),
        };
        (workspaces, title)
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if self
            .launcher
            .surface
            .as_ref()
            .is_some_and(|launcher| launcher.layer.wl_surface() == surface)
            && self.launcher.set_output_scale(factor)
        {
            self.draw_launcher();
        }
        if let Some(bar) = self
            .bars
            .values_mut()
            .find(|bar| bar.layer.wl_surface() == surface)
            && bar.fractional.is_none()
        {
            bar.scale = factor.max(1) as f32;
        }
        self.invalidate_bars();
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let name = self.output_state.info(&output).and_then(|info| info.name);
        if self.bar_wanted(name.as_deref()) {
            self.create_bar(output);
        }
        self.sync_hypr();
        self.sync_tray(self.tray_sender.clone());
        self.reschedule_clock();
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if self.launcher.output_id() == Some(output.id().protocol_id())
                && self.launcher.set_output_scale(info.scale_factor)
            {
                self.draw_launcher();
            }
            if let Some(bar) = self.bars.get_mut(&output.id().protocol_id()) {
                if bar.fractional.is_none() {
                    bar.scale = info.scale_factor.max(1) as f32;
                }
            } else if self.bar_wanted(info.name.as_deref()) {
                // `new_output` can run before the name is known, so a filtered
                // bar may only become eligible here.
                self.create_bar(output.clone());
            }
        }
        self.sync_hypr();
        self.sync_tray(self.tray_sender.clone());
        self.reschedule_clock();
        self.invalidate_bars();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = output.id().protocol_id();
        if self.launcher.output_id() == Some(id) {
            self.ime_disable();
            if let Some(surface) = self.launcher.forget_surface() {
                self.deferred_surfaces.push(surface);
            }
            self.sync_a11y();
        }
        self.remove_bar(id);
        self.sync_tray(self.tray_sender.clone());
        self.sync_hypr();
        self.reschedule_clock();
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(surface) = self.launcher.on_layer_closed(layer) {
            self.deferred_surfaces.push(surface);
            self.ime_disable();
            self.sync_a11y();
            self.sync_hypr();
            return;
        }
        let key = self
            .bars
            .iter()
            .find(|(_, bar)| bar.layer.wl_surface() == layer.wl_surface())
            .map(|(key, _)| *key);
        if let Some(key) = key {
            self.remove_bar(key);
            self.sync_tray(self.tray_sender.clone());
            self.sync_hypr();
            self.reschedule_clock();
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if self
            .launcher
            .surface
            .as_ref()
            .is_some_and(|launcher| launcher.owns(layer))
        {
            self.launcher
                .resize(configure.new_size.0, configure.new_size.1);
            if self.launcher.is_open() {
                self.draw_launcher();
            }
            return;
        }
        let fallback = self.cfg.bar.height.clamp(1, 4096) as u32;
        let Some(bar) = self
            .bars
            .values_mut()
            .find(|bar| bar.layer.wl_surface() == layer.wl_surface())
        else {
            return;
        };
        bar.width = if configure.new_size.0 == 0 {
            800
        } else {
            configure.new_size.0
        };
        bar.height = if configure.new_size.1 == 0 {
            fallback
        } else {
            configure.new_size.1
        };
        bar.configured = true;
        self.invalidate_bars();
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if let PointerEventKind::Press {
                button: BTN_LEFT, ..
            } = event.kind
            {
                self.click_bar(&event.surface, event.position.0 as f32);
            }
        }
    }
}

impl State {
    /// Left click on the bar. Only workspace pills react; everything else in
    /// the bar ignores the click.
    fn click_bar(&mut self, surface: &wl_surface::WlSurface, logical_x: f32) {
        let Some(bar) = self
            .bars
            .values()
            .find(|bar| bar.layer.wl_surface() == surface)
        else {
            return;
        };
        let scale = bar.scale.max(1.0);
        if let Some(item) = render::tray_at(&bar.tray_hits, logical_x, scale)
            .and_then(|id| self.tray_items.iter().find(|item| item.id == id))
            .cloned()
        {
            if let Some(service) = &self.tray_service {
                match service.activate(&item) {
                    Ok(()) => {}
                    Err(tray::ActivateError::Full) => {
                        eprintln!("hayu: tray activation queue full; activation rejected");
                    }
                    Err(tray::ActivateError::Stopped) => {
                        eprintln!("hayu: tray service stopped; activation dropped");
                    }
                }
            }
            return;
        }
        let Some(id) = render::pill_at(&bar.pills, logical_x, scale) else {
            return;
        };
        if id == self.active_workspace {
            return;
        }
        let Some(service) = &self.hypr_service else {
            return;
        };
        if service.switch_workspace(id).is_err() {
            eprintln!("hayu: Hyprland action queue full; workspace switch rejected for {id}");
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.ime_attach_seat(qh, &seat);
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            // The initial seat is bound before the handler exists, so
            // `new_seat` never runs for it; capability is the reliable hook.
            self.ime_attach_seat(qh, &seat);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(error) => eprintln!("hayu: cannot create pointer: {error}"),
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|state, _, event| state.handle_launcher_key(event)),
            ) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(error) => eprintln!("hayu: cannot create keyboard: {error}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_launcher_key(event);
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_launcher_key(event);
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpTextInputManagerV3,
        _: <ZwpTextInputManagerV3 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwpTextInputV3, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { surface } => state.ime_enter(&surface),
            zwp_text_input_v3::Event::Leave { surface } => state.ime_leave(&surface),
            zwp_text_input_v3::Event::PreeditString { text, .. } => {
                if let Some(ime) = state.ime.as_mut() {
                    ime.pending_preedit = text;
                }
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                if let Some(ime) = state.ime.as_mut() {
                    ime.pending_commit = text;
                }
            }
            zwp_text_input_v3::Event::DeleteSurroundingText { before_length, .. } => {
                if let Some(ime) = state.ime.as_mut() {
                    ime.pending_delete = before_length;
                }
            }
            zwp_text_input_v3::Event::Done { serial } => state.ime_done(serial),
            _ => {}
        }
    }
}

impl Dispatch2<WpFractionalScaleV1, State> for ScaleTarget {
    fn event(
        &self,
        state: &mut State,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let scale = (scale as f32 / 120.0).max(1.0);
        match *self {
            ScaleTarget::Bar(id) => {
                let Some(bar) = state.bars.get_mut(&id) else {
                    return;
                };
                if (bar.scale - scale).abs() < 0.001 {
                    return;
                }
                bar.scale = scale;
                state.invalidate_bars();
            }
            ScaleTarget::Launcher => {
                if state.launcher.set_scale(scale) {
                    state.draw_launcher();
                }
            }
        }
    }
}
impl Dispatch2<WpViewport, State> for NoEvents {
    fn event(
        &self,
        _: &mut State,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
impl Dispatch2<WpViewporter, State> for NoEvents {
    fn event(
        &self,
        _: &mut State,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
impl Dispatch2<WpFractionalScaleManagerV1, State> for NoEvents {
    fn event(
        &self,
        _: &mut State,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_registry!(State);
smithay_client_toolkit::delegate_dispatch2!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_predicates_follow_actual_bar_consumers() {
        let mut cfg = Config::default();
        assert!(hypr_wanted(&cfg, true, false));
        assert!(!hypr_wanted(&cfg, false, false));
        assert!(hypr_wanted(&cfg, false, true));
        cfg.launcher.enabled = false;
        assert!(hypr_wanted(&cfg, true, false));
        cfg.bar.left.clear();
        cfg.bar.center.clear();
        cfg.bar.right.clear();
        assert!(!hypr_wanted(&cfg, true, false));
        cfg.bar.right.push("tray".into());
        assert!(!hypr_wanted(&cfg, true, false));
        cfg.tray.enabled = false;
        assert!(!hypr_wanted(&cfg, true, false));
        cfg.tray.enabled = true;
        cfg.bar.outputs = vec!["DP-1".into()];
        assert!(bar_uses_tray(&cfg, Some("DP-1")));
        assert!(
            !bar_uses_tray(&cfg, None),
            "an unnamed output does not match a named filter"
        );
        assert!(!bar_uses_tray(&cfg, Some("DP-2")));
        cfg.bar.center.push("clock".into());
        assert!(clock_wanted(&cfg, true));
        cfg.bar.enabled = false;
        assert!(!clock_wanted(&cfg, true));
        cfg.bar.enabled = true;
        cfg.bar.center.clear();
        assert!(!clock_wanted(&cfg, true));
    }

    #[test]
    fn generation_and_connection_stamps_reject_stale_results() {
        assert!(generation_matches(4, 4));
        assert!(!generation_matches(3, 4));
        assert!(snapshot_stamp_matches(2, 7, 2, 7, true));
        assert!(!snapshot_stamp_matches(1, 7, 2, 7, true));
        assert!(!snapshot_stamp_matches(2, 6, 2, 7, true));
        assert!(!snapshot_stamp_matches(2, 7, 2, 7, false));
    }

    #[test]
    fn clock_step_only_reacts_to_unescaped_seconds() {
        assert_eq!(clock_step("%H:%M:%S"), ClockStep::Second);
        assert_eq!(clock_step("%S"), ClockStep::Second);
        assert_eq!(clock_step("%H:%M %%S"), ClockStep::Minute);
        assert_eq!(clock_step("%H:%M"), ClockStep::Minute);
        assert_eq!(clock_step("literal"), ClockStep::Minute);
    }

    #[test]
    fn clock_wanted_requires_enabled_bar_with_clock_module() {
        let mut cfg = Config::default();
        assert!(clock_wanted(&cfg, true));
        assert!(!clock_wanted(&cfg, false));

        cfg.bar.center.clear();
        assert!(!clock_wanted(&cfg, true));

        cfg.bar.left.push("clock".into());
        assert!(clock_wanted(&cfg, true));

        cfg.bar.enabled = false;
        assert!(!clock_wanted(&cfg, true));
    }

    #[test]
    fn next_clock_delay_aligns_to_the_period() {
        assert_eq!(
            next_clock_delay(0, 0, ClockStep::Minute),
            Duration::from_millis(60_020)
        );
        assert_eq!(
            next_clock_delay(59, 500, ClockStep::Minute),
            Duration::from_millis(520)
        );
        assert_eq!(
            next_clock_delay(10, 250, ClockStep::Second),
            Duration::from_millis(770)
        );
        assert_eq!(
            next_clock_delay(10, 999, ClockStep::Second),
            Duration::from_millis(21)
        );
    }

    #[test]
    fn dirty_marks_collapse_into_one_flush() {
        let mut dirty = false;
        for _ in 0..3 {
            dirty = true;
        }
        assert!(take_dirty(&mut dirty));
        assert!(!take_dirty(&mut dirty));
    }

    #[test]
    fn an_answered_error_stops_the_wait_only_for_the_epoch_it_answered() {
        // Fresh service, no answer yet: wait for the attempt.
        assert!(needs_fresh_snapshot(true, None, 7, 3));
        // A connected error answers epoch 3, so the pending launcher maps now.
        assert_eq!(
            launcher_map_decision(true, false, needs_fresh_snapshot(true, Some((7, 3)), 7, 3)),
            LauncherMap::Now
        );
        // The previous epoch's answer must not cover a reconnect: wait again.
        assert_eq!(
            launcher_map_decision(true, false, needs_fresh_snapshot(true, Some((7, 2)), 7, 3)),
            LauncherMap::Wait
        );
        // Disconnected after this generation answered: fall back, no wait.
        assert_eq!(
            launcher_map_decision(true, false, needs_fresh_snapshot(false, Some((7, 2)), 7, 3)),
            LauncherMap::Now
        );
        // Disconnected before the first answer: the worker still owes one.
        assert!(needs_fresh_snapshot(false, None, 7, 3));
        // A new service generation owes its own answer.
        assert!(needs_fresh_snapshot(false, Some((6, 3)), 7, 3));
    }

    #[test]
    fn a_current_epoch_error_maps_the_fallback_but_a_stale_one_is_ignored() {
        // Current connection: the error answers the attempt, so the pending
        // launcher maps now instead of waiting forever.
        assert_eq!(
            snapshot_error_map(3, 2, 3, 2, false),
            Some(LauncherMap::Now)
        );
        // Same generation but the previous connection epoch: stale, and it
        // must not mark the attempt or move the overlay.
        assert_eq!(snapshot_error_map(3, 1, 3, 2, false), None);
        // A late error from the previous service generation is stale too.
        assert_eq!(snapshot_error_map(2, 2, 3, 2, false), None);
    }

    #[test]
    fn launcher_waits_for_a_snapshot_only_while_one_is_owed() {
        // A service started by this open: wait for its first snapshot even
        // before the connection is up, so the first output is not a guess.
        assert_eq!(launcher_map_decision(true, false, true), LauncherMap::Wait);
        // A current snapshot maps immediately.
        assert_eq!(launcher_map_decision(true, true, true), LauncherMap::Now);
        // A failed discovery answers the attempt too, so a dead service
        // cannot stall an open; the same holds after it delivered once and
        // then lost the compositor. Either way: Wayland outputs.
        assert_eq!(launcher_map_decision(true, false, false), LauncherMap::Now);
        // No service at all: nothing is owed.
        assert_eq!(launcher_map_decision(false, false, true), LauncherMap::Now);
    }
}
