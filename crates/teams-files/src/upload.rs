//! `FileUploadService`: one PUT for chat files up to 10 MB, chunked classic REST upload otherwise.
use crate::{Auth, Body, Error, FilesClient, SpoError, alias, api, origin, path, str_at, u64_at};
use http::Method;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use teams_auth::Authenticator;
use teams_i18n::{t, tf};
use tokio::io::AsyncReadExt;
use url::Url;

/// Chunk size, and the quick-upload threshold (`switchToVroomV2UploadSizeThresholdInBytes`).
pub const CHUNK_SIZE: u64 = 10 * 1024 * 1024;
/// Longest `folderRelativeUrl + "/" + name` SharePoint accepts here.
const MAX_PATH: usize = 260;
/// Renames tried on a name conflict (`nom N.ext`).
const MAX_RENAMES: u32 = 50;

/// Where an upload goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// Site the `_api` calls are built on.
    pub site: Url,
    /// Server-relative folder (`/personal/x/Documents/Microsoft Teams Chat Files`).
    pub folder: String,
    /// Folder relative to the document library, for the quick upload (chats only).
    pub library_relative: Option<String>,
}

/// The uploaded item, as needed to share it and reference it in a message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadedFile {
    /// `sharepointIds.listItemUniqueId` / `UniqueId`: the GUID of the list item.
    pub file_id: String,
    /// driveItem id (quick upload only).
    pub item_id: Option<String>,
    pub name: String,
    /// Site the file ended on, with a trailing `/` (it can move after a multi-geo redirect).
    pub site_url: String,
    /// Absolute URL of the file (`webDavUrl`, or origin + `ServerRelativeUrl`).
    pub object_url: String,
    pub server_relative_url: String,
    pub drive_id: Option<String>,
    pub sharepoint_ids: Option<Value>,
    pub size: u64,
}

pub(crate) fn guid(id: &str) -> Result<&str, Error> {
    let ok = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    ok.then_some(id)
        .ok_or_else(|| Error::Invalid(t("files.upload.invalid_file_id").into()))
}
/// `rapport.docx` → `rapport 2.docx`.
fn renamed(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} {n}.{ext}"),
        _ => format!("{name} {n}"),
    }
}
/// Characters SharePoint refuses in a file name.
pub(crate) fn valid_name(name: &str) -> Result<(), Error> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|c| "\"*:<>?/\\|".contains(c) || c.is_control())
    {
        return Err(Error::Invalid(tf!("files.upload.unsupported_file_name", name = name)));
    }
    Ok(())
}
const UPLOAD: [(&str, &str); 3] = [
    ("Scenario", "ShareUploadFile"),
    ("ScenarioType", "AUO"),
    ("prefer", "manualRedirect"),
];
const ABORT: [(&str, &str); 2] = [("Scenario", "ShareAbort"), ("ScenarioType", "PO")];

async fn read_chunk(file: &mut tokio::fs::File, max: u64) -> Result<Vec<u8>, Error> {
    let mut buffer = Vec::with_capacity(max as usize);
    file.take(max)
        .read_to_end(&mut buffer)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(buffer)
}
fn new_site(redirect: &str) -> Option<Url> {
    let (base, _) = redirect.split_once("_api")?;
    crate::site_root(base).ok()
}

impl<A: Authenticator> FilesClient<A> {
    /// `progress(done, total)` counts calls; `cancel` is checked between chunks and removes the partial file.
    pub async fn upload(
        &self,
        destination: &Destination,
        file: &Path,
        name: &str,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
        cancel: &AtomicBool,
    ) -> Result<UploadedFile, Error> {
        valid_name(name)?;
        let size = tokio::fs::metadata(file)
            .await
            .map_err(|e| Error::Io(e.to_string()))?
            .len();
        let mut destination = destination.clone();
        for attempt in 0..2 {
            let result = match &destination.library_relative {
                Some(relative) if size <= CHUNK_SIZE => {
                    self.quick_upload(&destination.site, relative, file, name, size, progress)
                        .await
                }
                _ => {
                    self.chunked_upload(&destination, file, name, size, progress, cancel)
                        .await
                }
            };
            match result {
                // Multi-geo: replay once on the site the service points to.
                Err(Error::Http {
                    status: 404,
                    redirect: Some(redirect),
                    ..
                }) if attempt == 0 => {
                    let Some(site) = new_site(&redirect) else {
                        return Err(Error::Decode);
                    };
                    destination.site = site;
                }
                other => return other,
            }
        }
        Err(Error::Decode)
    }

