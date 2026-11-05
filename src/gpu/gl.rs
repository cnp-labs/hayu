#![cfg(feature = "gl")]

use std::num::NonZeroU32;
use std::os::raw::c_void;
use std::ptr::NonNull;
use std::time::Instant;

use glow::HasContext;
use glutin::config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, NotCurrentContext, NotCurrentGlContext,
    PossiblyCurrentContext, PossiblyCurrentGlContext, Version,
};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{
    GlSurface, PbufferSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface,
};
use wayland_client::Proxy;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

use super::present::Presenter;
use super::scene::Scene;
use super::{Error, OffscreenOutput, Result};

fn glutin_err(context: &str, err: impl std::fmt::Display) -> Error {
    Error::Gl(format!("{context}: {err}"))
}

fn display_from_raw(display_ptr: *mut c_void) -> Result<Display> {
    let handle = NonNull::new(display_ptr)
        .map(|ptr| WaylandDisplayHandle::new(ptr))
        .ok_or_else(|| Error::Gl("Wayland display pointer is null".into()))?;
    // SAFETY: the caller guarantees the wl_display outlives every GL object
    // created from this Display (the Wayland connection lives for the run).
    unsafe {
        Display::new(
            RawDisplayHandle::Wayland(handle),
            DisplayApiPreference::Egl,
        )
    }
    .map_err(|err| glutin_err("EGL display", err))
}

fn choose_config(display: &Display, surface_type: ConfigSurfaceTypes, srgb: bool) -> Result<Config> {
    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_surface_type(surface_type)
        .build();
    // SAFETY: template contains no raw handles; only the display is queried.
    let configs = unsafe { display.find_configs(template) }
        .map_err(|err| glutin_err("EGL configs", err))?;
    let mut fallback = None;
    for config in configs {
        if srgb && !config.srgb_capable() {
            continue;
        }
        if config.num_samples() == 0 {
            return Ok(config);
        }
        fallback = Some(config);
    }
    fallback.ok_or_else(|| {
        Error::Gl(format!(
            "no EGL config for {:?}{}",
            surface_type,
            if srgb { " with sRGB" } else { "" }
        ))
    })
}

fn create_context(display: &Display, config: &Config) -> Result<NotCurrentContext> {
    let attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(None);
    // SAFETY: config comes from this display; no window handle is required for
    // an EGL context that will be bound to a pbuffer or window surface below.
    unsafe { display.create_context(config, &attributes) }
        .map_err(|err| glutin_err("EGL context", err))
}

fn load_gl(display: &Display) -> Result<glow::Context> {
    // SAFETY: a GLES 3.0 context is current, which is what glow requires to
    // resolve entry points; the returned context stores only function pointers.
    Ok(unsafe {
        glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name))
    })
}

pub struct GlRenderer {
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: glow::Texture,
    image: glow::Texture,
    mode_uniform: Option<glow::UniformLocation>,
    texture_uniform: Option<glow::UniformLocation>,
    solid: (u32, u32),
    glyphs: (u32, u32),
    image_range: (u32, u32),
    info: String,
}

