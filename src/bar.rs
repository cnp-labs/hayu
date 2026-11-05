use std::collections::HashMap;
use std::time::{Duration, Instant};

use calloop::channel::Event as ChannelEvent;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopSignal};
use chrono::{Local, Timelike};
use cosmic_text::{FontSystem, SwashCache};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
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
    protocol::{wl_output, wl_shm, wl_surface},
};

use crate::config::Config;
use crate::hyprland::{self, Event, Hyprland, Snapshot, Workspace};
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

    let theme = Theme::from_config(&cfg.appearance.mode, &cfg.appearance.accent);
    let mut fonts = FontSystem::new();
    if fonts.db_mut().is_empty() {
        eprintln!("hayu: no system fonts found; text will not be drawn");
    }
    let clock = render::format_clock(&cfg.clock.format, Local::now().time());

    let mut state = State {
        registry_state,
        output_state,
        compositor,
        layer_shell,
        shm,
        qh: qh.clone(),
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
        loop_signal: None,
    };
    if state.hypr.is_none() {
        eprintln!("hayu: Hyprland IPC not found; bar will show the clock only");
    }
    state.resync();

    // Populate outputs (and create their bars) before entering the event loop.
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("Wayland roundtrip failed: {e}"))?;

    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().map_err(|e| format!("cannot create event loop: {e}"))?;
    let loop_handle = event_loop.handle();
    state.loop_signal = Some(event_loop.get_signal());

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

    event_loop
        .run(None, &mut state, |_| {})
        .map_err(|e| format!("event loop failed: {e}"))?;
    Ok(())
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
        if let Some(info) = self.output_state.info(&output)
            && let Some(bar) = self.bars.get_mut(&output.id().protocol_id())
        {
            bar.scale = info.scale_factor.max(1);
        }
        self.draw_all();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.bars.remove(&output.id().protocol_id());
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
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
