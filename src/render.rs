use std::collections::HashMap;

use chrono::{NaiveTime, Timelike};
use cosmic_text::{
    Attrs, Buffer, CacheKey, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
};
use tiny_skia::{Color as SkColor, Pixmap};

use crate::config;
use crate::hyprland::Workspace;

pub mod gl;

/// Backend-neutral draw list. No backend shapes text or rasterizes glyphs: the
/// builder does that once and stores the bitmaps in [`Atlas`].
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bg: [u8; 3],
    pub rects: Vec<RectCmd>,
    pub glyphs: Vec<Glyph>,
    /// Workspace pills exactly as drawn, in buffer pixels. Hit testing reads
    /// this list instead of recomputing the layout.
    pub pills: Vec<Pill>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pill {
    pub id: i64,
    pub x: f32,
    pub w: f32,
}

/// Workspace whose drawn pill contains `logical_x`, if any.
pub fn pill_at(pills: &[Pill], logical_x: f32, scale: f32) -> Option<i64> {
    let x = logical_x * scale;
    pills
        .iter()
        .find(|pill| x >= pill.x && x < pill.x + pill.w)
        .map(|pill| pill.id)
}

#[derive(Clone, Copy, Debug)]
pub struct RectCmd {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
    /// Atlas texel origin, used by the CPU rasterizer.
    pub ax: u32,
    pub ay: u32,
    pub uv: [f32; 4],
    pub color: [u8; 3],
}

