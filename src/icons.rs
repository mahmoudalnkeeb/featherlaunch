// SPDX-License-Identifier: GPL-3.0-only
//! Project and instance icons. Every icon is shrunk to a small square before it reaches the
//! GPU and cached on disk at that size, so a screen full of icons costs a few hundred KB.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};
use tokio::sync::Semaphore;

use super::{net, paths};

pub const SIZE: u32 = 64;
const INSTANCE_ICON_SIZE: u32 = 128;
pub const INSTANCE_ICON: &str = "icon.png";

/// Icon downloads share bandwidth with game downloads; keep them few.
static LIMIT: Semaphore = Semaphore::const_new(6);

pub struct Decoded {
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
}

fn square(img: DynamicImage, size: u32) -> RgbaImage {
    img.resize_to_fill(size, size, FilterType::Triangle).to_rgba8()
}

fn decoded(img: RgbaImage) -> Decoded {
    let (w, h) = img.dimensions();
    Decoded { size: [w as usize, h as usize], rgba: img.into_raw() }
}

pub async fn fetch(url: &str) -> Result<Decoded> {
    let _permit = LIMIT.acquire().await?;
    let path = paths::cache().join("icons").join(format!("{}.png", net::sha1_bytes(url.as_bytes())));
    if let Ok(bytes) = tokio::fs::read(&path).await {
        let cached = tokio::task::spawn_blocking(move || image::load_from_memory_with_format(&bytes, ImageFormat::Png)).await?;
        if let Ok(img) = cached {
            return Ok(decoded(img.to_rgba8()));
        }
    }
    let bytes = net::client().get(url).send().await?.error_for_status()?.bytes().await?;
    tokio::task::spawn_blocking(move || -> Result<Decoded> {
        let img = square(image::load_from_memory(&bytes)?, SIZE);
        std::fs::create_dir_all(path.parent().unwrap())?;
        img.save_with_format(&path, ImageFormat::Png)?;
        Ok(decoded(img))
    })
    .await?
}

pub async fn load_file(path: PathBuf) -> Result<Decoded> {
    tokio::task::spawn_blocking(move || Ok(decoded(square(image::open(&path)?, SIZE)))).await?
}

pub fn set_instance_icon(bytes: &[u8], inst_dir: &Path) -> Result<()> {
    let img = square(image::load_from_memory(bytes)?, INSTANCE_ICON_SIZE);
    img.save_with_format(inst_dir.join(INSTANCE_ICON), ImageFormat::Png)?;
    Ok(())
}

/// Makes an image file the instance's icon. Reads and decodes it, so call it off the UI thread.
pub fn set_instance_icon_file(path: &Path, inst_dir: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    set_instance_icon(&bytes, inst_dir).with_context(|| format!("using {} as an icon", path.display()))
}

pub async fn download_instance_icon(url: &str, inst_dir: PathBuf) -> Result<()> {
    let bytes = net::client().get(url).send().await?.error_for_status()?.bytes().await?;
    tokio::task::spawn_blocking(move || set_instance_icon(&bytes, &inst_dir)).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_become_squares() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, image::Rgba([10, 20, 30, 255])));
        let sq = square(img, SIZE);
        assert_eq!(sq.dimensions(), (SIZE, SIZE));
        let d = decoded(sq);
        assert_eq!(d.rgba.len(), (SIZE * SIZE * 4) as usize);
    }
}
