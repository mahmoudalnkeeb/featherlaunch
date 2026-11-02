// SPDX-License-Identifier: GPL-3.0-only
//! Skin download (cached) and a front view composed from the skin texture.

use anyhow::{Result, bail};
use image::RgbaImage;

use crate::net;
use crate::paths;

/// Pixels per skin pixel. A whole number, so the texture stays crisp at its display size.
const SCALE: i32 = 6;
/// Empty skin pixels around the body, for the outer layer to stick out into.
const MARGIN: i32 = 1;
/// How far the outer layer stands off the body in the game, in preview pixels.
const HAT_RAISE: i32 = 3;
const CLOTHES_RAISE: i32 = 2;
/// Darkens the body under the outer layer, which is what makes it read as raised.
const SHADOW: u8 = 70;
pub const FRONT_W: u32 = ((16 + 2 * MARGIN) * SCALE) as u32;
pub const FRONT_H: u32 = ((32 + 2 * MARGIN) * SCALE) as u32;

/// Skin textures are immutable per URL, so a cached copy never goes stale.
pub async fn fetch(url: &str) -> Result<Vec<u8>> {
    let cache = paths::cache().join("skins").join(format!("{}.png", net::sha1_bytes(url.as_bytes())));
    if let Ok(bytes) = std::fs::read(&cache) {
        return Ok(bytes);
    }
    let bytes = net::client().get(url).send().await?.error_for_status()?.bytes().await?.to_vec();
    std::fs::create_dir_all(cache.parent().unwrap())?;
    std::fs::write(&cache, &bytes)?;
    Ok(bytes)
}

/// `top` over `bottom`, so a see-through pixel tints what's under it.
fn over(bottom: [u8; 4], top: [u8; 4]) -> [u8; 4] {
    let a = u32::from(top[3]);
    let inverse = 255 - a;
    let mix = |t: u8, b: u8| ((u32::from(t) * a + u32::from(b) * inverse) / 255) as u8;
    [mix(top[0], bottom[0]), mix(top[1], bottom[1]), mix(top[2], bottom[2]), (a + u32::from(bottom[3]) * inverse / 255) as u8]
}

/// A part of the skin and where it goes, in skin pixels on a body 16 wide and 32 tall.
struct Part {
    source: (u32, u32, u32, u32),
    at: (i32, i32),
    mirror: bool,
}

const fn part(sx: u32, sy: u32, w: u32, h: u32, x: i32, y: i32) -> Part {
    Part { source: (sx, sy, w, h), at: (x, y), mirror: false }
}

/// Calls `pixel` with each preview pixel of `part`, and the skin pixel that shows there.
fn each_pixel(part: &Part, skin: &RgbaImage, raise: i32, mut pixel: impl FnMut(u32, u32, [u8; 4])) {
    let (sx, sy, w, h) = part.source;
    let (left, top) = ((part.at.0 + MARGIN) * SCALE - raise, (part.at.1 + MARGIN) * SCALE - raise);
    let (width, height) = (w as i32 * SCALE + 2 * raise, h as i32 * SCALE + 2 * raise);
    for y in 0..height {
        for x in 0..width {
            let tx = (x * w as i32 / width) as u32;
            let ty = (y * h as i32 / height) as u32;
            let tx = if part.mirror { w - 1 - tx } else { tx };
            pixel((left + x) as u32, (top + y) as u32, skin.get_pixel(sx + tx, sy + ty).0);
        }
    }
}

fn paint_base(front: &mut RgbaImage, skin: &RgbaImage, part: &Part) {
    each_pixel(part, skin, 0, |x, y, [r, g, b, _]| front.put_pixel(x, y, image::Rgba([r, g, b, 255])));
}

fn paint_outer(front: &mut RgbaImage, skin: &RgbaImage, part: &Part, raise: i32) {
    each_pixel(part, skin, raise, |x, y, p| {
        if p[3] == 0 {
            return;
        }
        // The shadow falls just below, on the body but not on the empty background.
        let below = y + raise as u32;
        if below < FRONT_H && front.get_pixel(x, below)[3] == 255 {
            let shaded = over(front.get_pixel(x, below).0, [0, 0, 0, SHADOW]);
            front.put_pixel(x, below, image::Rgba(shaded));
        }
    });
    each_pixel(part, skin, raise, |x, y, p| {
        if p[3] != 0 {
            let blended = over(front.get_pixel(x, y).0, p);
            front.put_pixel(x, y, image::Rgba(blended));
        }
    });
}

/// The 8×8 face with the hat on it, for small places like the title bar.
fn head(skin: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let [r, g, b, _] = skin.get_pixel(8 + x, 8 + y).0;
            out.extend(over([r, g, b, 255], skin.get_pixel(40 + x, 8 + y).0));
        }
    }
    out
}