impl GlRenderer {
    fn new(gl: glow::Context, scene: &Scene) -> Result<Self> {
        let info = format!(
            "{} / {}",
            unsafe { gl.get_parameter_string(glow::RENDERER) },
            unsafe { gl.get_parameter_string(glow::VERSION) }
        );
        let vertex_source = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec3 a_color;
out vec2 v_uv;
out vec3 v_color;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;
        let fragment_source = r#"#version 300 es
precision mediump float;
in vec2 v_uv;
in vec3 v_color;
uniform sampler2D u_tex;
uniform int u_mode;
out vec4 out_color;
void main() {
    if (u_mode == 0) {
        out_color = vec4(v_color, 1.0);
    } else if (u_mode == 1) {
        out_color = vec4(v_color, texture(u_tex, v_uv).a);
    } else {
        out_color = texture(u_tex, v_uv);
    }
}
"#;
        unsafe {
            let program = gl.create_program().map_err(|err| Error::Gl(err))?;
            let vertex = compile_shader(&gl, glow::VERTEX_SHADER, vertex_source)?;
            let fragment = compile_shader(&gl, glow::FRAGMENT_SHADER, fragment_source)?;
            gl.attach_shader(program, vertex);
            gl.attach_shader(program, fragment);
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                return Err(Error::Gl(format!("program link failed: {log}")));
            }
            gl.detach_shader(program, vertex);
            gl.detach_shader(program, fragment);
            gl.delete_shader(vertex);
            gl.delete_shader(fragment);

            let vao = gl
                .create_vertex_array()
                .map_err(|err| Error::Gl(err))?;
            let vbo = gl.create_buffer().map_err(|err| Error::Gl(err))?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let mesh = scene.mesh();
            let mut bytes = Vec::with_capacity(mesh.vertices.len() * 28);
            for vertex in &mesh.vertices {
                vertex.write_bytes(&mut bytes);
            }
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 28, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, 28, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, 28, 16);
            gl.bind_vertex_array(None);

            let atlas = upload_texture(
                &gl,
                scene.atlas.width,
                scene.atlas.height,
                glow::RGBA8 as i32,
                &scene.atlas.pixels,
            )?;
            let image = upload_texture(
                &gl,
                super::scene::IMAGE_SIZE,
                super::scene::IMAGE_SIZE,
                glow::SRGB8_ALPHA8 as i32,
                &scene.image,
            )?;

            gl.use_program(Some(program));
            let mode_uniform = gl.get_uniform_location(program, "u_mode");
            let texture_uniform = gl.get_uniform_location(program, "u_tex");
            gl.uniform_1_i32(texture_uniform.as_ref(), 0);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);

            Ok(Self {
                gl,
                program,
                vao,
                vbo,
                atlas,
                image,
                mode_uniform,
                texture_uniform,
                solid: mesh.solid,
                glyphs: mesh.glyphs,
                image_range: mesh.image,
                info,
            })
        }
    }

    fn draw(&self, scene: &Scene, width: u32, height: u32) {
        let clear = scene.clear_color();
        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, width as i32, height as i32);
            gl.clear_color(clear[0], clear[1], clear[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(self.texture_uniform.as_ref(), 0);

            gl.uniform_1_i32(self.mode_uniform.as_ref(), 0);
            gl.draw_arrays(
                glow::TRIANGLES,
                self.solid.0 as i32,
                (self.solid.1 - self.solid.0) as i32,
            );

            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.uniform_1_i32(self.mode_uniform.as_ref(), 1);
            gl.draw_arrays(
                glow::TRIANGLES,
                self.glyphs.0 as i32,
                (self.glyphs.1 - self.glyphs.0) as i32,
            );

            gl.bind_texture(glow::TEXTURE_2D, Some(self.image));
            gl.uniform_1_i32(self.mode_uniform.as_ref(), 2);
            gl.draw_arrays(
                glow::TRIANGLES,
                self.image_range.0 as i32,
                (self.image_range.1 - self.image_range.0) as i32,
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
        }
    }

    fn info(&self) -> String {
        self.info.clone()
    }
}

unsafe fn compile_shader(
    gl: &glow::Context,
    kind: u32,
    source: &str,
) -> Result<glow::Shader> {
    unsafe {
    let shader = gl.create_shader(kind).map_err(|err| Error::Gl(err))?;
    gl.shader_source(shader, source);
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        let log = gl.get_shader_info_log(shader);
        return Err(Error::Gl(format!("shader compile failed: {log}")));
    }
    Ok(shader)
    }
}

unsafe fn upload_texture(
    gl: &glow::Context,
    width: u32,
    height: u32,
    internal_format: i32,
    pixels: &[u8],
) -> Result<glow::Texture> {
    unsafe {
    let texture = gl.create_texture().map_err(|err| Error::Gl(err))?;
    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        internal_format,
        width as i32,
        height as i32,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(Some(pixels)),
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MIN_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MAG_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_S,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_T,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
    Ok(texture)
    }
}

