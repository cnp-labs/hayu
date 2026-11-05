use std::time::Instant;

use tiny_skia::{Color as SkColor, Paint, PathBuilder, Pixmap, Transform};

use super::scene::{linear_to_srgb, srgb_to_linear, Scene};
use super::{Error, Result};

pub struct CpuOutput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub times_ms: Vec<f64>,
}

pub fn render(scene: &Scene, frames: u32) -> Result<CpuOutput> {
    let mut times = Vec::with_capacity(frames as usize);
    let mut last = Vec::new();
    for _ in 0..frames {
        let start = Instant::now();
        let pixels = render_frame(scene)?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        last = pixels;
    }
    Ok(CpuOutput {
        pixels: last,
        width: scene.width,
        height: scene.height,
        times_ms: times,
    })
}

fn render_frame(scene: &Scene) -> Result<Vec<u8>> {
    let mut pixmap = Pixmap::new(scene.width, scene.height)
        .ok_or_else(|| Error::Scene("cannot allocate CPU pixmap".into()))?;
    pixmap.fill(SkColor::from_rgba8(
        scene.bg[0],
        scene.bg[1],
        scene.bg[2],
        255,
    ));

    for rect in &scene.rects {
        let mut paint = Paint::default();
        paint.set_color_rgba8(rect.color[0], rect.color[1], rect.color[2], 255);
        paint.anti_alias = true;
        if rect.radius <= 0.0 {
            if let Some(rect_sk) = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.w, rect.h) {
                pixmap.fill_rect(rect_sk, &paint, Transform::identity(), None);
            }
        } else if let Some(path) = rounded_path(rect.x, rect.y, rect.w, rect.h, rect.radius) {
            pixmap.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    blend_glyphs(&mut pixmap, scene);
    blit_image(&mut pixmap, scene);

    let data = pixmap.data();
    // The pixmap is fully opaque, so premultiplied bytes equal straight RGBA.
    Ok(data.to_vec())
}

fn rounded_path(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(w / 2.0).min(h / 2.0);
    let k = 0.552_284_75 * r;
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.cubic_to(
        x + w,
        y + h - r + k,
        x + w - r + k,
        y + h,
        x + w - r,
        y + h,
    );
    path.line_to(x + r, y + h);
    path.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    path.line_to(x, y + r);
    path.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    path.close();
    path.finish()
}

fn blend_glyphs(pixmap: &mut Pixmap, scene: &Scene) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let atlas_w = scene.atlas.width as usize;
    let data = pixmap.data_mut();
    for glyph in &scene.glyphs {
        let src = [
            srgb_to_linear(glyph.color[0]),
            srgb_to_linear(glyph.color[1]),
            srgb_to_linear(glyph.color[2]),
        ];
        for gy in 0..glyph.h as i32 {
            let py = glyph.y as i32 + gy;
            if py < 0 || py >= height {
                continue;
            }
            for gx in 0..glyph.w as i32 {
                let px = glyph.x as i32 + gx;
                if px < 0 || px >= width {
                    continue;
                }
                let ax = glyph.ax as usize + gx as usize;
                let ay = glyph.ay as usize + gy as usize;
                let coverage = scene.atlas.pixels[(ay * atlas_w + ax) * 4 + 3] as f32 / 255.0;
                if coverage == 0.0 {
                    continue;
                }
                let index = ((py * width + px) * 4) as usize;
                for channel in 0..3 {
                    let dst = srgb_to_linear(data[index + channel]);
                    let out = dst + (src[channel] - dst) * coverage;
                    data[index + channel] = linear_to_srgb(out);
                }
            }
        }
    }
}

fn blit_image(pixmap: &mut Pixmap, scene: &Scene) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let quad = scene.image_quad;
    let tex_w = super::scene::IMAGE_SIZE as usize;
    let tex_h = super::scene::IMAGE_SIZE as usize;
    let data = pixmap.data_mut();
    let start_x = quad.x as i32;
    let start_y = quad.y as i32;
    let end_x = (quad.x + quad.w) as i32;
    let end_y = (quad.y + quad.h) as i32;
    for py in start_y.max(0)..end_y.min(height) {
        for px in start_x.max(0)..end_x.min(width) {
            let u = (px as f32 + 0.5 - quad.x) / quad.w;
            let v = (py as f32 + 0.5 - quad.y) / quad.h;
            let rgb = sample_bilinear(&scene.image, tex_w, tex_h, u * tex_w as f32 - 0.5, v * tex_h as f32 - 0.5);
            let index = ((py * width + px) * 4) as usize;
            data[index] = linear_to_srgb(rgb[0]);
            data[index + 1] = linear_to_srgb(rgb[1]);
            data[index + 2] = linear_to_srgb(rgb[2]);
            data[index + 3] = 255;
        }
    }
}

fn sample_bilinear(image: &[u8], width: usize, height: usize, fx: f32, fy: f32) -> [f32; 3] {
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let clamp = |x: i32, y: i32| -> [f32; 3] {
        let x = x.clamp(0, width as i32 - 1) as usize;
        let y = y.clamp(0, height as i32 - 1) as usize;
        let index = (y * width + x) * 4;
        [
            srgb_to_linear(image[index]),
            srgb_to_linear(image[index + 1]),
            srgb_to_linear(image[index + 2]),
        ]
    };
    let c00 = clamp(x0, y0);
    let c10 = clamp(x0 + 1, y0);
    let c01 = clamp(x0, y0 + 1);
    let c11 = clamp(x0 + 1, y0 + 1);
    let mut out = [0.0; 3];
    for channel in 0..3 {
        let top = c00[channel] + (c10[channel] - c00[channel]) * tx;
        let bottom = c01[channel] + (c11[channel] - c01[channel]) * tx;
        out[channel] = top + (bottom - top) * ty;
    }
    out
}
