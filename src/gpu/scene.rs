use cosmic_text::FontSystem;

use super::atlas::Atlas;
use super::{Error, Result};

pub const LOGICAL_WIDTH: u32 = 1366;
pub const LOGICAL_HEIGHT: u32 = 32;
pub const IMAGE_SIZE: u32 = 64;

pub const BG: [u8; 3] = [0x1E, 0x1E, 0x2E];
pub const FG: [u8; 3] = [0xCD, 0xD6, 0xF4];
pub const MUTED: [u8; 3] = [0x6C, 0x70, 0x86];
pub const ACCENT: [u8; 3] = [0x8A, 0xB4, 0xF8];
pub const PILL_IDLE: [u8; 3] = [0x2A, 0x2A, 0x3C];

const CLOCK: &str = "09:41";
const TITLE: &str = "Hayu renderer spike";
const LABELS: [&str; 5] = ["1", "2", "3", "4", "5"];
const ACTIVE_PILL: usize = 1;

#[derive(Clone, Copy, Debug)]
pub struct RoundedRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub color: [u8; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct GlyphQuad {
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
    /// Atlas texel origin, used by the CPU path.
    pub ax: u32,
    pub ay: u32,
    pub uv: [f32; 4],
    pub color: [u8; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct ImageQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Shared scene used by CPU, GL and wgpu. Geometry is generated once by this
/// module so backends cannot drift apart.
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub bg: [u8; 3],
    pub rects: Vec<RoundedRect>,
    pub glyphs: Vec<GlyphQuad>,
    pub image_quad: ImageQuad,
    pub image: Vec<u8>,
    pub atlas: Atlas,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

impl Vertex {
    pub fn write_bytes(&self, out: &mut Vec<u8>) {
        for value in self
            .pos
            .iter()
            .chain(self.uv.iter())
            .chain(self.color.iter())
        {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub solid: (u32, u32),
    pub glyphs: (u32, u32),
    pub image: (u32, u32),
}

impl Scene {
    pub fn build(scale: u32) -> Result<Self> {
        let scale = scale.max(1);
        let width = LOGICAL_WIDTH * scale;
        let height = LOGICAL_HEIGHT * scale;
        let s = scale as f32;
        let font_size = 13.0 * s;
        let text_y = ((LOGICAL_HEIGHT as f32 * s) - font_size * 1.3) / 2.0;

        let mut fonts = FontSystem::new();
        let mut texts: Vec<&str> = LABELS.to_vec();
        texts.push(CLOCK);
        texts.push(TITLE);
        let atlas = Atlas::build(&mut fonts, font_size, &texts)?;

        let pill_w = 28.0 * s;
        let pill_h = 24.0 * s;
        let pill_y = 4.0 * s;
        let gap = 4.0 * s;
        let radius = 6.0 * s;
        let mut rects = Vec::with_capacity(LABELS.len());
        let mut glyphs = Vec::new();
        for (index, label) in LABELS.iter().enumerate() {
            let x = 8.0 * s + index as f32 * (pill_w + gap);
            let active = index == ACTIVE_PILL;
            rects.push(RoundedRect {
                x,
                y: pill_y,
                w: pill_w,
                h: pill_h,
                radius,
                color: if active { ACCENT } else { PILL_IDLE },
            });
            let layout = atlas.layout(label);
            let text_w = layout.map(|l| l.width).unwrap_or(0.0);
            let tx = (x + (pill_w - text_w) / 2.0).round();
            let color = if active { BG } else { FG };
            push_text(&mut glyphs, layout, tx, text_y.round(), color);
        }

        let clock_layout = atlas.layout(CLOCK);
        let clock_w = clock_layout.map(|l| l.width).unwrap_or(0.0);
        let clock_x = ((width as f32 - clock_w) / 2.0).round();
        push_text(&mut glyphs, clock_layout, clock_x, text_y.round(), FG);

        let title_layout = atlas.layout(TITLE);
        let title_x = (clock_x + clock_w + 12.0 * s).round();
        push_text(&mut glyphs, title_layout, title_x, text_y.round(), MUTED);

        let image_size = 24.0 * s;
        let image_quad = ImageQuad {
            x: (width as f32 - image_size - 8.0 * s).round(),
            y: 4.0 * s,
            w: image_size,
            h: image_size,
        };

        Ok(Self {
            width,
            height,
            scale,
            bg: BG,
            rects,
            glyphs,
            image_quad,
            image: procedural_image(),
            atlas,
        })
    }

    pub fn clear_color(&self) -> [f32; 3] {
        [
            srgb_to_linear(self.bg[0]),
            srgb_to_linear(self.bg[1]),
            srgb_to_linear(self.bg[2]),
        ]
    }

    fn ndc(&self, x: f32, y: f32) -> [f32; 2] {
        [
            x / self.width as f32 * 2.0 - 1.0,
            1.0 - y / self.height as f32 * 2.0,
        ]
    }

    pub fn mesh(&self) -> Mesh {
        let mut vertices = Vec::new();
        let solid_start = 0;
        push_textured_quad(
            &mut vertices,
            self,
            0.0,
            0.0,
            self.width as f32,
            self.height as f32,
            [0.0, 0.0, 1.0, 1.0],
            self.bg,
        );
        for rect in &self.rects {
            push_rounded_rect(&mut vertices, self, rect);
        }
        let solid_end = vertices.len() as u32;

        for glyph in &self.glyphs {
            push_textured_quad(
                &mut vertices,
                self,
                glyph.x,
                glyph.y,
                glyph.w as f32,
                glyph.h as f32,
                glyph.uv,
                glyph.color,
            );
        }
        let glyph_end = vertices.len() as u32;

        let image = &self.image_quad;
        push_textured_quad(
            &mut vertices,
            self,
            image.x,
            image.y,
            image.w,
            image.h,
            [0.0, 0.0, 1.0, 1.0],
            [255, 255, 255],
        );
        let image_end = vertices.len() as u32;

        Mesh {
            vertices,
            solid: (solid_start, solid_end),
            glyphs: (solid_end, glyph_end),
            image: (glyph_end, image_end),
        }
    }
}

fn push_text(
    out: &mut Vec<GlyphQuad>,
    layout: Option<&super::atlas::TextLayout>,
    x: f32,
    y: f32,
    color: [u8; 3],
) {
    let Some(layout) = layout else { return };
    for glyph in &layout.glyphs {
        out.push(GlyphQuad {
            x: x + glyph.x as f32,
            y: y + glyph.y as f32,
            w: glyph.w,
            h: glyph.h,
            ax: glyph.ax,
            ay: glyph.ay,
            uv: glyph.uv,
            color,
        });
    }
}

fn push_textured_quad(
    out: &mut Vec<Vertex>,
    scene: &Scene,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    uv: [f32; 4],
    color: [u8; 3],
) {
    let color = [
        srgb_to_linear(color[0]),
        srgb_to_linear(color[1]),
        srgb_to_linear(color[2]),
    ];
    let corners = [
        (x, y, [uv[0], uv[1]]),
        (x + w, y, [uv[2], uv[1]]),
        (x + w, y + h, [uv[2], uv[3]]),
        (x, y + h, [uv[0], uv[3]]),
    ];
    for &(cx, cy, cuv) in &[corners[0], corners[1], corners[2], corners[0], corners[2], corners[3]] {
        out.push(Vertex {
            pos: scene.ndc(cx, cy),
            uv: cuv,
            color,
        });
    }
}

fn push_rounded_rect(out: &mut Vec<Vertex>, scene: &Scene, rect: &RoundedRect) {
    let color = [
        srgb_to_linear(rect.color[0]),
        srgb_to_linear(rect.color[1]),
        srgb_to_linear(rect.color[2]),
    ];
    let r = rect.radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    if r <= 0.01 {
        push_textured_quad(
            out,
            scene,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            [0.0, 0.0, 1.0, 1.0],
            rect.color,
        );
        return;
    }
    let steps = (r * 1.5).clamp(2.0, 16.0) as u32;
    let mut points: Vec<(f32, f32)> = Vec::with_capacity((4 * (steps + 1)) as usize);
    let corners = [
        (rect.x + r, rect.y + r, std::f32::consts::PI),
        (
            rect.x + rect.w - r,
            rect.y + r,
            1.5 * std::f32::consts::PI,
        ),
        (
            rect.x + rect.w - r,
            rect.y + rect.h - r,
            0.0,
        ),
        (
            rect.x + r,
            rect.y + rect.h - r,
            0.5 * std::f32::consts::PI,
        ),
    ];
    for &(cx, cy, start) in &corners {
        let sweep = std::f32::consts::FRAC_PI_2;
        for step in 0..=steps {
            let angle = start + sweep * step as f32 / steps as f32;
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
    }
    let center = scene.ndc(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let mut push_triangle = |a: (f32, f32), b: (f32, f32)| {
        for (px, py) in [a, b] {
            out.push(Vertex {
                pos: scene.ndc(px, py),
                uv: [0.0, 0.0],
                color,
            });
        }
        out.push(Vertex {
            pos: center,
            uv: [0.0, 0.0],
            color,
        });
    };
    for pair in points.windows(2) {
        push_triangle(pair[0], pair[1]);
    }
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        push_triangle(*last, *first);
    }
}

pub fn srgb_to_linear(value: u8) -> f32 {
    let c = value as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(value: f32) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

fn procedural_image() -> Vec<u8> {
    let size = IMAGE_SIZE;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let checker = ((x / 8) + (y / 8)) % 2 == 0;
            let index = ((y * size + x) * 4) as usize;
            pixels[index] = (40 + x * 2 + if checker { 60 } else { 0 }).min(255) as u8;
            pixels[index + 1] = (30 + y * 3) as u8;
            pixels[index + 2] = (110 + x + if checker { 60 } else { 0 }).min(255) as u8;
            pixels[index + 3] = 255;
        }
    }
    pixels
}

pub fn validate_scale(scale: u32) -> Result<()> {
    if !(1..=4).contains(&scale) {
        return Err(Error::Usage(format!(
            "--scale must be 1..=4, got {scale}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_geometry_is_stable() {
        let scene = Scene::build(1).unwrap();
        assert_eq!(scene.width, 1366);
        assert_eq!(scene.height, 32);
        assert_eq!(scene.rects.len(), 5);
        assert_eq!(scene.image.len(), 64 * 64 * 4);
        let mesh = scene.mesh();
        assert!(mesh.solid.1 > mesh.solid.0);
        assert!(mesh.glyphs.1 > mesh.glyphs.0);
        assert_eq!(mesh.image.1 - mesh.image.0, 6);
        assert_eq!(mesh.vertices.len() as u32, mesh.image.1);
    }

    #[test]
    fn scale_two_doubles_pixels() {
        let scene = Scene::build(2).unwrap();
        assert_eq!(scene.width, 2732);
        assert_eq!(scene.height, 64);
        assert_eq!(scene.image_quad.w, 48.0);
    }
}
