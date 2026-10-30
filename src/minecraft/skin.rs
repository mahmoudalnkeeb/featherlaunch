// SPDX-License-Identifier: GPL-3.0-only
//! Skin download (cached) and a flat front view composed from the skin texture.

use anyhow::{Result, bail};
use image::{RgbaImage, imageops};

use crate::net;
use crate::paths;

pub const FRONT_W: u32 = 16;
pub const FRONT_H: u32 = 32;

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

fn blit(dst: &mut RgbaImage, src: &RgbaImage, (sx, sy, w, h): (u32, u32, u32, u32), (dx, dy): (u32, u32), mirror: bool, overlay: bool) {
    for y in 0..h {
        for x in 0..w {
            let fx = if mirror { w - 1 - x } else { x };
            let p = *src.get_pixel(sx + fx, sy + y);
            if overlay && p[3] == 0 {
                continue;
            }
            dst.put_pixel(dx + x, dy + y, if overlay { p } else { image::Rgba([p[0], p[1], p[2], 255]) });
        }
    }
}

/// Front view (16×32) and head (8×8) as RGBA bytes, including the outer overlay layer.
pub fn compose(png: &[u8], slim: bool) -> Result<(Vec<u8>, Vec<u8>)> {
    let skin = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.to_rgba8();
    let (w, h) = skin.dimensions();
    if w != 64 || (h != 64 && h != 32) {
        bail!("unsupported skin size {w}x{h}");
    }
    let modern = h == 64;
    let arm = if slim && modern { 3 } else { 4 };
    let mut front = RgbaImage::new(FRONT_W, FRONT_H);

    blit(&mut front, &skin, (8, 8, 8, 8), (4, 0), false, false);
    blit(&mut front, &skin, (20, 20, 8, 12), (4, 8), false, false);
    blit(&mut front, &skin, (44, 20, arm, 12), (4 - arm, 8), false, false);
    blit(&mut front, &skin, (4, 20, 4, 12), (4, 20), false, false);
    if modern {
        blit(&mut front, &skin, (36, 52, arm, 12), (12, 8), false, false);
        blit(&mut front, &skin, (20, 52, 4, 12), (8, 20), false, false);
        // Second layer: jacket, sleeves, pants.
        blit(&mut front, &skin, (20, 36, 8, 12), (4, 8), false, true);
        blit(&mut front, &skin, (44, 36, arm, 12), (4 - arm, 8), false, true);
        blit(&mut front, &skin, (52, 52, arm, 12), (12, 8), false, true);
        blit(&mut front, &skin, (4, 36, 4, 12), (4, 20), false, true);
        blit(&mut front, &skin, (4, 52, 4, 12), (8, 20), false, true);
    } else {
        // 64×32 skins have one arm and one leg; the other side is the mirror image.
        blit(&mut front, &skin, (44, 20, 4, 12), (12, 8), true, false);
        blit(&mut front, &skin, (4, 20, 4, 12), (8, 20), true, false);
    }
    blit(&mut front, &skin, (40, 8, 8, 8), (4, 0), false, true);

    let head = imageops::crop_imm(&front, 4, 0, 8, 8).to_image();
    Ok((front.into_raw(), head.into_raw()))
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
    fn rejects_wrong_sizes() {
        assert!(compose(&png(32, 32, |_, _| [0; 4]), false).is_err());
        assert!(compose(&png(64, 32, |_, _| [0; 4]), false).is_ok());
    }
}
