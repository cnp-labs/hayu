use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use calloop::channel::Event as ChannelEvent;
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction};
use chrono::{Local, Timelike};
use cosmic_text::{FontSystem, SwashCache};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
    },
    shell::WaylandSurface,
    shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        LayerSurfaceConfigure,
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
};

use crate::config::{self, Config};
use crate::desktop;
use crate::hyprland::{self, Event, Hyprland, Snapshot, Workspace};
use crate::ipc::{self, Command as IpcCommand};
use crate::launcher::{self, Launcher};
use crate::render::{self, Scene, Theme};

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
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm unavailable: {e}"))?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);
    let seat_state = SeatState::new(&globals, &qh);

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
        shm,
        qh: qh.clone(),
        seat_state,
        keyboard: None,
        loop_handle: loop_handle.clone(),
        cfg,
        theme,
        fonts,
        cache: SwashCache::new(),
        hypr: Hyprland::discover(),
        snapshot: None,
        active_workspace: 0,
        title: String::new(),
        clock,
        bars: HashMap::new(),
        launcher: Launcher::new(Vec::new()),
        ipc_path: None,
        loop_signal: Some(event_loop.get_signal()),
    };
    if state.hypr.is_none() {
        eprintln!("hayu: Hyprland IPC not found; bar will show the clock only");
    }
    state.resync();

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

    let timer = Timer::from_duration(duration_to_next_minute());
    loop_handle
        .insert_source(timer, |_, _, state| {
            state.tick();
            TimeoutAction::ToInstant(Instant::now() + duration_to_next_minute())
        })
        .map_err(|e| format!("cannot insert clock timer: {e}"))?;

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
        .run(None, &mut state, |_| {})
        .map_err(|e| format!("event loop failed: {e}"));
    if let Some(path) = &state.ipc_path {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn duration_to_next_minute() -> Duration {
    let now = Local::now();
    let elapsed = now.second() * 1000 + now.timestamp_subsec_millis();
    Duration::from_millis(60_000 - elapsed as u64 + 20)
}

struct Bar {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    pool: SlotPool,
    width: u32,
    height: u32,
    bottom: bool,
    scale: i32,
    configured: bool,
}

pub struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    qh: QueueHandle<State>,
    seat_state: SeatState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    loop_handle: LoopHandle<'static, State>,
    cfg: Config,
    theme: Theme,
    fonts: FontSystem,
    cache: SwashCache,
    hypr: Option<Hyprland>,
    snapshot: Option<Snapshot>,
    active_workspace: i64,
    title: String,
    clock: String,
    bars: HashMap<u32, Bar>,
    launcher: Launcher,
    ipc_path: Option<PathBuf>,
    loop_signal: Option<LoopSignal>,
}

