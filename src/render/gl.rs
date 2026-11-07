use std::num::NonZeroU32;
use std::os::raw::c_void;
use std::ptr::NonNull;

use glow::HasContext;
use glutin::config::{Config, ConfigSurfaceTypes, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, NotCurrentContext, NotCurrentGlContext,
    PossiblyCurrentContext, PossiblyCurrentGlContext, Version,
};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

use super::{Atlas, Frame};

/// GLES/EGL presenter for one layer surface. Every output owns its own EGL
/// context, so the context is made current before each draw.
pub struct GlPresenter {
    display: Display,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    texture: glow::Texture,
    image_texture: glow::Texture,
    atlas_version: Option<u32>,
    image_version: Option<u32>,
    mode: Option<glow::UniformLocation>,
    sampler: Option<glow::UniformLocation>,
    width: u32,
    height: u32,
}

const STRIDE: i32 = 28;

impl GlPresenter {
    pub fn new(
        display_ptr: *mut c_void,
        surface_ptr: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let display = display_from_raw(display_ptr)?;
        let config = choose_config(&display)?;
        let not_current = create_context(&display, &config)?;
        let window = NonNull::new(surface_ptr)
            .map(WaylandWindowHandle::new)
            .ok_or_else(|| "Wayland surface pointer is null".to_string())?;
        let attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
            .with_srgb(Some(true))
            .build(
                RawWindowHandle::Wayland(window),
                NonZeroU32::new(width.max(1)).unwrap(),
                NonZeroU32::new(height.max(1)).unwrap(),
            );
        // SAFETY: the wl_surface belongs to the live layer surface and outlives
        // this EGL window surface.
        let surface = unsafe { display.create_window_surface(&config, &attributes) }
            .map_err(|err| format!("EGL window surface: {err}"))?;
        let context = not_current
            .make_current(&surface)
            .map_err(|err| format!("EGL make current: {err}"))?;
        if let Err(err) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
            eprintln!("hayu: swap interval: {err}");
        }
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name))
        };
        unsafe {
            let (program, vao, vbo, texture, image_texture) = setup(&gl)?;
            gl.enable(glow::FRAMEBUFFER_SRGB);
            let mode = gl.get_uniform_location(program, "u_mode");
            let sampler = gl.get_uniform_location(program, "u_tex");
            Ok(Self {
                display,
                context,
                surface,
                gl,
                program,
                vao,
                vbo,
                texture,
                image_texture,
                atlas_version: None,
                image_version: None,
                mode,
                sampler,
                width,
                height,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
        }
        self.surface.resize(
            &self.context,
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        );
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn render(&mut self, frame: &Frame, atlas: &Atlas) -> Result<(), String> {
        self.context
            .make_current(&self.surface)
            .map_err(|err| format!("EGL make current: {err}"))?;
        if frame.width != self.width || frame.height != self.height {
            self.resize(frame.width, frame.height)?;
        }
        let mut vertices = Vec::with_capacity((frame.rects.len() + frame.glyphs.len()) * 6 * 7);
        for rect in &frame.rects {
            let color = linear(rect.color);
            let corners = [
                (rect.x, rect.y),
                (rect.x + rect.w, rect.y),
                (rect.x, rect.y + rect.h),
                (rect.x + rect.w, rect.y + rect.h),
            ];
            for index in [0, 1, 2, 2, 1, 3] {
                let (x, y) = corners[index];
                push(
                    &mut vertices,
                    ndc(x, y, frame.width as f32, frame.height as f32),
                    [0.0, 0.0],
                    color,
                );
            }
        }
        let solid_end = vertices.len() as u32 / 7;
        for sprite in &frame.sprites {
            let (x0, y0) = (sprite.x, sprite.y);
            let (x1, y1) = (x0 + sprite.w, y0 + sprite.h);
            let uv = sprite.uv;
            let corners = [
                (x0, y0, [uv[0], uv[1]]),
                (x1, y0, [uv[2], uv[1]]),
                (x0, y1, [uv[0], uv[3]]),
                (x1, y1, [uv[2], uv[3]]),
            ];
            for index in [0, 1, 2, 2, 1, 3] {
                let (x, y, uv) = corners[index];
                push(
                    &mut vertices,
                    ndc(x, y, frame.width as f32, frame.height as f32),
                    uv,
                    [1.0, 1.0, 1.0],
                );
            }
        }
        let sprite_end = vertices.len() as u32 / 7;
        for glyph in &frame.glyphs {
            let color = linear(glyph.color);
            let (x0, y0) = (glyph.x, glyph.y);
            let (x1, y1) = (x0 + glyph.w as f32, y0 + glyph.h as f32);
            let uv = glyph.uv;
            let corners = [
                (x0, y0, [uv[0], uv[1]]),
                (x1, y0, [uv[2], uv[1]]),
                (x0, y1, [uv[0], uv[3]]),
                (x1, y1, [uv[2], uv[3]]),
            ];
            for index in [0, 1, 2, 2, 1, 3] {
                let (x, y, uv) = corners[index];
                push(
                    &mut vertices,
                    ndc(x, y, frame.width as f32, frame.height as f32),
                    uv,
                    color,
                );
            }
        }
        let total = vertices.len() as u32 / 7;

        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, self.width as i32, self.height as i32);
            let clear = linear(frame.bg);
            gl.clear_color(clear[0], clear[1], clear[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                floats_to_bytes(&vertices),
                glow::DYNAMIC_DRAW,
            );

            gl.active_texture(glow::TEXTURE0);
            gl.uniform_1_i32(self.sampler.as_ref(), 0);
            gl.uniform_1_i32(self.mode.as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLES, 0, solid_end as i32);
            if sprite_end > solid_end {
                if let Some(image) = &frame.image
                    && self.image_version != Some(image.version)
                {
                    upload_image(gl, self.image_texture, image);
                    self.image_version = Some(image.version);
                }
                gl.bind_texture(glow::TEXTURE_2D, Some(self.image_texture));
                gl.uniform_1_i32(self.mode.as_ref(), 2);
                gl.draw_arrays(
                    glow::TRIANGLES,
                    solid_end as i32,
                    (sprite_end - solid_end) as i32,
                );
            }
            if total > sprite_end {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
                if self.atlas_version != Some(atlas.version) {
                    upload_atlas(gl, self.texture, atlas);
                    self.atlas_version = Some(atlas.version);
                }
                gl.uniform_1_i32(self.mode.as_ref(), 1);
                gl.draw_arrays(
                    glow::TRIANGLES,
                    sprite_end as i32,
                    (total - sprite_end) as i32,
                );
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
        }
        self.surface
            .swap_buffers(&self.context)
            .map_err(|err| format!("EGL swap buffers: {err}"))
    }
}

impl Drop for GlPresenter {
    fn drop(&mut self) {
        // Each output has its own context, so another presenter's context may be
        // current here; the object names are only valid in this one.
        if self.context.make_current(&self.surface).is_err() {
            return;
        }
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_texture(self.texture);
            self.gl.delete_texture(self.image_texture);
        }
        let _ = &self.display;
    }
}

