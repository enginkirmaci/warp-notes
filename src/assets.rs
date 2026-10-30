//! Embedded asset source: our icons ship inside the binary, with gpui-kit's
//! bundled Lucide icon set as the fallback so kit components (Input
//! decorations, popovers, …) can resolve their IconName assets.

use gpui::AssetSource;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
struct Assets;

pub struct EmbedAssets {
    kit: gpui_kit::assets::Assets,
}

impl EmbedAssets {
    pub fn new() -> Self {
        Self {
            kit: gpui_kit::assets::Assets,
        }
    }
}

impl Default for EmbedAssets {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetSource for EmbedAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match Assets::get(path) {
            Some(file) => Ok(Some(file.data)),
            None => self.kit.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<gpui::SharedString>> {
        let mut ours: Vec<gpui::SharedString> = Assets::iter()
            .filter(|p| p.starts_with(path))
            .map(gpui::SharedString::from)
            .collect();
        let mut kit = self.kit.list(path)?;
        ours.append(&mut kit);
        Ok(ours)
    }
}
