// SPDX-License-Identifier: GPL-3.0-only
//! Texture cache for icons. Widgets ask for a key while drawing; unknown keys are queued and
//! fetched after the frame, so only icons on screen are loaded.

use std::cell::RefCell;
use std::collections::HashMap;

use eframe::egui;

use crate::icons::Decoded;

/// Past this many textures the cache starts over; icons reload quickly from the disk cache.
const MAX_ENTRIES: usize = 300;

enum Slot {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

#[derive(Default)]
pub struct Icons {
    slots: RefCell<HashMap<String, Slot>>,
    wanted: RefCell<Vec<String>>,
}

impl Icons {
    /// The texture for `key` (a URL, or `file:<path>|<stamp>`) once it has loaded.
    pub fn get(&self, key: &str) -> Option<egui::TextureId> {
        let mut slots = self.slots.borrow_mut();
        match slots.get(key) {
            Some(Slot::Ready(tex)) => Some(tex.id()),
            Some(_) => None,
            None => {
                slots.insert(key.to_string(), Slot::Loading);
                self.wanted.borrow_mut().push(key.to_string());
                None
            }
        }
    }

    pub fn get_opt(&self, key: Option<&str>) -> Option<egui::TextureId> {
        key.and_then(|k| self.get(k))
    }

    pub fn take_wanted(&self) -> Vec<String> {
        std::mem::take(&mut self.wanted.borrow_mut())
    }

    pub fn finish(&self, ctx: &egui::Context, key: String, result: Option<Decoded>) {
        let mut slots = self.slots.borrow_mut();
        if slots.len() > MAX_ENTRIES {
            slots.retain(|_, s| matches!(s, Slot::Loading));
        }
        let slot = match result {
            Some(d) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(d.size, &d.rgba);
                Slot::Ready(ctx.load_texture(format!("icon-{key}"), image, egui::TextureOptions::LINEAR))
            }
            None => Slot::Failed,
        };
        slots.insert(key, slot);
    }
}
