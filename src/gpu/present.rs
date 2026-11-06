use std::collections::HashMap;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopSignal};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::{wl_output, wl_surface};
use wayland_client::{globals::registry_queue_init, Connection, Proxy, QueueHandle};

use super::scene::{Scene, LOGICAL_HEIGHT, LOGICAL_WIDTH};
use super::{Backend, Error, Result};

static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn request_stop(_signal: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}
fn install_stop_handler() {
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, request_stop as extern "C" fn(_) as usize);
        libc::signal(libc::SIGINT, request_stop as extern "C" fn(_) as usize);
    }
}
pub trait Presenter {
    fn render(&mut self, scene: &Scene) -> Result<()>;
    fn renderer_name(&self) -> String;
    fn resize(&mut self, _width: u32, _height: u32) -> Result<()> {
        Ok(())
    }
}

pub struct PresentOutput {
    pub startup_ms: f64,
    pub times_ms: Vec<f64>,
    pub width: u32,
    pub height: u32,
    pub renderer: String,
}

struct Bar {
    // Declared before `layer` so the GPU surface is destroyed while the
    // wl_surface still exists. Reversing this crashed RADV on teardown.
    presenter: Option<Box<dyn Presenter>>,
    _output: wl_output::WlOutput,
    layer: LayerSurface,
    width: u32,
    height: u32,
    scale: u32,
    configured: bool,
    failed: Option<String>,
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    qh: QueueHandle<State>,
    display_ptr: *mut c_void,
    backend: Backend,
    scene: Scene,
    bars: HashMap<u32, Bar>,
    fatal: Option<String>,
    loop_signal: Option<LoopSignal>,
}