pub fn offscreen(scene: &Scene, frames: u32) -> Result<OffscreenOutput> {
    let conn = wayland_client::Connection::connect_to_env()
        .map_err(|err| Error::Wayland(format!("cannot connect for EGL offscreen: {err}")))?;
    let display_ptr = conn.display().id().as_ptr() as *mut c_void;
    let display = display_from_raw(display_ptr)?;
    let config = choose_config(&display, ConfigSurfaceTypes::PBUFFER, false)?;
    let not_current = create_context(&display, &config)?;
    let surface_attributes = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    );
    // SAFETY: a pbuffer needs no window handle; the display outlives the surface.
    let surface = unsafe { display.create_pbuffer_surface(&config, &surface_attributes) }
        .map_err(|err| glutin_err("pbuffer surface", err))?;
    let context = not_current
        .make_current(&surface)
        .map_err(|err| glutin_err("make current", err))?;
    let gl = load_gl(&display)?;
    let renderer = GlRenderer::new(gl, scene)?;

    let (framebuffer, color) = unsafe {
        let texture = renderer
            .gl
            .create_texture()
            .map_err(|err| Error::Gl(err))?;
        renderer
            .gl
            .bind_texture(glow::TEXTURE_2D, Some(texture));
        renderer.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        renderer.gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::SRGB8_ALPHA8 as i32,
            scene.width as i32,
            scene.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        renderer.gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        renderer.gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        let framebuffer = renderer
            .gl
            .create_framebuffer()
            .map_err(|err| Error::Gl(err))?;
        renderer
            .gl
            .bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        renderer.gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        let status = renderer.gl.check_framebuffer_status(glow::FRAMEBUFFER);
        if status != glow::FRAMEBUFFER_COMPLETE {
            return Err(Error::Gl(format!(
                "offscreen framebuffer incomplete: 0x{status:x}"
            )));
        }
        renderer.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        renderer.gl.enable(glow::FRAMEBUFFER_SRGB);
        (framebuffer, texture)
    };

    let mut times = Vec::with_capacity(frames as usize);
    unsafe {
        renderer
            .gl
            .bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
    }
    for _ in 0..frames {
        let start = Instant::now();
        renderer.draw(scene, scene.width, scene.height);
        unsafe {
            // Completion, not submission: this is what makes the timing honest.
            renderer.gl.finish();
        }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let mut pixels = vec![0u8; (scene.width * scene.height * 4) as usize];
    unsafe {
        renderer.gl.read_pixels(
            0,
            0,
            scene.width as i32,
            scene.height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        renderer
            .gl
            .bind_framebuffer(glow::FRAMEBUFFER, None);
        renderer.gl.delete_framebuffer(framebuffer);
        renderer.gl.delete_texture(color);
    }
    flip_rows(&mut pixels, scene.width, scene.height);

    let info = renderer.info();
    let gl = renderer.gl;
    unsafe {
        gl.delete_program(renderer.program);
        gl.delete_vertex_array(renderer.vao);
        gl.delete_buffer(renderer.vbo);
        gl.delete_texture(renderer.atlas);
        gl.delete_texture(renderer.image);
    }
    drop(context);
    drop(surface);
    drop(display);
    drop(conn);

    Ok(OffscreenOutput {
        pixels,
        width: scene.width,
        height: scene.height,
        times_ms: times,
        renderer: info,
    })
}

fn flip_rows(pixels: &mut [u8], width: u32, height: u32) {
    let stride = (width * 4) as usize;
    for row in 0..height as usize / 2 {
        let top = row * stride;
        let bottom = (height as usize - 1 - row) * stride;
        for offset in 0..stride {
            pixels.swap(top + offset, bottom + offset);
        }
    }
}

pub struct GlPresenter {
    display: Display,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    renderer: GlRenderer,
    width: u32,
    height: u32,
}

impl GlPresenter {
    pub fn new(
        display_ptr: *mut c_void,
        surface_ptr: *mut c_void,
        width: u32,
        height: u32,
        scene: &Scene,
    ) -> Result<Self> {
        let display = display_from_raw(display_ptr)?;
        let config = choose_config(&display, ConfigSurfaceTypes::WINDOW, true)?;
        let not_current = create_context(&display, &config)?;
        let window = NonNull::new(surface_ptr)
            .map(WaylandWindowHandle::new)
            .ok_or_else(|| Error::Gl("Wayland surface pointer is null".into()))?;
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
            .with_srgb(Some(true))
            .build(
                RawWindowHandle::Wayland(window),
                NonZeroU32::new(width).unwrap(),
                NonZeroU32::new(height).unwrap(),
            );
        // SAFETY: the wl_surface belongs to the live layer surface and outlives
        // this EGL window surface.
        let surface = unsafe { display.create_window_surface(&config, &attributes) }
            .map_err(|err| glutin_err("EGL window surface", err))?;
        let context = not_current
            .make_current(&surface)
            .map_err(|err| glutin_err("make current", err))?;
        if let Err(err) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
            eprintln!("hayu-gpu-spike: swap interval: {err}");
        }
        let gl = load_gl(&display)?;
        let renderer = GlRenderer::new(gl, scene)?;
        unsafe {
            renderer.gl.enable(glow::FRAMEBUFFER_SRGB);
        }
        Ok(Self {
            display,
            context,
            surface,
            renderer,
            width,
            height,
        })
    }
}

