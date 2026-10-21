//! Bounded in-memory image decoding. The renderer receives pixels, not network URLs.
use crate::app::*;
use base64::Engine;
use teams_i18n::t;
/// Previews render in a 420×250 box; keep 2× for Retina and drop the rest.
const PREVIEW_W: u32 = 840;
const PREVIEW_H: u32 = 500;
pub const MAX_IMAGES: usize = 48;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
#[derive(Debug, Clone)]
pub enum ImageState {
    Loading,
    Ready(LoadedImage),
    Failed(String),
}
#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub width: u32,
    pub height: u32,
    /// Empty once handed to [`Handoff`]: the state only keeps the size.
    pub rgba: Arc<Vec<u8>>,
    pub bytes: usize,
}
/// Ready preview pixels and voicemail audio awaiting Swift, behind their own lock so taking them never waits on an update.
#[derive(Debug, Default)]
pub struct Handoff {
    images: HashMap<String, (u32, u32, Arc<Vec<u8>>)>,
    /// Id, content type and bytes.
    voicemail: Option<(String, String, Vec<u8>)>,
}
impl Handoff {
    /// Moves the pixels of a ready preview here; the state keeps its size for the budget.
    pub fn offer(&mut self, src: String, image: &mut LoadedImage) {
        let rgba = std::mem::take(&mut image.rgba);
        self.images.insert(src, (image.width, image.height, rgba));
    }
    /// Swift has not taken this preview yet.
    pub fn waiting(&self, src: &str) -> bool {
        self.images.contains_key(src)
    }
    pub fn withdraw(&mut self, src: &str) {
        self.images.remove(src);
    }
    pub fn take_image(&mut self, src: &str) -> Option<(u32, u32, Vec<u8>)> {
        let (width, height, rgba) = self.images.remove(src)?;
        let pixels = Arc::try_unwrap(rgba).unwrap_or_else(|shared| shared.as_ref().clone());
        Some((width, height, pixels))
    }
    pub fn offer_voicemail(&mut self, id: String, kind: String, data: Vec<u8>) {
        self.voicemail = Some((id, kind, data));
    }
    pub fn withdraw_voicemail(&mut self) {
        self.voicemail = None;
    }
    pub fn take_voicemail(&mut self, id: &str) -> Option<(String, Vec<u8>)> {
        if self.voicemail.as_ref().is_none_or(|(a, _, _)| a != id) {
            return None;
        }
        self.voicemail.take().map(|(_, kind, data)| (kind, data))
    }
}
/// Previews of closed conversations (LRU first) whose pixels Swift keeps; they share `MAX_IMAGE_BYTES` and are evicted first.
#[derive(Debug, Default)]
pub struct Retained {
    entries: Vec<(String, LoadedImage)>,
    bytes: usize,
}
impl Retained {
    pub fn keep(&mut self, src: String, image: LoadedImage) {
        self.take(&src);
        self.bytes += image.bytes;
        self.entries.push((src, image));
    }
    pub fn take(&mut self, src: &str) -> Option<LoadedImage> {
        let index = self.entries.iter().position(|(s, _)| s == src)?;
        let (_, image) = self.entries.remove(index);
        self.bytes -= image.bytes;
        Some(image)
    }
    /// Drops the oldest previews until at most `budget` bytes remain.
    pub fn evict_to(&mut self, budget: usize) {
        let mut evicted = 0;
        while self.bytes > budget && evicted < self.entries.len() {
            self.bytes -= self.entries[evicted].1.bytes;
            evicted += 1;
        }
        self.entries.drain(..evicted);
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn srcs(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().map(|(src, _)| src)
    }
}
pub async fn load(client: Client, src: String) -> Result<LoadedImage, String> {
    let bytes = if src.starts_with("data:image/") {
        let (meta, data) = src
            .split_once(',')
            .ok_or(t("core.media.invalid_image_data"))?;
        if !meta.ends_with(";base64") || data.len() > 12 * 1024 * 1024 {
            return Err(t("core.media.unsupported_embedded_image_format").into());
        }
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| t("core.media.invalid_image_data"))?
    } else {
        client
            .download_image(&src)
            .await
            .map_err(|e| e.to_string())?
    };
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|_| t("core.media.image_decoding_interrupted").to_owned())?
}
fn decode(bytes: &[u8]) -> Result<LoadedImage, String> {
    decode_sized(bytes, PREVIEW_W, PREVIEW_H)
}
pub(crate) fn decode_avatar(bytes: &[u8]) -> Result<LoadedImage, String> {
    decode_sized(bytes, 128, 128)
}
fn decode_sized(bytes: &[u8], max_width: u32, max_height: u32) -> Result<LoadedImage, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| t("core.media.unknown_image_format"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| t("core.media.invalid_image_or_unsupported_format"))?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 8_000_000 {
        return Err(t("core.media.image_too_large_to_preview").into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| t("core.media.unknown_image_format"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(48 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader
        .decode()
        .map_err(|_| t("core.media.the_image_cannot_be_decoded"))?;
    if image.width() > max_width || image.height() > max_height {
        image = image.thumbnail(max_width, max_height);
    }
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    premultiply(&mut pixels);
    let bytes = pixels.len();
    Ok(LoadedImage {
        width,
        height,
        rgba: Arc::new(pixels),
        bytes,
    })
}
/// Core Graphics would otherwise convert straight alpha on every draw.
fn premultiply(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn image(bytes: usize) -> LoadedImage {
        LoadedImage {
            width: 1,
            height: 1,
            rgba: Arc::default(),
            bytes,
        }
    }
    #[test]
    fn retained_previews_evict_the_least_recently_shown_first() {
        let mut retained = Retained::default();
        retained.keep("a".into(), image(10));
        retained.keep("b".into(), image(10));
        retained.keep("c".into(), image(10));
        // Shown again: "a" becomes the most recent.
        let a = retained.take("a").unwrap();
        retained.keep("a".into(), a);
        retained.evict_to(20);
        assert_eq!(retained.srcs().collect::<Vec<_>>(), ["c", "a"]);
        assert_eq!(retained.bytes(), 20);
        retained.evict_to(0);
        assert_eq!(retained.srcs().count(), 0);
        assert_eq!(retained.bytes(), 0);
    }
    #[test]
    fn pixels_are_premultiplied() {
        let mut pixels = [200, 100, 50, 255, 200, 100, 50, 128, 255, 255, 255, 0];
        premultiply(&mut pixels);
        assert_eq!(pixels, [200, 100, 50, 255, 100, 50, 25, 128, 0, 0, 0, 0]);
    }
    #[test]
    fn large_images_are_downscaled_to_preview_size() {
        let mut png = Vec::new();
        image::RgbImage::new(3000, 2000)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let loaded = decode(&png).unwrap();
        assert!(loaded.bytes > 0 && loaded.bytes <= (PREVIEW_W * PREVIEW_H * 4) as usize);
    }
}