pub fn run(
    backend: Backend,
    frames: u32,
    hold: f64,
    scene: Scene,
    start: Instant,
) -> Result<PresentOutput> {
    if backend == Backend::Cpu {
        return Err(Error::Usage(
            "cpu has no present backend in this spike; use `hayu run` or --mode offscreen".into(),
        ));
    }
    let conn = Connection::connect_to_env()
        .map_err(|err| Error::Wayland(format!("cannot connect to Wayland: {err}")))?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&conn)
        .map_err(|err| Error::Wayland(format!("cannot enumerate Wayland globals: {err}")))?;
    let qh = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|err| Error::Wayland(format!("wl_compositor unavailable: {err}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|err| Error::Wayland(format!("wlr-layer-shell unavailable: {err}")))?;
    let output_state = OutputState::new(&globals, &qh);
    let registry_state = RegistryState::new(&globals);
    let display_ptr = conn.display().id().as_ptr() as *mut c_void;

    let mut state = State {
        registry_state,
        output_state,
        compositor,
        layer_shell,
        qh,
        display_ptr,
        backend,
        scene,
        bars: HashMap::new(),
        fatal: None,
        loop_signal: None,
    };

    for _ in 0..3 {
        event_queue
            .roundtrip(&mut state)
            .map_err(|err| Error::Wayland(format!("roundtrip failed: {err}")))?;
        if state.bars.values().any(|bar| bar.presenter.is_some()) {
            break;
        }
    }
    if let Some(err) = state.fatal.take() {
        return Err(Error::Wayland(err));
    }
    if state.bars.is_empty() {
        return Err(Error::Wayland("no outputs discovered".into()));
    }
    if !state.bars.values().any(|bar| bar.presenter.is_some()) {
        let detail = state
            .bars
            .values()
            .filter_map(|bar| bar.failed.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Error::Wayland(format!(
            "layer surface was not configured{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(" ({detail})")
            }
        )));
    }

    install_stop_handler();
    let mut times = Vec::with_capacity(frames as usize);
    let mut startup = None;
    for _ in 0..frames {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        let frame_start = Instant::now();
        let mut presented = false;
        {
            let scene = &state.scene;
            for bar in state.bars.values_mut() {
                if let Some(presenter) = bar.presenter.as_mut() {
                    presenter.render(scene)?;
                    presented = true;
                }
            }
        }
        if !presented {
            return Err(Error::Wayland("no presenter is ready".into()));
        }
        if startup.is_none() {
            startup = Some(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.push(frame_start.elapsed().as_secs_f64() * 1000.0);
        event_queue
            .dispatch_pending(&mut state)
            .map_err(|err| Error::Wayland(format!("dispatch failed: {err}")))?;
        if let Some(err) = state.fatal.take() {
            return Err(Error::Wayland(err));
        }
    }

    let renderer = state
        .bars
        .values()
        .find_map(|bar| bar.presenter.as_ref().map(|p| p.renderer_name()))
        .unwrap_or_else(|| backend.name().to_string());

    if hold > 0.0 {
        let mut event_loop: EventLoop<State> = EventLoop::try_new()
            .map_err(|err| Error::Wayland(format!("cannot create event loop: {err}")))?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn, event_queue)
            .insert(loop_handle.clone())
            .map_err(|err| Error::Wayland(format!("cannot insert Wayland source: {err}")))?;
        let signals = calloop::signals::Signals::new(&[
            calloop::signals::Signal::SIGINT,
            calloop::signals::Signal::SIGTERM,
        ])
        .map_err(|err| Error::Wayland(format!("cannot install signal handler: {err}")))?;
        loop_handle
            .insert_source(signals, |_, _, state: &mut State| {
                if let Some(signal) = &state.loop_signal {
                    signal.stop();
                }
            })
            .map_err(|err| Error::Wayland(format!("cannot insert signal source: {err}")))?;
        let timer = Timer::from_duration(Duration::from_secs_f64(hold));
        loop_handle
            .insert_source(timer, |_, _, state: &mut State| {
                if let Some(signal) = &state.loop_signal {
                    signal.stop();
                }
                TimeoutAction::Drop
            })
            .map_err(|err| Error::Wayland(format!("cannot insert hold timer: {err}")))?;
        state.loop_signal = Some(event_loop.get_signal());
        event_loop
            .run(None, &mut state, |_| {})
            .map_err(|err| Error::Wayland(format!("event loop failed: {err}")))?;
        // Tear down GPU surfaces while the Wayland connection is still alive;
        // destroying a Vulkan swapchain after the connection is gone crashes RADV.
        state.bars.clear();
    } else {
        state.bars.clear();
    }

    Ok(PresentOutput {
        startup_ms: startup.unwrap_or(0.0),
        times_ms: times,
        width: state.scene.width,
        height: state.scene.height,
        renderer,
    })
}

impl State {
    fn create_bar(&mut self, output: wl_output::WlOutput) {
        let scale = self.scene.scale;
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Top,
            Some("hayu-gpu-spike"),
            Some(&output),
        );
        layer.set_anchor(Anchor::LEFT | Anchor::RIGHT | Anchor::TOP);
        layer.set_size(0, LOGICAL_HEIGHT);
        layer.set_exclusive_zone(LOGICAL_HEIGHT as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        if let Err(err) = layer.set_buffer_scale(scale) {
            eprintln!("hayu-gpu-spike: buffer scale: {err:?}");
        }
        layer.commit();
        self.bars.insert(
            output.id().protocol_id(),
            Bar {
                _output: output,
                layer,
                width: LOGICAL_WIDTH,
                height: LOGICAL_HEIGHT,
                scale,
                configured: false,
                presenter: None,
                failed: None,
            },
        );
    }

    fn configure_bar(&mut self, key: u32, width: u32, height: u32) {
        let Some(bar) = self.bars.get_mut(&key) else {
            return;
        };
        bar.width = if width == 0 {
            self.scene.width / bar.scale
        } else {
            width
        };
        bar.height = if height == 0 { LOGICAL_HEIGHT } else { height };
        bar.configured = true;
        let physical_width = bar.width * bar.scale;
        let physical_height = bar.height * bar.scale;
        if let Some(presenter) = bar.presenter.as_mut() {
            if let Err(err) = presenter.resize(physical_width, physical_height) {
                self.fatal = Some(format!("resize failed: {err}"));
            }
            return;
        }
        let surface_ptr = bar.layer.wl_surface().id().as_ptr() as *mut c_void;
        match create_presenter(
            self.backend,
            self.display_ptr,
            surface_ptr,
            physical_width,
            physical_height,
            &self.scene,
        ) {
            Ok(presenter) => bar.presenter = Some(presenter),
            Err(err) => {
                eprintln!("hayu-gpu-spike: presenter: {err}");
                self.fatal = Some(err.to_string());
                bar.failed = Some(err.to_string());
            }
        }
    }
}

fn create_presenter(
    backend: Backend,
    display_ptr: *mut c_void,
    surface_ptr: *mut c_void,
    width: u32,
    height: u32,
    scene: &Scene,
) -> Result<Box<dyn Presenter>> {
    match backend {
        Backend::Gl => Ok(Box::new(super::gl::GlPresenter::new(
            display_ptr,
            surface_ptr,
            width,
            height,
            scene,
        )?)),
        Backend::Wgpu => {
            #[cfg(feature = "wgpu")]
            {
                Ok(Box::new(super::wgpu::WgpuPresenter::new(
                    display_ptr,
                    surface_ptr,
                    width,
                    height,
                    scene,
                )?))
            }
            #[cfg(not(feature = "wgpu"))]
            {
                let _ = (display_ptr, surface_ptr, width, height, scene);
                Err(Error::Usage(
                    "this binary was built without the wgpu feature".into(),
                ))
            }
        }
        Backend::Cpu => Err(Error::Usage("cpu cannot present".into())),
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
        if factor <= 0 || factor as u32 == self.scene.scale {
            return;
        }
        if !self
            .bars
            .values()
            .any(|bar| bar.layer.wl_surface() == surface)
        {
            return;
        }
        let new_scale = factor as u32;
        let scene = match Scene::build(new_scale) {
            Ok(scene) => scene,
            Err(err) => {
                self.fatal = Some(format!("scene rebuild for scale {new_scale}: {err}"));
                return;
            }
        };
        self.scene = scene;
        let keys: Vec<u32> = self.bars.keys().copied().collect();
        for key in keys {
            let Some(mut bar) = self.bars.remove(&key) else {
                continue;
            };
            bar.scale = new_scale;
            let _ = bar.layer.set_buffer_scale(new_scale);
            bar.presenter = None;
            if bar.configured {
                let surface_ptr = bar.layer.wl_surface().id().as_ptr() as *mut c_void;
                let width = bar.width * new_scale;
                let height = bar.height * new_scale;
                match create_presenter(
                    self.backend,
                    self.display_ptr,
                    surface_ptr,
                    width,
                    height,
                    &self.scene,
                ) {
                    Ok(presenter) => bar.presenter = Some(presenter),
                    Err(err) => self.fatal = Some(format!("scale change: {err}")),
                }
            }
            self.bars.insert(key, bar);
        }
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
        _output: wl_output::WlOutput,
    ) {
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
        let key = self
            .bars
            .iter()
            .find(|(_, bar)| bar.layer.wl_surface() == layer.wl_surface())
            .map(|(key, _)| *key);
        if let Some(key) = key {
            self.configure_bar(key, configure.new_size.0, configure.new_size.1);
        }
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