fn ndc(x: f32, y: f32, width: f32, height: f32) -> [f32; 2] {
    [x / width * 2.0 - 1.0, 1.0 - y / height * 2.0]
}

fn push(out: &mut Vec<f32>, pos: [f32; 2], uv: [f32; 2], color: [f32; 3]) {
    out.extend_from_slice(&[pos[0], pos[1], uv[0], uv[1], color[0], color[1], color[2]]);
}

fn floats_to_bytes(floats: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and the slice is contiguous; the borrow keeps
    // the bytes alive for the duration of the upload.
    unsafe { std::slice::from_raw_parts(floats.as_ptr().cast(), std::mem::size_of_val(floats)) }
}

fn linear(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(|value| {
        let c = value as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn display_from_raw(display_ptr: *mut c_void) -> Result<Display, String> {
    let handle = NonNull::new(display_ptr)
        .map(WaylandDisplayHandle::new)
        .ok_or_else(|| "Wayland display pointer is null".to_string())?;
    // SAFETY: the caller guarantees the wl_display outlives every GL object
    // created from this Display (the Wayland connection lives for the run).
    unsafe { Display::new(RawDisplayHandle::Wayland(handle), DisplayApiPreference::Egl) }
        .map_err(|err| format!("EGL display: {err}"))
}

fn choose_config(display: &Display) -> Result<Config, String> {
    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_surface_type(ConfigSurfaceTypes::WINDOW)
        .build();
    // SAFETY: template contains no raw handles; only the display is queried.
    let configs =
        unsafe { display.find_configs(template) }.map_err(|err| format!("EGL configs: {err}"))?;
    let mut fallback = None;
    for config in configs {
        if !config.srgb_capable() {
            continue;
        }
        if config.num_samples() == 0 {
            return Ok(config);
        }
        fallback = Some(config);
    }
    fallback.ok_or_else(|| "no sRGB EGL config for a window surface".to_string())
}

fn create_context(display: &Display, config: &Config) -> Result<NotCurrentContext, String> {
    let attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(None);
    // SAFETY: config comes from this display; the context is bound to a window
    // surface right after creation.
    unsafe { display.create_context(config, &attributes) }
        .map_err(|err| format!("EGL context: {err}"))
}

unsafe fn setup(
    gl: &glow::Context,
) -> Result<
    (
        glow::Program,
        glow::VertexArray,
        glow::Buffer,
        glow::Texture,
        glow::Texture,
    ),
    String,
> {
    unsafe {
        let program = gl.create_program().map_err(|err| err.to_string())?;
        let vertex = compile(
            gl,
            glow::VERTEX_SHADER,
            r#"#version 300 es
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
"#,
        )?;
        let fragment = compile(
            gl,
            glow::FRAGMENT_SHADER,
            r#"#version 300 es
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
"#,
        )?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!(
                "program link failed: {}",
                gl.get_program_info_log(program)
            ));
        }
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);

        let vao = gl.create_vertex_array().map_err(|err| err.to_string())?;
        let vbo = gl.create_buffer().map_err(|err| err.to_string())?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, STRIDE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, STRIDE, 8);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, STRIDE, 16);
        gl.bind_vertex_array(None);

        let texture = new_texture(gl)?;
        let image_texture = new_texture(gl)?;
        gl.use_program(Some(program));
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

        gl.use_program(Some(program));
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.bind_vertex_array(None);
        Ok((program, vao, vbo, texture, image_texture))
    }
}
unsafe fn new_texture(gl: &glow::Context) -> Result<glow::Texture, String> {
    unsafe {
        let texture = gl.create_texture().map_err(|err| err.to_string())?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
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
        Ok(texture)
    }
}

unsafe fn compile(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl.create_shader(kind).map_err(|err| err.to_string())?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            return Err(format!(
                "shader compile failed: {}",
                gl.get_shader_info_log(shader)
            ));
        }
        Ok(shader)
    }
}

unsafe fn upload_image(gl: &glow::Context, texture: glow::Texture, image: &super::Image) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::SRGB8_ALPHA8 as i32,
            image.width as i32,
            image.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&image.rgba)),
        );
    }
}
unsafe fn upload_atlas(gl: &glow::Context, texture: glow::Texture, atlas: &Atlas) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            atlas.width as i32,
            atlas.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&atlas.pixels)),
        );
    }
}