impl Frame {
    pub fn new(width: u32, height: u32, bg: [u8; 3]) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            bg,
            rects: Vec::new(),
            glyphs: Vec::new(),
            pills: Vec::new(),
        })
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 3]) {
        self.rects.push(RectCmd { x, y, w, h, color });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        fonts: &mut FontSystem,
        atlas: &mut Atlas,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: [u8; 3],
    ) {
        if text.is_empty() {
            return;
        }
        let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        buffer.set_size(None, Some(size * 1.3));
        buffer.shape_until_scroll(fonts, false);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((x, y + run.line_y), 1.0);
                let Some(slot) = atlas.slot(fonts, physical.cache_key) else {
                    continue;
                };
                self.glyphs.push(Glyph {
                    x: (physical.x + slot.left) as f32,
                    y: (physical.y - slot.top) as f32,
                    w: slot.w,
                    h: slot.h,
                    ax: slot.ax,
                    ay: slot.ay,
                    uv: slot.uv,
                    color,
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Slot {
    ax: u32,
    ay: u32,
    w: u32,
    h: u32,
    left: i32,
    top: i32,
    uv: [f32; 4],
}

/// Dynamic glyph atlas. A glyph is rasterized the first time it is drawn and
/// then reused by every later frame. One surface's atlas is 1024×1024, which
/// holds thousands of unique glyphs. When it fills, the rest of that frame skips
/// new glyphs and the next frame resets once, so no emitted UV is invalidated;
/// upgrade by growing the texture if skipping ever shows up in practice.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8, white RGB with coverage in alpha.
    pub pixels: Vec<u8>,
    /// Bumped on every change so a presenter knows when to re-upload.
    pub version: u32,
    slots: HashMap<CacheKey, Slot>,
    cache: SwashCache,
    pen_x: u32,
    pen_y: u32,
    row_h: u32,
    frame_glyphs: u32,
}

const ATLAS_SIZE: u32 = 1024;
const PAD: u32 = 1;

impl Atlas {
    pub fn new() -> Self {
        Self {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            pixels: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            version: 0,
            slots: HashMap::new(),
            cache: SwashCache::new(),
            pen_x: PAD,
            pen_y: PAD,
            row_h: 0,
            frame_glyphs: 0,
        }
    }
    /// Marks the start of a frame. A full atlas may only be reset here: glyphs
    /// already emitted into the current frame hold UVs into the old contents.
    pub fn start_frame(&mut self) {
        self.frame_glyphs = 0;
    }

    fn slot(&mut self, fonts: &mut FontSystem, key: CacheKey) -> Option<Slot> {
        if let Some(slot) = self.slots.get(&key) {
            return Some(*slot);
        }
        let Some(image) = self.cache.get_image(fonts, key).clone() else {
            return None;
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        if self.pen_x + w + PAD > self.width {
            self.pen_x = PAD;
            self.pen_y += self.row_h + PAD;
            self.row_h = 0;
        }
        if self.pen_y + h + PAD > self.height {
            // Full: reset only between frames, otherwise this frame's earlier
            // UVs would point at cleared or reused texels.
            if self.frame_glyphs == 0 {
                self.reset();
            } else {
                return None;
            }
        }
        if self.pen_y + h + PAD > self.height {
            return None;
        }
        for gy in 0..h {
            for gx in 0..w {
                let index = (gy * w + gx) as usize;
                let (r, g, b, a) = match image.content {
                    SwashContent::Mask => (255, 255, 255, image.data[index]),
                    SwashContent::Color => {
                        let i = index * 4;
                        (
                            image.data[i],
                            image.data[i + 1],
                            image.data[i + 2],
                            image.data[i + 3],
                        )
                    }
                    SwashContent::SubpixelMask => {
                        let i = index * 3;
                        let alpha = image.data[i..i + 3].iter().copied().max().unwrap_or(0);
                        (255, 255, 255, alpha)
                    }
                };
                let dst = (((self.pen_y + gy) * self.width + self.pen_x + gx) * 4) as usize;
                self.pixels[dst] = r;
                self.pixels[dst + 1] = g;
                self.pixels[dst + 2] = b;
                self.pixels[dst + 3] = a;
            }
        }
        let slot = Slot {
            ax: self.pen_x,
            ay: self.pen_y,
            w,
            h,
            left: image.placement.left,
            top: image.placement.top,
            uv: [
                self.pen_x as f32 / self.width as f32,
                self.pen_y as f32 / self.height as f32,
                (self.pen_x + w) as f32 / self.width as f32,
                (self.pen_y + h) as f32 / self.height as f32,
            ],
        };
        self.pen_x += w + PAD;
        self.row_h = self.row_h.max(h);
        self.frame_glyphs += 1;
        self.version = self.version.wrapping_add(1);
        self.slots.insert(key, slot);
        Some(slot)
    }

    /// Test hook: pretend the current row ended at the bottom edge.
    #[cfg(test)]
    fn force_full(&mut self) {
        self.pen_y = self.height;
    }
    fn reset(&mut self) {
        self.pixels.fill(0);
        self.slots.clear();
        self.pen_x = PAD;
        self.pen_y = PAD;
        self.row_h = 0;
        self.version = self.version.wrapping_add(1);
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Reference rasterizer: used by unit tests and only reachable from them.
pub fn rasterize(frame: &Frame, atlas: &Atlas) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(frame.width, frame.height)?;
    let bg = frame.bg;
    pixmap.fill(SkColor::from_rgba8(bg[0], bg[1], bg[2], 255));
    let width = frame.width as i32;
    let height = frame.height as i32;
    let data = pixmap.data_mut();

    for rect in &frame.rects {
        let x0 = (rect.x.round() as i32).clamp(0, width);
        let y0 = (rect.y.round() as i32).clamp(0, height);
        let x1 = ((rect.x + rect.w).round() as i32).clamp(0, width);
        let y1 = ((rect.y + rect.h).round() as i32).clamp(0, height);
        for py in y0..y1 {
            for px in x0..x1 {
                let index = ((py * width + px) * 4) as usize;
                data[index] = rect.color[0];
                data[index + 1] = rect.color[1];
                data[index + 2] = rect.color[2];
                data[index + 3] = 255;
            }
        }
    }

    for glyph in &frame.glyphs {
        for dy in 0..glyph.h as i32 {
            for dx in 0..glyph.w as i32 {
                let px = glyph.x.round() as i32 + dx;
                let py = glyph.y.round() as i32 + dy;
                if px < 0 || py < 0 || px >= width || py >= height {
                    continue;
                }
                let texel =
                    (((glyph.ay + dy as u32) * atlas.width + glyph.ax + dx as u32) * 4) as usize;
                let alpha = atlas.pixels[texel + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                let index = ((py * width + px) * 4) as usize;
                let inverse = 255 - alpha;
                for channel in 0..3 {
                    data[index + channel] = ((glyph.color[channel] as u32 * alpha
                        + data[index + channel] as u32 * inverse)
                        / 255) as u8;
                }
                data[index + 3] = (alpha + data[index + 3] as u32 * inverse / 255).min(255) as u8;
            }
        }
    }

    Some(pixmap)
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
    pub error: [u8; 3],
}

impl Theme {
    pub fn from_config(mode: &str, accent: &str) -> Self {
        let (bg, fg, muted, error) = if mode == "light" {
            (
                [0xF5, 0xF5, 0xF5],
                [0x1E, 0x1E, 0x2E],
                [0x6C, 0x6F, 0x85],
                [0xA5, 0x1C, 0x30],
            )
        } else {
            (
                [0x1E, 0x1E, 0x2E],
                [0xCD, 0xD6, 0xF4],
                [0x6C, 0x70, 0x86],
                [0xFF, 0xB4, 0xAB],
            )
        };
        Self {
            bg,
            fg,
            muted,
            accent: config::parse_color(accent),
            error,
        }
    }
}

pub struct Scene<'a> {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub theme: &'a Theme,
    pub clock: &'a str,
    pub title: &'a str,
    pub workspaces: &'a [Workspace],
    pub active_workspace: i64,
    /// Widget IDs per bar area, in draw order. Unknown IDs already failed
    /// config validation; IDs whose module does not exist yet draw nothing.
    pub left: &'a [&'a str],
    pub center: &'a [&'a str],
    pub right: &'a [&'a str],
}

pub fn build(scene: &Scene<'_>, fonts: &mut FontSystem, atlas: &mut Atlas) -> Option<Frame> {
    atlas.start_frame();
    let mut frame = Frame::new(scene.width, scene.height, scene.theme.bg)?;

    if fonts.db().is_empty() {
        // No system fonts: keep the bar usable (solid background) instead of panicking.
        return Some(frame);
    }

    let scale = scene.scale.max(1.0);
    let font_size = 13.0 * scale;
    let width = scene.width as f32;
    let edge = 8.0 * scale;
    let gap = 10.0 * scale;
    let text_y = ((scene.height as f32) - font_size * 1.3) / 2.0;
    // Centre first: the side areas only get the room it leaves.
    let center_width = area_width(fonts, scene, scene.center, font_size, scale)
        .min((width - 2.0 * edge).max(0.0));
    let center_x = ((width - center_width) / 2.0).max(edge);
    let left_budget = (center_x - edge - gap).max(0.0);
    let right_edge = width - edge;
    let right_start = (center_x + center_width + gap).min(right_edge);
    let right_budget = (right_edge - right_start).max(0.0);
    draw_area(
        &mut frame,
        fonts,
        atlas,
        scene,
        scene.left,
        edge,
        left_budget,
        font_size,
        text_y,
        scale,
        gap,
    );
    draw_area(
        &mut frame,
        fonts,
        atlas,
        scene,
        scene.center,
        center_x,
        center_width,
        font_size,
        text_y,
        scale,
        gap,
    );
    let right_width = area_width(fonts, scene, scene.right, font_size, scale).min(right_budget);
    draw_area(
        &mut frame,
        fonts,
        atlas,
        scene,
        scene.right,
        right_edge - right_width,
        right_width,
        font_size,
        text_y,
        scale,
        gap,
    );
    Some(frame)
}

/// Natural width of an area: every module that would draw, plus the gaps
/// between them. `active_window` is measured at its natural text width and is
/// ellipsized later when the budget is smaller.
fn area_width(
    fonts: &mut FontSystem,
    scene: &Scene<'_>,
    ids: &[&str],
    font_size: f32,
    scale: f32,
) -> f32 {
    let mut total = 0.0;
    let mut count = 0;
    for id in ids {
        let width = match *id {
            "workspaces" => workspaces_width(scene, fonts, font_size, scale, f32::INFINITY),
            "clock" => text_width(fonts, scene.clock, font_size),
            "active_window" if !scene.title.is_empty() => {
                text_width(fonts, scene.title, font_size)
            }
            _ => 0.0,
        };
        if width > 0.0 {
            total += width;
            count += 1;
        }
    }
    if count > 1 {
        total += 10.0 * scale * (count - 1) as f32;
    }
    total
}

#[allow(clippy::too_many_arguments)]
fn draw_area(
    frame: &mut Frame,
    fonts: &mut FontSystem,
    atlas: &mut Atlas,
    scene: &Scene<'_>,
    ids: &[&str],
    start: f32,
    budget: f32,
    font_size: f32,
    text_y: f32,
    scale: f32,
    gap: f32,
) {
    let mut x = start;
    let mut used = 0.0;
    for id in ids {
        let offset = if used > 0.0 { gap } else { 0.0 };
        let remaining = (budget - used - offset).max(0.0);
        let width = match *id {
            "workspaces" => draw_workspaces(frame, fonts, atlas, scene, x + offset, remaining, font_size, text_y, scale),
            "clock" => {
                let width = text_width(fonts, scene.clock, font_size);
                if width <= remaining {
                    frame.text(fonts, atlas, scene.clock, x + offset, text_y, font_size, scene.theme.fg);
                    width
                } else {
                    0.0
                }
            }
            "active_window" if !scene.title.is_empty() && remaining >= 16.0 * scale => {
                let title = ellipsize(scene.title, remaining, |s| text_width(fonts, s, font_size));
                if title.is_empty() {
                    0.0
                } else {
                    let width = text_width(fonts, &title, font_size);
                    frame.text(fonts, atlas, &title, x + offset, text_y, font_size, scene.theme.muted);
                    width
                }
            }
            _ => 0.0,
        };
        if width <= 0.0 {
            continue;
        }
        x += offset + width;
        used += offset + width;
    }
}

fn workspaces_width(
    scene: &Scene<'_>,
    fonts: &mut FontSystem,
    font_size: f32,
    scale: f32,
    budget: f32,
) -> f32 {
    let mut width = 0.0;
    let mut count = 0;
    for ws in scene.workspaces {
        let label = if ws.name.is_empty() {
            ws.id.to_string()
        } else {
            ws.name.clone()
        };
        let pill = text_width(fonts, &label, font_size) + 12.0 * scale;
        let offset = if count > 0 { 4.0 * scale } else { 0.0 };
        if width + offset + pill > budget {
            break;
        }
        width += offset + pill;
        count += 1;
    }
    width
}

#[allow(clippy::too_many_arguments)]
fn draw_workspaces(
    frame: &mut Frame,
    fonts: &mut FontSystem,
    atlas: &mut Atlas,
    scene: &Scene<'_>,
    start: f32,
    budget: f32,
    font_size: f32,
    text_y: f32,
    scale: f32,
) -> f32 {
    let mut x = start;
    let mut used = 0.0;
    for ws in scene.workspaces {
        let label = if ws.name.is_empty() {
            ws.id.to_string()
        } else {
            ws.name.clone()
        };
        let pill = text_width(fonts, &label, font_size) + 12.0 * scale;
        let offset = if used > 0.0 { 4.0 * scale } else { 0.0 };
        if used + offset + pill > budget {
            break;
        }
        let active = ws.id == scene.active_workspace;
        if active {
            frame.fill(
                x + offset,
                4.0 * scale,
                pill,
                scene.height as f32 - 8.0 * scale,
                scene.theme.accent,
            );
        }
        let color = if active { scene.theme.bg } else { scene.theme.fg };
        frame.pills.push(Pill {
            id: ws.id,
            x: x + offset,
            w: pill,
        });
        frame.text(
            fonts,
            atlas,
            &label,
            x + offset + 6.0 * scale,
            text_y,
            font_size,
            color,
        );
        x += offset + pill;
        used += offset + pill;
    }
    used
}
pub(crate) fn text_width(fonts: &mut FontSystem, text: &str, size: f32) -> f32 {
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0, f32::max)
}

