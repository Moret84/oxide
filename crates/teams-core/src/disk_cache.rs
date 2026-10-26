//! Downloaded pictures kept in the caches folder between launches: reopening a chat or relaunching reads them instead of downloading again.
use crate::emoji;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

static DIR: OnceLock<PathBuf> = OnceLock::new();
/// Previews beyond this are deleted at launch, the least recently shown first.
const MAX_PREVIEW_BYTES: u64 = 200 * 1024 * 1024;
/// Photos nobody was shown for a month are deleted at launch.
const PHOTO_KEEP: Duration = Duration::from_secs(30 * 86_400);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// Message pictures, link and file thumbnails: their address names fixed content.
    Preview,
    /// Profile photos and app icons, which change: older than a week, they are downloaded again.
    Photo,
}
impl Kind {
    fn folder(self) -> &'static str {
        match self {
            Kind::Preview => "previews",
            Kind::Photo => "photos",
        }
    }
}

/// Where pictures are kept; without it every one is downloaded. Prunes what earlier launches left.
pub fn set_dir(path: PathBuf) {
    if DIR.set(path.clone()).is_ok() {
        std::thread::spawn(move || prune(&path));
    }
}

fn path(kind: Kind, key: &str) -> Option<PathBuf> {
    let name: String = Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Some(DIR.get()?.join(kind.folder()).join(name))
}

/// A preview's bytes, marked as just shown so pruning keeps it longest.
pub(crate) async fn preview(key: &str) -> Option<Vec<u8>> {
    let path = path(Kind::Preview, key)?;
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path).ok()?;
        if let Ok(file) = std::fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(bytes)
    })
    .await
    .ok()?
}

/// A photo's bytes, and whether they are younger than the web client's one-week refresh.
pub(crate) async fn photo(key: &str) -> Option<(Vec<u8>, bool)> {
    let path = path(Kind::Photo, key)?;
    let bytes = tokio::fs::read(&path).await.ok()?;
    Some((bytes, emoji::fresh(&path)))
}

pub(crate) async fn store(kind: Kind, key: &str, bytes: &[u8]) {
    if let Some(path) = path(kind, key) {
        emoji::save(&path, bytes, "picture").await;
    }
}

/// Signing out leaves no picture of the account's chats behind.
pub(crate) fn clear() {
    std::thread::spawn(clear_now);
}

/// Deletes every kept picture; Swift still shows the ones it decoded.
pub(crate) fn clear_now() {
    let Some(dir) = DIR.get() else {
        return;
    };
    for kind in [Kind::Preview, Kind::Photo] {
        let _ = std::fs::remove_dir_all(dir.join(kind.folder()));
    }
}

/// Files and bytes kept for each kind, for Settings › Debug.
pub(crate) fn usage(kind: Kind) -> (u64, u64) {
    let Some(dir) = DIR.get() else {
        return (0, 0);
    };
    files(dir, kind)
        .iter()
        .fold((0, 0), |(count, bytes), (_, _, len)| {
            (count + 1, bytes + len)
        })
}

fn files(dir: &Path, kind: Kind) -> Vec<(PathBuf, SystemTime, u64)> {
    std::fs::read_dir(dir.join(kind.folder()))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            meta.is_file().then(|| (entry.path(), modified, meta.len()))
        })
        .collect()
}

fn prune(dir: &Path) {
    let mut previews = files(dir, Kind::Preview);
    previews.sort_by_key(|(_, modified, _)| std::cmp::Reverse(*modified));
    let mut kept = 0;
    for (path, _, len) in previews {
        kept += len;
        if kept > MAX_PREVIEW_BYTES {
            let _ = std::fs::remove_file(path);
        }
    }
    for (path, modified, _) in files(dir, Kind::Photo) {
        if modified.elapsed().is_ok_and(|age| age > PHOTO_KEEP) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &Path, len: u64, age: Duration) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        file.set_len(len).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn pruning_drops_the_least_recently_shown_previews_and_month_old_photos() {
        let dir = std::env::temp_dir().join(format!("oxide-cache-{}", uuid::Uuid::new_v4()));
        let previews = dir.join("previews");
        let photos = dir.join("photos");
        let half = MAX_PREVIEW_BYTES / 2;
        file(&previews.join("new"), half, Duration::ZERO);
        file(&previews.join("recent"), half, Duration::from_secs(60));
        file(&previews.join("old"), 1, Duration::from_secs(3600));
        file(&photos.join("stale"), 1, Duration::from_secs(8 * 86_400));
        file(
            &photos.join("gone"),
            1,
            PHOTO_KEEP + Duration::from_secs(60),
        );
        prune(&dir);
        let names = |dir: &Path| {
            let mut names: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };
        assert_eq!(names(&previews), ["new", "recent"]);
        // A stale photo stays for offline use until its refresh replaces it.
        assert_eq!(names(&photos), ["stale"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
