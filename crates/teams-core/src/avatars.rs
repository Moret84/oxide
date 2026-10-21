//! Small session-only photo cache; Swift takes each decoded image once.
use crate::app::*;
use crate::media::{self, LoadedImage};
#[derive(Debug)]
enum State {
    Pending { mri: String, name: String },
    Loading,
    Ready(LoadedImage),
    Failed,
}
/// Requests waiting for a download; more are ignored until the queue drains.
const MAX_QUEUED: usize = 256;
/// Photos Swift may hold (64 KiB each); the least recently asked for are evicted.
const MAX_READY: usize = 256;
#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<String, State>,
    /// Ready ids, least recently requested first.
    ready: std::collections::VecDeque<String>,
}
impl Cache {
    pub fn request(&mut self, id: String, mri: String, name: String) {
        if matches!(self.entries.get(&id), Some(State::Ready(_))) {
            self.touch(&id);
            return;
        }
        let queued = self
            .entries
            .values()
            .filter(|s| matches!(s, State::Pending { .. } | State::Loading))
            .count();
        if queued < MAX_QUEUED {
            self.entries
                .entry(id)
                .or_insert(State::Pending { mri, name });
        }
    }
    fn touch(&mut self, id: &str) {
        if let Some(index) = self.ready.iter().position(|r| r == id) {
            let id = self.ready.remove(index).expect("index in bounds");
            self.ready.push_back(id);
        }
    }
    pub fn jobs(&mut self, client: Client, generation: u64) -> Task<Msg> {
        let active = self
            .entries
            .values()
            .filter(|s| matches!(s, State::Loading))
            .count();
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, s)| matches!(s, State::Pending { .. }))
            .take(4usize.saturating_sub(active))
            .map(|(id, _)| id.clone())
            .collect();
        Task::batch(ids.into_iter().map(|id| {
            let Some(State::Pending { mri, name }) =
                self.entries.insert(id.clone(), State::Loading)
            else {
                return Task::none();
            };
            let client = client.clone();
            Task::perform(
                async move {
                    let bytes = client.profile_picture(&mri, &name).await.map_err(|_| ())?;
                    tokio::task::spawn_blocking(move || media::decode_avatar(&bytes))
                        .await
                        .map_err(|_| ())?
                        .map_err(|_| ())
                },
                move |result| Msg::AvatarLoaded {
                    generation,
                    id,
                    result,
                },
            )
        }))
    }
    pub fn finish(&mut self, id: String, result: Result<LoadedImage, ()>) {
        let ready = result.is_ok();
        self.entries.insert(
            id.clone(),
            result.map(State::Ready).unwrap_or(State::Failed),
        );
        if ready {
            self.ready.retain(|r| r != &id);
            self.ready.push_back(id);
        }
        // An evicted id leaves `ready_ids`, so Swift drops its photo and a later request re-downloads it.
        while self.ready.len() > MAX_READY {
            if let Some(oldest) = self.ready.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
    pub fn ready_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, s)| matches!(s, State::Ready(_)))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }
    pub fn take(&mut self, id: &str) -> Option<(u32, u32, Vec<u8>)> {
        let State::Ready(image) = self.entries.get_mut(id)? else {
            return None;
        };
        let rgba = std::mem::take(&mut image.rgba);
        if rgba.is_empty() {
            return None;
        }
        Some((
            image.width,
            image.height,
            Arc::try_unwrap(rgba).unwrap_or_else(|v| v.as_ref().clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_cache_is_bounded_deduplicates_and_hands_pixels_over_once() {
        let mut cache = Cache::default();
        cache.request("alice".into(), "8:orgid:alice".into(), "Alice".into());
        cache.request("alice".into(), "8:orgid:alice".into(), "Alice".into());
        assert_eq!(cache.entries.len(), 1);
        let mut bytes = Vec::new();
        image::RgbImage::new(400, 400)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let photo = media::decode_avatar(&bytes).unwrap();
        assert_eq!((photo.width, photo.height), (128, 128));
        cache.finish("alice".into(), Ok(photo));
        assert_eq!(cache.ready_ids(), vec!["alice"]);
        assert!(cache.take("alice").is_some());
        assert!(cache.take("alice").is_none());
        cache.finish("missing".into(), Err(()));
        cache.request("missing".into(), "8:orgid:missing".into(), "".into());
        assert!(matches!(cache.entries["missing"], State::Failed));
        for id in 0..300 {
            cache.request(id.to_string(), "8:orgid:test".into(), "Test".into());
        }
        // The queue is bounded; finished photos do not count against it.
        assert_eq!(cache.entries.len(), MAX_QUEUED + 2);
    }
    #[test]
    fn ready_photos_are_bounded_and_the_least_recently_asked_for_go_first() {
        let photo = || LoadedImage {
            width: 1,
            height: 1,
            rgba: Arc::default(),
            bytes: 4,
        };
        let mut cache = Cache::default();
        for id in 0..MAX_READY {
            cache.finish(id.to_string(), Ok(photo()));
        }
        // Asking for "0" again keeps it; "1" is now the oldest.
        cache.request("0".into(), "8:orgid:0".into(), "Zéro".into());
        cache.finish("new".into(), Ok(photo()));
        let ids = cache.ready_ids();
        assert_eq!(ids.len(), MAX_READY);
        assert!(ids.contains(&"0".to_owned()) && ids.contains(&"new".to_owned()));
        assert!(!ids.contains(&"1".to_owned()));
        // Long after the first photos, new ones still load.
        cache.request("later".into(), "8:orgid:later".into(), "Plus tard".into());
        assert!(matches!(cache.entries["later"], State::Pending { .. }));
    }
}