impl Presenter for GlPresenter {
    fn render(&mut self, scene: &Scene) -> Result<()> {
        // Each output owns its own EGL context, so the current context must be
        // restored before every draw: creating the second output's presenter
        // switches it away from the first one.
        self.context
            .make_current(&self.surface)
            .map_err(|err| glutin_err("make current", err))?;
        self.renderer.draw(scene, self.width, self.height);
        unsafe {
            // Finish before the swap so the reported frame time includes the
            // GPU completing this scene.
            self.renderer.gl.finish();
        }
        self.surface
            .swap_buffers(&self.context)
            .map_err(|err| glutin_err("swap buffers", err))
    }

    fn renderer_name(&self) -> String {
        self.renderer.info()
    }
}

impl Drop for GlPresenter {
    fn drop(&mut self) {
        let renderer = &self.renderer;
        unsafe {
            renderer.gl.delete_program(renderer.program);
            renderer.gl.delete_vertex_array(renderer.vao);
            renderer.gl.delete_buffer(renderer.vbo);
            renderer.gl.delete_texture(renderer.atlas);
            renderer.gl.delete_texture(renderer.image);
        }
        let _ = &self.display;
        let _ = self.width;
        let _ = self.height;
    }
}

pub fn capability() -> Result<String> {
    let conn = wayland_client::Connection::connect_to_env()
        .map_err(|err| Error::Wayland(format!("cannot connect for EGL: {err}")))?;
    let display = display_from_raw(conn.display().id().as_ptr() as *mut c_void)?;
    let config = choose_config(&display, ConfigSurfaceTypes::PBUFFER, false)?;
    let not_current = create_context(&display, &config)?;
    let attributes = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    );
    // SAFETY: display outlives the pbuffer.
    let surface = unsafe { display.create_pbuffer_surface(&config, &attributes) }
        .map_err(|err| glutin_err("pbuffer", err))?;
    let _context = not_current
        .make_current(&surface)
        .map_err(|err| glutin_err("make current", err))?;
    let gl = load_gl(&display)?;
    let info = format!(
        "{} / {}",
        unsafe { gl.get_parameter_string(glow::RENDERER) },
        unsafe { gl.get_parameter_string(glow::VERSION) }
    );
    Ok(info)
}
