#![cfg(feature = "wgpu")]

use std::os::raw::c_void;
use std::ptr::NonNull;
use std::time::Instant;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wgpu::util::DeviceExt;

use super::present::Presenter;
use super::scene::Scene;
use super::{Error, OffscreenOutput, Result};

const SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec3<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_solid(input: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}

@fragment
fn fs_text(input: VertexOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(tex, samp, input.uv).a;
    return vec4<f32>(input.color, alpha);
}

@fragment
fn fs_image(input: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, input.uv);
}
"#;

fn wgpu_err(context: &str, err: impl std::fmt::Display) -> Error {
    Error::Wgpu(format!("{context}: {err}"))
}

struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    solid: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    image: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    atlas_bind: wgpu::BindGroup,
    image_bind: wgpu::BindGroup,
    solid_range: (u32, u32),
    glyph_range: (u32, u32),
    image_range: (u32, u32),
    info: String,
}

impl GpuRenderer {
    fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        scene: &Scene,
        info: String,
    ) -> Result<Self> {
        let mesh = scene.mesh();
        let mut bytes = Vec::with_capacity(mesh.vertices.len() * 28);
        for vertex in &mesh.vertices {
            vertex.write_bytes(&mut bytes);
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hayu-spike vertices"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hayu-spike shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hayu-spike texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hayu-spike pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: 28,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 16,
                    shader_location: 2,
                },
            ],
        };

        let make_pipeline = |label: &str, fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = make_pipeline("hayu-spike solid", "fs_solid");
        let text = make_pipeline("hayu-spike text", "fs_text");
        let image = make_pipeline("hayu-spike image", "fs_image");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hayu-spike sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let atlas_view = create_texture(
            &device,
            &queue,
            "hayu-spike atlas",
            scene.atlas.width,
            scene.atlas.height,
            wgpu::TextureFormat::Rgba8Unorm,
            &scene.atlas.pixels,
        );
        let image_view = create_texture(
            &device,
            &queue,
            "hayu-spike image",
            super::scene::IMAGE_SIZE,
            super::scene::IMAGE_SIZE,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &scene.image,
        );
        let make_bind = |view: &wgpu::TextureView, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let atlas_bind = make_bind(&atlas_view, "hayu-spike atlas bind");
        let image_bind = make_bind(&image_view, "hayu-spike image bind");

        Ok(Self {
            device,
            queue,
            solid,
            text,
            image,
            vertex_buffer,
            atlas_bind,
            image_bind,
            solid_range: mesh.solid,
            glyph_range: mesh.glyphs,
            image_range: mesh.image,
            info,
        })
    }

    fn draw(&self, view: &wgpu::TextureView, scene: &Scene) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hayu-spike encoder"),
            });
        let clear = scene.clear_color();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hayu-spike pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            pass.set_pipeline(&self.solid);
            pass.set_bind_group(0, &self.atlas_bind, &[]);
            pass.draw(
                self.solid_range.0..self.solid_range.1,
                0..1,
            );

            pass.set_pipeline(&self.text);
            pass.draw(
                self.glyph_range.0..self.glyph_range.1,
                0..1,
            );

            pass.set_pipeline(&self.image);
            pass.set_bind_group(0, &self.image_bind, &[]);
            pass.draw(
                self.image_range.0..self.image_range.1,
                0..1,
            );
        }
        self.queue.submit(Some(encoder.finish()));
    }

    fn wait(&self) {
        if let Err(err) = self.device.poll(wgpu::PollType::wait_indefinitely()) {
            eprintln!("hayu-gpu-spike: wgpu poll: {err}");
        }
    }
}

fn create_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    pixels: &[u8],
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    wgpu::Instance::new(descriptor)
}

fn request_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter> {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: surface,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|err| wgpu_err("no Vulkan adapter", err))
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("hayu-gpu-spike"),
        ..Default::default()
    }))
    .map_err(|err| wgpu_err("request device", err))
}

fn info_string(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "{} [{} {:?} {} / {}]",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    )
}