pub fn ellipsize(text: &str, max_width: f32, mut measure: impl FnMut(&str) -> f32) -> String {
    if text.is_empty() || measure(text) <= max_width {
        return text.to_string();
    }
    if measure("…") > max_width {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut candidate: String = chars[..mid].iter().collect();
        candidate.push('…');
        if measure(&candidate) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut result: String = chars[..low].iter().collect();
    result.push('…');
    result
}

pub fn format_clock(format: &str, time: NaiveTime) -> String {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str(&format!("{:02}", time.hour())),
            Some('M') => out.push_str(&format!("{:02}", time.minute())),
            Some('S') => out.push_str(&format!("{:02}", time.second())),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_measure(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn ellipsis_fits_width() {
        assert_eq!(ellipsize("abcdef", 100.0, fake_measure), "abcdef");
        assert_eq!(ellipsize("abcdef", 45.0, fake_measure), "abc…");
        assert_eq!(ellipsize("abcdef", 10.0, fake_measure), "…");
        assert_eq!(ellipsize("abcdef", 5.0, fake_measure), "");
        let out = ellipsize("hello world", 65.0, fake_measure);
        assert!(out.ends_with('…'));
        assert!(fake_measure(&out) <= 65.0);
    }

    #[test]
    fn clock_minute_boundary() {
        let before = NaiveTime::from_hms_opt(9, 59, 59).unwrap();
        let after = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
        assert_eq!(format_clock("%H:%M", before), "09:59");
        assert_eq!(format_clock("%H:%M", after), "10:00");
        assert_eq!(format_clock("%H.%M %%", before), "09.59 %");
    }

    #[test]
    fn frame_reuses_glyphs_from_the_atlas() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let mut atlas = Atlas::new();
        let mut frame = Frame::new(120, 32, [0, 0, 0]).unwrap();
        frame.text(
            &mut fonts,
            &mut atlas,
            "09:41",
            4.0,
            4.0,
            13.0,
            [255, 255, 255],
        );
        let first = frame.glyphs.len();
        assert!(first > 0);
        let version = atlas.version;
        let mut again = Frame::new(120, 32, [0, 0, 0]).unwrap();
        again.text(
            &mut fonts,
            &mut atlas,
            "09:41",
            4.0,
            4.0,
            13.0,
            [255, 255, 255],
        );
        assert_eq!(again.glyphs.len(), first);
        assert_eq!(
            atlas.version, version,
            "second frame reused the cached glyphs"
        );
        let pixmap = rasterize(&again, &atlas).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (120, 32));
        assert!(
            pixmap
                .data()
                .chunks_exact(4)
                .any(|px| px[3] == 255 && px[0] > 0),
            "glyphs produced visible coverage"
        );
    }
    fn workspace(id: i64, name: &str) -> Workspace {
        Workspace {
            id,
            name: name.into(),
            ..Default::default()
        }
    }
    #[test]
    fn bar_areas_follow_the_configured_modules() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let theme = Theme::from_config("dark", "#8AB4F8");
        let workspaces = [workspace(1, "one"), workspace(2, "")];
        let mut atlas = Atlas::new();
        let left = ["clock"];
        let center: [&str; 0] = [];
        let right = ["workspaces"];
        let scene = Scene {
            width: 800,
            height: 32,
            scale: 1.0,
            theme: &theme,
            clock: "12:34",
            title: "Title",
            workspaces: &workspaces,
            active_workspace: 1,
            left: &left,
            center: &center,
            right: &right,
        };
        let frame = build(&scene, &mut fonts, &mut atlas).expect("frame");
        let clock_x = frame
            .glyphs
            .iter()
            .map(|glyph| glyph.x)
            .fold(f32::MAX, f32::min);
        assert!(clock_x < 60.0, "clock moved to the left area: {clock_x}");
        let accent = frame
            .rects
            .iter()
            .find(|rect| rect.color == theme.accent)
            .expect("active workspace pill");
        assert!(
            accent.x > 600.0,
            "workspaces moved to the right area: {}",
            accent.x
        );
        assert!(
            frame.glyphs.iter().all(|glyph| glyph.x + glyph.w as f32 <= 800.0),
            "no glyph crossed the right edge"
        );
    }
    #[test]
    fn workspace_hit_test_uses_the_drawn_pills() {
        let pills = [
            Pill {
                id: 3,
                x: 10.0,
                w: 20.0,
            },
            Pill {
                id: 5,
                x: 40.0,
                w: 10.0,
            },
        ];
        assert_eq!(pill_at(&pills, 10.0, 1.0), Some(3));
        assert_eq!(pill_at(&pills, 29.9, 1.0), Some(3));
        assert_eq!(pill_at(&pills, 30.0, 1.0), None, "half-open ranges");
        assert_eq!(pill_at(&pills, 45.0, 1.0), Some(5));
        assert_eq!(pill_at(&pills, 20.0, 2.0), Some(5), "logical x is scaled");
        assert_eq!(pill_at(&pills, 100.0, 1.0), None);
    }
    #[test]
    fn crowded_bar_stays_inside_the_frame() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let theme = Theme::from_config("dark", "#8AB4F8");
        let workspaces: Vec<Workspace> = (1..=12).map(|id| workspace(id, "desk")).collect();
        let mut atlas = Atlas::new();
        let left = ["launcher", "workspaces", "active_window"];
        let center = ["clock"];
        let right = [
            "media",
            "audio",
            "network",
            "battery",
            "tray",
            "notifications",
            "control_center",
            "active_window",
        ];
        let title = "A window title long enough to need ellipsis even on a wide bar";
        let scene = Scene {
            width: 320,
            height: 32,
            scale: 1.0,
            theme: &theme,
            clock: "23:59",
            title,
            workspaces: &workspaces,
            active_workspace: 3,
            left: &left,
            center: &center,
            right: &right,
        };
        let frame = build(&scene, &mut fonts, &mut atlas).expect("frame");
        for rect in &frame.rects {
            assert!(
                rect.x >= 0.0 && rect.x + rect.w <= 320.0,
                "rect escaped the frame: {rect:?}"
            );
            assert!(rect.y >= 0.0 && rect.y + rect.h <= 32.0, "rect height: {rect:?}");
        }
        for glyph in &frame.glyphs {
            assert!(
                glyph.x >= 0.0 && glyph.x + glyph.w as f32 <= 320.0,
                "glyph escaped the frame: {glyph:?}"
            );
            assert!(
                glyph.y >= 0.0 && glyph.y + glyph.h as f32 <= 32.0,
                "glyph height: {glyph:?}"
            );
        }
        for pill in &frame.pills {
            assert!(
                pill.x >= 0.0 && pill.x + pill.w <= 320.0,
                "pill escaped the frame: {pill:?}"
            );
        }
        let pixmap = rasterize(&frame, &atlas).expect("pixels");
        assert_eq!((pixmap.width(), pixmap.height()), (320, 32));
    }
    #[test]
    fn full_atlas_skips_glyphs_until_the_next_frame() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let mut atlas = Atlas::new();
        atlas.start_frame();
        let mut frame = Frame::new(120, 32, [0, 0, 0]).unwrap();
        frame.text(&mut fonts, &mut atlas, "A", 4.0, 4.0, 13.0, [255, 255, 255]);
        assert_eq!(frame.glyphs.len(), 1);
        let placed = atlas.slots.len();
        let version = atlas.version;
        atlas.force_full();
        frame.text(
            &mut fonts,
            &mut atlas,
            "B",
            20.0,
            4.0,
            13.0,
            [255, 255, 255],
        );
        assert_eq!(frame.glyphs.len(), 1, "full atlas skips a new glyph");
        assert_eq!(
            atlas.slots.len(),
            placed,
            "no reset while the frame is live"
        );
        assert_eq!(atlas.version, version, "the atlas was not wiped mid-frame");
        atlas.start_frame();
        frame.text(
            &mut fonts,
            &mut atlas,
            "B",
            20.0,
            4.0,
            13.0,
            [255, 255, 255],
        );
        assert_eq!(
            frame.glyphs.len(),
            2,
            "the next frame resets once and places"
        );
        assert_eq!(atlas.slots.len(), 1, "the reset cleared the earlier glyphs");
    }
}