/// Front view ([`FRONT_W`]×[`FRONT_H`]) and head (8×8) as RGBA bytes. The outer layer is drawn
/// slightly bigger than the body with a shadow under it, because that's how it stands out in game.
pub fn compose(png: &[u8], slim: bool) -> Result<(Vec<u8>, Vec<u8>)> {
    let skin = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (w, h) = skin.dimensions();
    if w != 64 || (h != 64 && h != 32) {
        bail!("unsupported skin size {w}x{h}");
    }
    let modern = h == 64;
    let arm = if slim && modern { 3 } else { 4 };
    let right_arm = 4 - arm as i32;
    let mut front = RgbaImage::new(FRONT_W, FRONT_H);

    let mut base = vec![part(8, 8, 8, 8, 4, 0), part(20, 20, 8, 12, 4, 8), part(44, 20, arm, 12, right_arm, 8), part(4, 20, 4, 12, 4, 20)];
    if modern {
        base.extend([part(36, 52, arm, 12, 12, 8), part(20, 52, 4, 12, 8, 20)]);
    } else {
        // 64×32 skins have one arm and one leg; the other side is the mirror image.
        base.push(Part { mirror: true, ..part(44, 20, 4, 12, 12, 8) });
        base.push(Part { mirror: true, ..part(4, 20, 4, 12, 8, 20) });
    }
    for piece in &base {
        paint_base(&mut front, &skin, piece);
    }
    if modern {
        // Legs and arms first, because the jacket and hat sit over them.
        let clothes = [
            part(4, 36, 4, 12, 4, 20),
            part(4, 52, 4, 12, 8, 20),
            part(44, 36, arm, 12, right_arm, 8),
            part(52, 52, arm, 12, 12, 8),
            part(20, 36, 8, 12, 4, 8),
        ];
        for piece in &clothes {
            paint_outer(&mut front, &skin, piece, CLOTHES_RAISE);
        }
    }
    paint_outer(&mut front, &skin, &part(40, 8, 8, 8, 4, 0), HAT_RAISE);
    Ok((front.into_raw(), head(&skin)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn pixel(front: &[u8], x: i32, y: i32) -> [u8; 4] {
        let at = (y as usize * FRONT_W as usize + x as usize) * 4;
        front[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn composes_head_from_face_region() {
        // Face pixels red, everything else transparent.
        let bytes = png(64, 64, |x, y| if (8..16).contains(&x) && (8..16).contains(&y) { [255, 0, 0, 255] } else { [0, 0, 0, 0] });
        let (front, head) = compose(&bytes, false).unwrap();
        assert_eq!(front.len(), (FRONT_W * FRONT_H * 4) as usize);
        assert_eq!(head.len(), 8 * 8 * 4);
        assert!(head.chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn the_hat_stands_out_past_the_head() {
        let in_hat = |x: u32, y: u32| (40..48).contains(&x) && (8..16).contains(&y);
        let in_face = |x: u32, y: u32| (8..16).contains(&x) && (8..16).contains(&y);
        let bytes = png(64, 64, |x, y| {
            if in_hat(x, y) {
                [0, 255, 0, 255]
            } else if in_face(x, y) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let (front, _) = compose(&bytes, false).unwrap();
        let (left, top) = ((4 + MARGIN) * SCALE, MARGIN * SCALE);
        assert_eq!(pixel(&front, left - 1, top + 12), [0, 255, 0, 255], "the hat reaches past the head's edge");
        assert_eq!(pixel(&front, left + 12, top + 12), [0, 255, 0, 255]);
    }

    #[test]
    fn the_jacket_shows_on_the_body() {
        let bytes = png(64, 64, |x, y| {
            if (20..28).contains(&x) && (36..48).contains(&y) {
                [0, 0, 255, 255]
            } else if (20..28).contains(&x) && (20..32).contains(&y) {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 0]
            }
        });
        let (front, _) = compose(&bytes, false).unwrap();
        assert_eq!(pixel(&front, (8 + MARGIN) * SCALE, (14 + MARGIN) * SCALE), [0, 0, 255, 255]);
    }

    #[test]
    fn see_through_outer_pixels_blend() {
        assert_eq!(over([0, 0, 0, 255], [255, 255, 255, 0]), [0, 0, 0, 255]);
        assert_eq!(over([0, 0, 0, 255], [255, 255, 255, 255]), [255, 255, 255, 255]);
        let half = over([0, 0, 0, 255], [255, 255, 255, 128]);
        assert_eq!((half[0], half[3]), (128, 255));
    }

    #[test]
    fn old_skins_mirror_their_arm_and_leg() {
        let bytes = png(64, 32, |x, y| if (44..48).contains(&x) && (20..32).contains(&y) { [9, 9, 9, 255] } else { [1, 1, 1, 255] });
        let (front, _) = compose(&bytes, false).unwrap();
        assert_eq!(pixel(&front, (13 + MARGIN) * SCALE, (14 + MARGIN) * SCALE), [9, 9, 9, 255]);
    }

    #[test]
    fn rejects_wrong_sizes() {
        assert!(compose(&png(32, 32, |_, _| [0; 4]), false).is_err());
        assert!(compose(&png(64, 32, |_, _| [0; 4]), false).is_ok());
    }
}
