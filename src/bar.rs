//! Bar surfaces: one layer-shell surface per output and its drawing.
//!
//! Lifecycle, input and services live in [`crate::app`]. The fractional-scale
//! helpers here are shared with the launcher surface.

use std::os::raw::c_void;

use cosmic_text::FontSystem;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use wayland_client::{
    Proxy,
    protocol::{wl_output, wl_surface},
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::config::Config;
use crate::hyprland::Workspace;
use crate::render::{self, Atlas, Scene, Theme, gl::GlPresenter};

/// Marker user data for globals whose objects emit no events we handle.
pub(crate) struct NoEvents;

/// Which surface a fractional-scale object belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScaleTarget {
    Bar(u32),
    Launcher,
}

/// Pixel size of a logical length at a possibly fractional scale.
pub(crate) fn pixel_size(logical: u32, scale: f32) -> u32 {
    ((logical as f32) * scale).round().max(1.0) as u32
}

/// Buffer scale and viewport destination for the next commit. With fractional
/// scale the buffer is presented at integer scale 1 and the viewport maps it
/// onto the logical size; otherwise the integer buffer scale is used.
pub(crate) fn apply_scale(
    surface: &wl_surface::WlSurface,
    viewport: Option<&WpViewport>,
    logical_width: u32,
    logical_height: u32,
    scale: f32,
) {
    match viewport {
        Some(viewport) => {
            surface.set_buffer_scale(1);
            viewport.set_destination(logical_width as i32, logical_height as i32);
        }
        None => {
            surface.set_buffer_scale(scale.round().max(1.0) as i32);
        }
    }
}

/// One bar surface, its GL presenter and the hit geometry from the last frame.
pub(crate) struct Bar {
    pub(crate) output: wl_output::WlOutput,
    // Must drop before `layer`: the EGL surface wraps `layer`'s wl_surface.
    presenter: Option<GlPresenter>,
    atlas: Atlas,
    pub(crate) layer: LayerSurface,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bottom: bool,
    /// Pixels per logical unit: the fractional preferred scale when the
    /// compositor offers it, otherwise the output's integer scale.
    pub(crate) scale: f32,
    pub(crate) fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    pub(crate) configured: bool,
    /// Workspace pills as last drawn, so a click uses the same geometry.
    pub(crate) pills: Vec<render::Pill>,
    /// Tray icons as last drawn, with the source item index.
    pub(crate) tray_hits: Vec<render::TrayHit>,
}

impl Drop for Bar {
    fn drop(&mut self) {
        // Destroy the surface children first; `presenter` and `layer` still
        // drop in field order, and the app destroys the native wl_surface
        // after the layer role is gone.
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}
impl Bar {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        output: wl_output::WlOutput,
        layer: LayerSurface,
        scale: f32,
        width: u32,
        height: u32,
        bottom: bool,
        fractional: Option<WpFractionalScaleV1>,
        viewport: Option<WpViewport>,
    ) -> Self {
        Self {
            output,
            presenter: None,
            atlas: Atlas::new(),
            layer,
            width,
            height,
            bottom,
            scale,
            fractional,
            viewport,
            configured: false,
            pills: Vec::new(),
            tray_hits: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        cfg: &Config,
        theme: &Theme,
        clock: &str,
        active_workspace: i64,
        workspaces: &[Workspace],
        title: &str,
        tray: &[render::TrayImage],
        tray_version: u32,
        fonts: &mut FontSystem,
        display_ptr: *mut c_void,
    ) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }
        let scale = self.scale.max(1.0);
        let width = pixel_size(self.width, scale);
        let height = pixel_size(self.height, scale);
        let left: Vec<&str> = cfg.bar.left.iter().map(String::as_str).collect();
        let center: Vec<&str> = cfg.bar.center.iter().map(String::as_str).collect();
        let right: Vec<&str> = cfg.bar.right.iter().map(String::as_str).collect();
        let scene = Scene {
            width,
            height,
            scale,
            theme,
            clock,
            title,
            workspaces,
            active_workspace,
            left: &left,
            center: &center,
            right: &right,
            tray,
            tray_version,
        };
        let Some(frame) = render::build(&scene, fonts, &mut self.atlas) else {
            return;
        };
        if std::env::var_os("HAYU_DEBUG_POINTER").is_some() {
            let pills: Vec<String> = frame
                .pills
                .iter()
                .map(|pill| format!("{}:{:.1}:{:.1}", pill.id, pill.x / scale, pill.w / scale))
                .collect();
            eprintln!("hayu: bar pills {} ({})", pills.join(" "), self.width);
        }
        if std::env::var_os("HAYU_DEBUG_POINTER").is_some() && !frame.tray_hits.is_empty() {
            let icons: Vec<String> = frame
                .tray_hits
                .iter()
                .map(|hit| format!("{}:{:.1}:{:.1}", hit.id, hit.x / scale, hit.w / scale))
                .collect();
            eprintln!("hayu: bar tray {}", icons.join(" "));
        }
        self.pills = frame.pills.clone();
        self.tray_hits = frame.tray_hits.clone();
        let surface = self.layer.wl_surface();
        apply_scale(
            surface,
            self.viewport.as_ref(),
            self.width,
            self.height,
            scale,
        );
        if self.presenter.is_none() {
            let surface_ptr = surface.id().as_ptr() as *mut c_void;
            match GlPresenter::new(display_ptr, surface_ptr, width, height) {
                Ok(presenter) => self.presenter = Some(presenter),
                Err(err) => {
                    eprintln!("hayu: cannot create bar GL surface: {err}");
                    return;
                }
            }
        }
        if let Some(presenter) = self.presenter.as_mut() {
            if let Err(err) = presenter.render(&frame, &self.atlas) {
                eprintln!("hayu: bar render failed: {err}");
            }
        }
    }
}