    async fn quick_upload(
        &self,
        site: &Url,
        relative: &str,
        file: &Path,
        name: &str,
        size: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<UploadedFile, Error> {
        let bytes = tokio::fs::read(file)
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        let folder = path(relative);
        let url = api(
            site,
            &format!(
                "_api/v2.0/drive/root:/{}{}:/content?@name.conflictBehavior=rename&$select=*,sharepointIds,webDavUrl",
                if folder.is_empty() { String::new() } else { format!("{folder}/") },
                crate::component(name)
            ),
        )?;
        progress(0, size);
        let item = self
            .json(Auth::Resource, Method::PUT, url, &UPLOAD, Body::Bytes(bytes))
            .await?;
        progress(size, size);
        let object_url = str_at(&item, &["/webDavUrl", "/webUrl"])
            .ok_or(Error::Decode)?
            .to_owned();
        Ok(UploadedFile {
            file_id: str_at(&item, &["/sharepointIds/listItemUniqueId"])
                .unwrap_or_default()
                .to_owned(),
            item_id: str_at(&item, &["/id"]).map(str::to_owned),
            name: str_at(&item, &["/name"]).unwrap_or(name).to_owned(),
            site_url: str_at(&item, &["/sharepointIds/siteUrl"])
                .and_then(|s| crate::site_root(s).ok())
                .unwrap_or_else(|| site.clone())
                .to_string(),
            server_relative_url: Url::parse(&object_url)
                .map(|u| crate::site::decoded(u.path()))
                .unwrap_or_default(),
            object_url,
            drive_id: str_at(&item, &["/parentReference/driveId"]).map(str::to_owned),
            sharepoint_ids: item.get("sharepointIds").cloned(),
            size: u64_at(&item, &["/size"]).unwrap_or(size),
        })
    }

    /// `addStub`, creating the folder once when it is missing and renaming on conflicts.
    async fn add_stub(&self, site: &Url, folder: &str, name: &str) -> Result<(Value, String), Error> {
        let mut created = false;
        let mut n = 1;
        let mut current = name.to_owned();
        loop {
            if folder.chars().count() + 1 + current.chars().count() > MAX_PATH {
                return Err(Error::Invalid(t("files.upload.file_name_is_too_long").into()));
            }
            let url = api(
                site,
                &format!(
                    "_api/web/GetFolderByServerRelativeUrl(@v)/Files/addStub(@fn)?@v='{}'&@fn='{}'",
                    alias(folder),
                    alias(&current)
                ),
            )?;
            match self
                .json(Auth::Resource, Method::POST, url, &UPLOAD, Body::Empty)
                .await
            {
                Ok(stub) => return Ok((stub, current)),
                Err(e) if e.spo() == Some(&SpoError::FolderMissing) && !created => {
                    created = true;
                    self.add_folder(site, folder).await?;
                }
                Err(e) if e.spo() == Some(&SpoError::AlreadyPresent) && n < MAX_RENAMES => {
                    n += 1;
                    current = renamed(name, n);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// `AddUsingPath(DecodedUrl=@a1,overwrite=@a2)`.
    pub async fn add_folder(&self, site: &Url, folder: &str) -> Result<(), Error> {
        let url = api(
            site,
            &format!(
                "_api/web/folders/AddUsingPath(DecodedUrl=@a1,overwrite=@a2)?@a1='{}'&@a2=false",
                alias(folder)
            ),
        )?;
        self.send(Auth::Resource, Method::POST, url, &UPLOAD, Body::Empty)
            .await
            .map(drop)
    }

    async fn chunked_upload(
        &self,
        destination: &Destination,
        file: &Path,
        name: &str,
        size: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
        cancel: &AtomicBool,
    ) -> Result<UploadedFile, Error> {
        let site = &destination.site;
        let folder = destination.folder.trim_end_matches('/');
        let total = size.div_ceil(CHUNK_SIZE).max(1) + 2;
        progress(0, total);
        let (stub, name) = self.add_stub(site, folder, name).await?;
        progress(1, total);
        let file_id = str_at(&stub, &["/UniqueId", "/d/UniqueId"])
            .ok_or(Error::Decode)?
            .to_owned();
        guid(&file_id)?;
        let relative = str_at(&stub, &["/ServerRelativeUrl", "/d/ServerRelativeUrl"])
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{folder}/{name}"));
        let upload_id = uuid::Uuid::new_v4().to_string();
        let result = self
            .send_chunks(site, folder, &name, &file_id, &upload_id, file, size, progress, total, cancel)
            .await;
        if let Err(e) = result {
            // Leave no stub behind: cancel the session, then delete the file.
            let _ = self.cancel_upload(site, &file_id, &upload_id).await;
            let _ = self.delete_file(site, &file_id).await;
            return Err(e);
        }
        let origin = origin(site)?;
        Ok(UploadedFile {
            file_id,
            item_id: None,
            name,
            site_url: crate::site_root(site.as_str())?.to_string(),
            object_url: origin
                .join(&relative)
                .map_err(|_| Error::Decode)?
                .to_string(),
            server_relative_url: relative,
            drive_id: None,
            sharepoint_ids: None,
            size,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_chunks(
        &self,
        site: &Url,
        folder: &str,
        name: &str,
        file_id: &str,
        upload_id: &str,
        file: &Path,
        size: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
        total: u64,
        cancel: &AtomicBool,
    ) -> Result<(), Error> {
        let query = format!("?@folder='{}'&@file='{}'", alias(folder), alias(name));
        let base = format!("_api/web/GetFileById('{file_id}')");
        let mut reader = tokio::fs::File::open(file)
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        let mut offset = 0u64;
        let mut calls = 1;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let chunk = read_chunk(&mut reader, CHUNK_SIZE).await?;
            let length = chunk.len() as u64;
            let call = if offset == 0 {
                format!("{base}/StartUpload(uploadId=guid'{upload_id}'){query}")
            } else {
                format!("{base}/ContinueUpload(uploadId=guid'{upload_id}',fileOffset={offset}){query}")
            };
            let value = self
                .json(
                    Auth::Resource,
                    Method::POST,
                    api(site, &call)?,
                    &UPLOAD,
                    Body::Bytes(chunk),
                )
                .await?;
            let next = u64_at(
                &value,
                &["/value", "/d/StartUpload", "/d/ContinueUpload"],
            )
            .unwrap_or(offset + length);
            calls += 1;
            progress(calls, total);
            if next != offset + length {
                return Err(Error::Decode);
            }
            offset = next;
            if offset >= size {
                break;
            }
        }
        let finish = format!("{base}/FinishUpload(uploadId=guid'{upload_id}',fileOffset={size}){query}");
        self.send(Auth::Resource, Method::POST, api(site, &finish)?, &UPLOAD, Body::Empty)
            .await?;
        progress(total, total);
        Ok(())
    }

    async fn cancel_upload(&self, site: &Url, file_id: &str, upload_id: &str) -> Result<(), Error> {
        let url = api(
            site,
            &format!("_api/web/GetFileById('{}')/CancelUpload(uploadId=guid'{upload_id}')", guid(file_id)?),
        )?;
        self.send(Auth::Resource, Method::POST, url, &ABORT, Body::Empty)
            .await
            .map(drop)
    }
    /// `GetFileById_DeleteFile`: removes a stub during an aborted upload.
    async fn delete_file(&self, site: &Url, file_id: &str) -> Result<(), Error> {
        let url = api(site, &format!("_api/web/GetFileById('{}')", guid(file_id)?))?;
        self.send(Auth::Resource, Method::DELETE, url, &ABORT, Body::Empty)
            .await
            .map(drop)
    }
    /// `RecycleFile`: an attachment removed from the draft goes to the recycle bin.
    pub async fn recycle(&self, site: &Url, file_id: &str) -> Result<(), Error> {
        let url = api(site, &format!("_api/web/GetFileById('{}')/recycle", guid(file_id)?))?;
        self.send(Auth::Resource, Method::DELETE, url, &ABORT, Body::Empty)
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_like_teams_and_validates_names() {
        assert_eq!(renamed("rapport.docx", 2), "rapport 2.docx");
        assert_eq!(renamed("README", 3), "README 3");
        assert_eq!(renamed(".env", 2), ".env 2");
        assert!(valid_name("a:b.txt").is_err());
        assert!(valid_name("ok (1).pdf").is_ok());
        assert!(guid("0fb7e6b8-1b9f-4c3e-9b39-6a0bc1f7c111").is_ok());
        assert!(guid("x') or 1 eq 1 or ('").is_err());
    }

    #[test]
    fn redirect_keeps_the_part_before_api() {
        assert_eq!(
            new_site("https://eu-my.sharepoint.com/personal/a/_api/v2.0/drive").unwrap().as_str(),
            "https://eu-my.sharepoint.com/personal/a/"
        );
    }
}
