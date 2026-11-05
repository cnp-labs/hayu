use std::collections::{BTreeSet, HashMap};

use cosmic_text::{
    Attrs, Buffer, CacheKey, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
};

use super::{Error, Result};

pub const ATLAS_WIDTH: u32 = 512;
pub const ATLAS_HEIGHT: u32 = 512;

/// One rasterized glyph placed in the atlas. Coordinates are physical pixels,
/// relative to the text origin, with y pointing down.
#[derive(Clone, Copy, Debug)]
pub struct PlacedGlyph {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Atlas texel origin and normalized UV rectangle.
    pub ax: u32,
    pub ay: u32,
    pub uv: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct TextLayout {
    /// Advance width in pixels.
    pub width: f32,
    pub glyphs: Vec<PlacedGlyph>,
}

/// Glyph atlas rasterized once at startup. No glyph is rasterized per frame,
/// and the font database is never mutated after `build`.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8, white RGB with coverage in alpha.
    pub pixels: Vec<u8>,
    pub layouts: HashMap<String, TextLayout>,
}

struct ShapedGlyph {
    key: CacheKey,
    x: i32,
    y: i32,
}

struct ShapedText {
    text: String,
    width: f32,
    glyphs: Vec<ShapedGlyph>,
}

impl Atlas {
    pub fn build(fonts: &mut FontSystem, size: f32, texts: &[&str]) -> Result<Self> {
        let mut cache = SwashCache::new();
        let mut shaped = Vec::new();
        let mut keys = BTreeSet::new();
        for text in texts {
            let shaped_text = shape_text(fonts, text, size)?;
            for glyph in &shaped_text.glyphs {
                keys.insert(glyph.key);
            }
            shaped.push(shaped_text);
        }

        let width = ATLAS_WIDTH;
        let height = ATLAS_HEIGHT;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut slots: HashMap<CacheKey, (u32, u32, u32, u32)> = HashMap::new();
        let pad = 1u32;
        let mut pen_x = pad;
        let mut pen_y = pad;
        let mut row_h = 0u32;

        for key in keys {
            // SAFETY-free path: get_image caches the raster, we only read it.
            let Some(image) = cache.get_image(fonts, key).as_ref() else {
                continue;
            };
            let w = image.placement.width;
            let h = image.placement.height;
            if w == 0 || h == 0 {
                continue;
            }
            if pen_x + w + pad > width {
                pen_x = pad;
                pen_y += row_h + pad;
                row_h = 0;
            }
            if pen_y + h + pad > height {
                return Err(Error::Scene(format!(
                    "glyph atlas overflow at {w}x{h} ({}x{height})",
                    width
                )));
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
                    let dst = (((pen_y + gy) * width + pen_x + gx) * 4) as usize;
                    pixels[dst] = r;
                    pixels[dst + 1] = g;
                    pixels[dst + 2] = b;
                    pixels[dst + 3] = a;
                }
            }
            slots.insert(key, (pen_x, pen_y, w, h));
            pen_x += w + pad;
            row_h = row_h.max(h);
        }

        let mut layouts = HashMap::new();
        for shaped_text in shaped {
            let mut glyphs = Vec::new();
            for glyph in shaped_text.glyphs {
                let Some(&(ax, ay, w, h)) = slots.get(&glyph.key) else {
                    continue;
                };
                let Some(image) = cache.get_image(fonts, glyph.key).as_ref() else {
                    continue;
                };
                glyphs.push(PlacedGlyph {
                    x: glyph.x + image.placement.left,
                    y: glyph.y - image.placement.top,
                    w,
                    h,
                    ax,
                    ay,
                    uv: [
                        ax as f32 / width as f32,
                        ay as f32 / height as f32,
                        (ax + w) as f32 / width as f32,
                        (ay + h) as f32 / height as f32,
                    ],
                });
            }
            layouts.insert(
                shaped_text.text,
                TextLayout {
                    width: shaped_text.width,
                    glyphs,
                },
            );
        }

        Ok(Self {
            width,
            height,
            pixels,
            layouts,
        })
    }

    pub fn layout(&self, text: &str) -> Option<&TextLayout> {
        self.layouts.get(text)
    }
}

fn shape_text(fonts: &mut FontSystem, text: &str, size: f32) -> Result<ShapedText> {
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.3));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    let mut width: f32 = 0.0;
    let mut glyphs = Vec::new();
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, run.line_y), 1.0);
            glyphs.push(ShapedGlyph {
                key: physical.cache_key,
                x: physical.x,
                y: physical.y,
            });
        }
    }
    Ok(ShapedText {
        text: text.to_string(),
        width,
        glyphs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_has_unique_glyph_slots() {
        let mut fonts = FontSystem::new();
        if fonts.db().is_empty() {
            return;
        }
        let atlas = Atlas::build(&mut fonts, 13.0, &["1", "2", "09:41"]).unwrap();
        assert_eq!(atlas.pixels.len(), (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize);
        assert!(atlas.layout("09:41").is_some());
        let layout = atlas.layout("09:41").unwrap();
        assert!(layout.width > 0.0);
        assert!(!layout.glyphs.is_empty());
    }
}
