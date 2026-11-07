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
use crate::hyprland::{self, Event, Hyprland, Snapshot, Workspace};
use crate::ipc::{self, Command as IpcCommand};
use crate::launcher::{self, Launcher};
use crate::render::{self, Theme};
use crate::tray;

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

    let (tray_tx, tray_rx) = calloop::channel::channel::<tray::Event>();
    if cfg.tray.enabled {
        tray::spawn(tray_tx);
    }
    let theme = Theme::from_config(&cfg.appearance.mode, &cfg.appearance.accent);
    let mut fonts = FontSystem::new();
    if fonts.db_mut().is_empty() {
        eprintln!("hayu: no system fonts found; text will not be drawn");
    }
    let clock = render::format_clock(&cfg.clock.format, Local::now().time());

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
        hypr: Hyprland::discover(),
        snapshot: None,
        active_workspace: 0,
        title: String::new(),
        clock,
        bars: HashMap::new(),
        bars_dirty: false,
        clock_timer: None,
        tray_items: Vec::new(),
        tray_version: 0,
        launcher: Launcher::new(Vec::new()),
        a11y: A11y::new(),
        ipc_path: None,
        loop_signal: Some(event_loop.get_signal()),
    };
    if state.hypr.is_none() {
        eprintln!("hayu: Hyprland IPC not found; bar will show the clock only");
    }
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

    state.clock_timer = schedule_clock(&mut state)?;

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

    let (tx, rx) = calloop::channel::channel::<Event>();
    loop_handle
        .insert_source(rx, |event, _, state| {
            if let ChannelEvent::Msg(event) = event {
                state.handle_hypr_event(event);
            }
        })
        .map_err(|e| format!("cannot insert Hyprland channel: {e}"))?;
    hyprland::spawn_event_thread(tx);

    loop_handle
        .insert_source(tray_rx, |event, _, state| {
            if let ChannelEvent::Msg(event) = event {
                state.handle_tray_event(event);
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
fn clock_wanted(cfg: &Config) -> bool {
    cfg.bar.enabled
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
    if !clock_wanted(&state.cfg) {
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
    hypr: Option<Hyprland>,
    snapshot: Option<Snapshot>,
    active_workspace: i64,
    title: String,
    clock: String,
    bars: HashMap<u32, Bar>,
    /// Set by any state change that affects the bar; one flush per loop cycle.
    bars_dirty: bool,
    clock_timer: Option<RegistrationToken>,
    tray_items: Vec<tray::TrayItem>,
    tray_version: u32,
    launcher: Launcher,
    a11y: A11y,
    ipc_path: Option<PathBuf>,
    loop_signal: Option<LoopSignal>,
}

impl State {
    fn handle_tray_event(&mut self, event: tray::Event) {
        match event {
            tray::Event::Items(items) => {
                self.tray_items = items;
                self.tray_version = self.tray_version.wrapping_add(1);
                self.invalidate_bars();
            }
            tray::Event::Disabled(reason) => {
                eprintln!("hayu: {reason}");
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

    /// Called once per event-loop cycle: many capturers collapse into one draw.
    fn flush_invalidated(&mut self) {
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

    fn resync(&mut self) {
        if self.hypr.is_none() {
            self.hypr = Hyprland::discover();
        }
        let Some(hypr) = &self.hypr else { return };
        match hypr.snapshot() {
            Ok(snapshot) => {
                self.active_workspace = snapshot.active_workspace.id;
                self.title = snapshot.active_window.title.clone();
                self.snapshot = Some(snapshot);
                self.invalidate_bars();
            }
            Err(err) => eprintln!("hayu: Hyprland snapshot failed: {err}"),
        }
    }

    fn handle_hypr_event(&mut self, event: Event) {
        match event {
            Event::Workspace { id, name } => {
                self.active_workspace = id;
                if let (Some(name), Some(snapshot)) = (name, self.snapshot.as_mut()) {
                    if let Some(ws) = snapshot.workspaces.iter_mut().find(|w| w.name == name) {
                        ws.id = id;
                    } else {
                        snapshot.workspaces.push(Workspace {
                            id,
                            name,
                            ..Workspace::default()
                        });
                    }
                }
                self.invalidate_bars();
            }
            Event::CreateWorkspace { id, name } => {
                if let Some(snapshot) = self.snapshot.as_mut()
                    && !snapshot.workspaces.iter().any(|w| w.name == name)
                {
                    snapshot.workspaces.push(Workspace {
                        id,
                        name,
                        ..Workspace::default()
                    });
                }
                self.invalidate_bars();
            }
            Event::DestroyWorkspace { name } => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.workspaces.retain(|w| w.name != name);
                }
                self.invalidate_bars();
            }
            Event::ActiveWindow { title } => {
                self.title = title;
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.active_window.title = self.title.clone();
                }
                self.invalidate_bars();
            }
            Event::FocusMonitor { monitor } => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    for m in &mut snapshot.monitors {
                        m.focused = m.name == monitor;
                    }
                }
                self.invalidate_bars();
            }
            Event::Refresh | Event::Resync => self.resync(),
            Event::Disconnected => {
                eprintln!("hayu: Hyprland event socket disconnected; reconnecting");
            }
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
        if !self.cfg.launcher.enabled && self.launcher.is_open() {
            self.close_launcher();
        }
        self.sync_bars();
        self.apply_bar_geometry();
        self.invalidate_bars();
        Ok((!restart.is_empty()).then(|| format!("restart required for: {}", restart.join(", "))))
    }

    /// Height and edge are layer-shell state, so a change needs a new configure
    /// before the next buffer. Outputs and widget lists still need a restart.
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
        let output = self
            .launcher_output()
            .ok_or_else(|| "no output is available for the launcher".to_string())?;
        let scale = self
            .output_state
            .info(&output)
            .map(|info| info.scale_factor)
            .unwrap_or(1) as f32;
        // Refresh on every open so an application installed since startup can
        // be launched without restarting the shell.
        self.launcher.set_entries(desktop::scan());
        self.launcher.begin_open();
        if self.launcher.surface.is_none() {
            match launcher::Surface::create(
                &self.compositor,
                &self.layer_shell,
                &self.qh,
                &output,
                scale,
                self.fractional.as_ref(),
                self.viewporter.as_ref(),
            ) {
                Ok(surface) => self.launcher.show(surface),
                Err(error) => {
                    self.close_launcher();
                    return Err(error);
                }
            }
        }
        // The draw happens on the configure event for this open; painting here
        // would attach a buffer before the compositor reconfigured the surface.
        Ok(())
    }

    fn close_launcher(&mut self) {
        self.ime_disable();
        self.launcher.close();
        self.sync_a11y();
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
            if let Some(bar) = self.bars.remove(&key) {
                // The layer role must be destroyed before its surface.
                let surface = bar.layer.wl_surface().clone();
                drop(bar);
                surface.destroy();
            }
        }
    }

    fn launcher_output(&self) -> Option<wl_output::WlOutput> {
        let focused = self.snapshot.as_ref().and_then(|snapshot| {
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
        self.a11y.sync(&self.launcher);
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
            self.launcher.forget_surface();
            self.sync_a11y();
        }
        self.bars.remove(&id);
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.launcher.on_layer_closed(layer) {
            self.ime_disable();
            self.sync_a11y();
            return;
        }
        let key = self
            .bars
            .iter()
            .find(|(_, bar)| bar.layer.wl_surface() == layer.wl_surface())
            .map(|(key, _)| *key);
        if let Some(key) = key {
            self.bars.remove(&key);
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
            tray::activate(&item);
            return;
        }
        let Some(id) = render::pill_at(&bar.pills, logical_x, scale) else {
            return;
        };
        if id == self.active_workspace {
            return;
        }
        let Some(hypr) = &self.hypr else { return };
        if let Err(error) = hypr.switch_workspace(id) {
            eprintln!("hayu: cannot switch to workspace {id}: {error}");
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
        assert!(clock_wanted(&cfg));

        cfg.bar.center.clear();
        assert!(!clock_wanted(&cfg));

        cfg.bar.left.push("clock".into());
        assert!(clock_wanted(&cfg));

        cfg.bar.enabled = false;
        assert!(!clock_wanted(&cfg));
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
}