pub fn offscreen(scene: &Scene, frames: u32) -> Result<OffscreenOutput> {
    let instance = instance();
    let adapter = request_adapter(&instance, None)?;
    let info = info_string(&adapter);
    let (device, queue) = request_device(&adapter)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hayu-spike offscreen"),
        size: wgpu::Extent3d {
            width: scene.width,
            height: scene.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let renderer = GpuRenderer::new(device, queue, format, scene, info)?;

    let mut times = Vec::with_capacity(frames as usize);
    for _ in 0..frames {
        let start = Instant::now();
        renderer.draw(&view, scene);
        // Completion, not submission: the number below includes GPU work.
        renderer.wait();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let bytes_per_row = scene.width * 4;
    let padded = bytes_per_row.div_ceil(256) * 256;
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hayu-spike readback"),
        size: (padded * scene.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hayu-spike readback encoder"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(scene.height),
            },
        },
        wgpu::Extent3d {
            width: scene.width,
            height: scene.height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit(Some(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = tx.send(result);
    });
    renderer.wait();
    rx.recv()
        .map_err(|err| Error::Wgpu(format!("readback channel: {err}")))?
        .map_err(|err| wgpu_err("map readback", err))?;
    let mapped = readback
        .get_mapped_range(..)
        .map_err(|err| wgpu_err("mapped range", err))?;
    let mut pixels = vec![0u8; (bytes_per_row * scene.height) as usize];
    for row in 0..scene.height as usize {
        let src = row * padded as usize;
        let dst = row * bytes_per_row as usize;
        pixels[dst..dst + bytes_per_row as usize]
            .copy_from_slice(&mapped[src..src + bytes_per_row as usize]);
    }
    drop(mapped);
    readback.unmap();

    let info = renderer.info.clone();
    Ok(OffscreenOutput {
        pixels,
        width: scene.width,
        height: scene.height,
        times_ms: times,
        renderer: info,
    })
}

pub struct WgpuPresenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
}

impl WgpuPresenter {
    pub fn new(
        display_ptr: *mut c_void,
        surface_ptr: *mut c_void,
        width: u32,
        height: u32,
        scene: &Scene,
    ) -> Result<Self> {
        let display = NonNull::new(display_ptr)
            .ok_or_else(|| Error::Wgpu("Wayland display pointer is null".into()))?;
        let window = NonNull::new(surface_ptr)
            .ok_or_else(|| Error::Wgpu("Wayland surface pointer is null".into()))?;
        let instance = instance();
        // SAFETY: both pointers come from the live Wayland connection and layer
        // surface; the harness drops this presenter before either object.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Wayland(
                    WaylandDisplayHandle::new(display),
                )),
                raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
            })
        }
        .map_err(|err| wgpu_err("create Wayland surface", err))?;
        let adapter = request_adapter(&instance, Some(&surface))?;
        let info = info_string(&adapter);
        let (device, queue) = request_device(&adapter)?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
            .ok_or_else(|| {
                Error::Wgpu(format!(
                    "surface has no sRGB format (offered {:?})",
                    capabilities.formats
                ))
            })?;
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| Error::Wgpu("surface is not supported by the adapter".into()))?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);
        let renderer = GpuRenderer::new(device, queue, format, scene, info)?;
        Ok(Self {
            surface,
            config,
            renderer,
        })
    }
}

impl Presenter for WgpuPresenter {
    fn render(&mut self, scene: &Scene) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                return Err(Error::Wgpu(format!(
                    "cannot acquire surface texture: {other:?}"
                )));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.draw(&view, scene);
        self.renderer.wait();
        self.renderer.queue.present(frame);
        Ok(())
    }

    fn renderer_name(&self) -> String {
        self.renderer.info.clone()
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.config.width == width && self.config.height == height {
            return Ok(());
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.renderer.device, &self.config);
        Ok(())
    }
}

pub fn capability() -> Result<String> {
    let instance = instance();
    let adapter = request_adapter(&instance, None)?;
    Ok(info_string(&adapter))
}