impl State {
    fn tick(&mut self) {
        let clock = render::format_clock(&self.cfg.clock.format, Local::now().time());
        if clock != self.clock {
            self.clock = clock;
            self.draw_all();
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
                self.draw_all();
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
                self.draw_all();
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
                self.draw_all();
            }
            Event::DestroyWorkspace { name } => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.workspaces.retain(|w| w.name != name);
                }
                self.draw_all();
            }
            Event::ActiveWindow { title } => {
                self.title = title;
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.active_window.title = self.title.clone();
                }
                self.draw_all();
            }
            Event::FocusMonitor { monitor } => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    for m in &mut snapshot.monitors {
                        m.focused = m.name == monitor;
                    }
                }
                self.draw_all();
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
                    IpcCommand::LauncherToggle => self.toggle_launcher(),
                    IpcCommand::LauncherOpen => self.open_launcher(),
                    IpcCommand::LauncherClose => {
                        self.close_launcher();
                        Ok(())
                    }
                    IpcCommand::Quit => Ok(()),
                    IpcCommand::Reload => self.reload_config(),
                };
                match result {
                    Ok(()) => (ipc::Response::ok(id), command == IpcCommand::Quit),
                    Err(error) => (ipc::Response::error(id, error), false),
                }
            }
            Err(error) => (ipc::Response::error(id, error), false),
        };
        if let Err(error) = ipc::write_response(&mut stream, &response) {
            eprintln!("hayu: cannot answer control request: {error}");
        }
        if quit && let Some(signal) = &self.loop_signal {
            signal.stop();
        }
    }

    /// Re-read the config file. A parse or validation error keeps the running
    /// state untouched, so a bad edit cannot take the shell down.
    fn reload_config(&mut self) -> Result<(), String> {
        let path = config::default_path();
        let cfg = if path.exists() {
            config::load(&path).map_err(|error| format!("{}: {error}", path.display()))?
        } else {
            config::Config::default()
        };
        let theme = Theme::from_config(&cfg.appearance.mode, &cfg.appearance.accent);
        self.cfg = cfg;
        self.theme = theme;
        self.clock = render::format_clock(&self.cfg.clock.format, Local::now().time());
        if !self.cfg.launcher.enabled && self.launcher.is_open() {
            self.close_launcher();
        }
        self.apply_bar_geometry();
        self.draw_all();
        Ok(())
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
            .unwrap_or(1);
        // Refresh on every open so an application installed since startup can
        // be launched without restarting the shell.
        self.launcher.set_entries(desktop::scan());
        self.launcher.begin_open();
        if self.launcher.surface.is_none() {
            match launcher::Surface::create(
                &self.compositor,
                &self.layer_shell,
                &self.shm,
                &self.qh,
                &output,
                scale,
            ) {
                Ok(surface) => self.launcher.show(surface),
                Err(error) => {
                    self.launcher.close();
                    return Err(error);
                }
            }
        }
        // The draw happens on the configure event for this open; painting here
        // would attach a buffer before the compositor reconfigured the surface.
        Ok(())
    }

    fn close_launcher(&mut self) {
        self.launcher.close();
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
        self.bars.values().next().map(|bar| bar.output.clone())
    }

    fn handle_launcher_key(&mut self, event: KeyEvent) {
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            let (query, selected, results) = self.launcher.debug_state();
            eprintln!(
                "hayu: key keysym=0x{:x} utf8={:?} open={} before query={query:?} sel={selected} results={results}",
                event.keysym.raw(),
                event.utf8,
                self.launcher.is_open(),
            );
        }
        let action = self.launcher.key(&event);
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            let (query, selected, results) = self.launcher.debug_state();
            eprintln!(
                "hayu: key -> {action:?} query={query:?} sel={selected} results={results} top={:?}",
                self.launcher.debug_results()
            );
        }
        match action {
            launcher::Action::None => {}
            launcher::Action::Changed => self.draw_launcher(),
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
            .draw(&self.theme, &mut self.fonts, &mut self.cache);
        if std::env::var_os("HAYU_DEBUG_KEYS").is_some() {
            eprintln!(
                "hayu: launcher frame {:.2} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
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
        let pool = match SlotPool::new(1, &self.shm) {
            Ok(pool) => pool,
            Err(err) => {
                eprintln!("hayu: cannot create shm pool: {err}");
                return;
            }
        };
        self.bars.insert(
            output.id().protocol_id(),
            Bar {
                output,
                layer,
                pool,
                width: scale as u32 * 256,
                height,
                bottom: self.cfg.bar.position == "bottom",
                scale,
                configured: false,
            },
        );
    }

    fn draw_all(&mut self) {
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
            draw_bar(
                bar,
                &self.theme,
                &self.clock,
                self.active_workspace,
                &workspaces,
                &title,
                &mut self.fonts,
                &mut self.cache,
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

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    bar: &mut Bar,
    theme: &Theme,
    clock: &str,
    active_workspace: i64,
    workspaces: &[Workspace],
    title: &str,
    fonts: &mut FontSystem,
    cache: &mut SwashCache,
) {
    if !bar.configured || bar.width == 0 || bar.height == 0 {
        return;
    }
    let scale = bar.scale.max(1);
    let width = bar.width.saturating_mul(scale as u32);
    let height = bar.height.saturating_mul(scale as u32);
    let scene = Scene {
        width,
        height,
        scale: scale as f32,
        theme,
        clock,
        title,
        workspaces,
        active_workspace,
    };
    let Some(pixmap) = render::draw(&scene, fonts, cache) else {
        return;
    };
    let stride = (width * 4) as i32;
    let (buffer, canvas) = match bar.pool.create_buffer(
        width as i32,
        height as i32,
        stride,
        wl_shm::Format::Argb8888,
    ) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("hayu: cannot create shm buffer: {err}");
            return;
        }
    };
    render::copy_to_shm(&pixmap, canvas);
    let surface = bar.layer.wl_surface();
    let _ = bar.layer.set_buffer_scale(scale as u32);
    surface.damage_buffer(0, 0, width as i32, height as i32);
    if let Err(err) = buffer.attach_to(surface) {
        eprintln!("hayu: cannot attach buffer: {err}");
        return;
    }
    bar.layer.commit();
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
            && self.launcher.set_scale(factor)
        {
            self.draw_launcher();
        }
        if let Some(bar) = self
            .bars
            .values_mut()
            .find(|bar| bar.layer.wl_surface() == surface)
        {
            bar.scale = factor.max(1);
        }
        self.draw_all();
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
        self.create_bar(output);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if self.launcher.output_id() == Some(output.id().protocol_id())
                && self.launcher.set_scale(info.scale_factor)
            {
                self.draw_launcher();
            }
            if let Some(bar) = self.bars.get_mut(&output.id().protocol_id()) {
                bar.scale = info.scale_factor.max(1);
            }
        }
        self.draw_all();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let id = output.id().protocol_id();
        if self.launcher.output_id() == Some(id) {
            self.launcher.forget_surface();
        }
        self.bars.remove(&id);
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.launcher.on_layer_closed(layer) {
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
        self.draw_all();
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
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

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
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
