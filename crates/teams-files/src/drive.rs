//! Folder browsing (OneDrive, channel libraries) through REST and Vroom, where the web client embeds `filebrowser.aspx`.
use crate::upload::{guid, valid_name};
use crate::{Auth, Body, Error, FilesClient, alias, api, origin, str_at, u64_at};
use http::Method;
use serde_json::{Value, json};
use teams_auth::Authenticator;
use teams_i18n::t;
use url::Url;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriveEntry {
    /// `UniqueId` (folder listings) or driveItem id (recent, shared with me).
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub size: Option<u64>,
    pub child_count: Option<u64>,
    /// ISO 8601.
    pub modified: String,
    pub modified_by: String,
    /// Where it opens in the browser.
    pub web_url: String,
    /// Set for entries of a folder listing on this site.
    pub server_relative_url: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriveList {
    pub entries: Vec<DriveEntry>,
}

/// Folders SharePoint keeps at the root of a library.
const SYSTEM_FOLDERS: [&str; 1] = ["Forms"];

fn folder_entries(value: &Value, origin: &Url) -> Vec<DriveEntry> {
    let list = |key: &str| -> Vec<Value> {
        let v = value.get(key).or_else(|| value.pointer(&format!("/d/{key}")));
        v.and_then(|v| v.as_array().or_else(|| v.get("results")?.as_array()))
            .cloned()
            .unwrap_or_default()
    };
    let entry = |x: &Value, folder: bool| -> Option<DriveEntry> {
        let name = str_at(x, &["/Name"])?.to_owned();
        let relative = str_at(x, &["/ServerRelativeUrl"])?.to_owned();
        Some(DriveEntry {
            id: str_at(x, &["/UniqueId"]).unwrap_or_default().to_owned(),
            folder,
            size: (!folder).then(|| u64_at(x, &["/Length"])).flatten(),
            child_count: folder.then(|| u64_at(x, &["/ItemCount"])).flatten(),
            modified: str_at(x, &["/TimeLastModified"]).unwrap_or_default().to_owned(),
            modified_by: str_at(x, &["/ModifiedBy/Title"]).unwrap_or_default().to_owned(),
            web_url: origin.join(&relative).map(|u| u.to_string()).unwrap_or_default(),
            server_relative_url: Some(relative),
            name,
        })
    };
    let mut folders: Vec<_> = list("Folders")
        .iter()
        .filter_map(|x| entry(x, true))
        .filter(|e| !SYSTEM_FOLDERS.contains(&e.name.as_str()))
        .collect();
    let mut files: Vec<_> = list("Files").iter().filter_map(|x| entry(x, false)).collect();
    folders.sort_by_key(|e| e.name.to_lowercase());
    files.sort_by_key(|e| e.name.to_lowercase());
    folders.extend(files);
    folders
}
fn drive_items(value: &Value) -> Vec<DriveEntry> {
    value
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|x| {
            // Items of other drives come back wrapped in `remoteItem`.
            let item = x.get("remoteItem").filter(|r| r.is_object()).unwrap_or(x);
            let name = str_at(x, &["/name"]).or_else(|| str_at(item, &["/name"]))?;
            let folder = item.get("folder").is_some() || x.get("folder").is_some();
            Some(DriveEntry {
                id: str_at(item, &["/id"]).unwrap_or_default().to_owned(),
                name: name.to_owned(),
                folder,
                size: (!folder).then(|| u64_at(item, &["/size"])).flatten(),
                child_count: u64_at(item, &["/folder/childCount"]),
                modified: str_at(
                    item,
                    &["/lastModifiedDateTime", "/fileSystemInfo/lastModifiedDateTime"],
                )
                .or_else(|| str_at(x, &["/lastModifiedDateTime"]))
                .unwrap_or_default()
                .to_owned(),
                modified_by: str_at(
                    item,
                    &["/lastModifiedBy/user/displayName", "/shared/sharedBy/user/displayName"],
                )
                .unwrap_or_default()
                .to_owned(),
                web_url: str_at(item, &["/webUrl"])
                    .or_else(|| str_at(x, &["/webUrl"]))
                    .unwrap_or_default()
                    .to_owned(),
                server_relative_url: None,
            })
        })
        .collect()
}

impl<A: Authenticator> FilesClient<A> {
    /// Folders then files of a server-relative folder.
    pub async fn list_folder(&self, site: &Url, folder: &str) -> Result<DriveList, Error> {
        let url = api(
            site,
            &format!(
                "_api/web/GetFolderByServerRelativePath(decodedUrl=@a1)?@a1='{}'&$expand=Folders,Files,Files/ModifiedBy",
                alias(folder)
            ),
        )?;
        let value = self
            .json(Auth::Resource, Method::GET, url, &[], Body::Empty)
            .await?;
        Ok(DriveList {
            entries: folder_entries(&value, &origin(site)?),
        })
    }

    pub async fn recent(&self, site: &Url) -> Result<DriveList, Error> {
        self.drive_items(site, "_api/v2.0/drive/recent").await
    }

    pub async fn shared_with_me(&self, site: &Url) -> Result<DriveList, Error> {
        self.drive_items(site, "_api/v2.0/drive/sharedWithMe").await
    }

    async fn drive_items(&self, site: &Url, rest: &str) -> Result<DriveList, Error> {
        let value = self
            .json(Auth::Resource, Method::GET, api(site, rest)?, &[], Body::Empty)
            .await?;
        Ok(DriveList {
            entries: drive_items(&value),
        })
    }

    /// Results on this site get their server-relative path, so they open and browse like listed ones.
    pub async fn search(&self, site: &Url, query: &str) -> Result<DriveList, Error> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(DriveList::default());
        }
        let url = api(
            site,
            &format!("_api/v2.0/drive/root/search(q='{}')?$top=200", alias(query)),
        )?;
        let value = self
            .json(Auth::Resource, Method::GET, url, &[], Body::Empty)
            .await?;
        let origin = origin(site)?;
        let root = crate::site::decoded(site.path());
        let root = root.trim_end_matches('/');
        let mut entries = drive_items(&value);
        for e in &mut entries {
            e.server_relative_url = Url::parse(&e.web_url)
                .ok()
                .filter(|u| u.origin() == origin.origin())
                .map(|u| crate::site::decoded(u.path()))
                .filter(|p| p.starts_with(root));
        }
        Ok(DriveList { entries })
    }

    /// A new folder inside `parent` (server-relative).
    pub async fn create_folder(&self, site: &Url, parent: &str, name: &str) -> Result<(), Error> {
        valid_name(name)?;
        self.add_folder(site, &format!("{}/{}", parent.trim_end_matches('/'), name.trim()))
            .await
    }

    /// Renames a file or folder of a listing (`FileLeafRef` of its list item).
    pub async fn rename(&self, site: &Url, entry: &DriveEntry, name: &str) -> Result<(), Error> {
        valid_name(name)?;
        let kind = if entry.folder { "GetFolderById" } else { "GetFileById" };
        let url = api(
            site,
            &format!(
                "_api/web/{kind}('{}')/ListItemAllFields/validateUpdateListItem",
                guid(&entry.id)?
            ),
        )?;
        let value = self
            .json(
                Auth::Resource,
                Method::POST,
                url,
                &[],
                Body::Json(json!({
                    "formValues": [{"FieldName": "FileLeafRef", "FieldValue": name.trim()}],
                    "bNewDocumentUpdate": false
                })),
            )
            .await?;
        // The call succeeds overall and reports field errors per value.
        let failed = value
            .get("value")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|v| {
                v.get("HasException")
                    .and_then(Value::as_bool)
                    .filter(|e| *e)
                    .map(|_| str_at(v, &["/ErrorMessage"]).unwrap_or(t("files.drive.rename_refused")).to_owned())
            });
        match failed {
            Some(message) => Err(Error::Invalid(message)),
            None => Ok(()),
        }
    }

    /// Moves a file (`RecycleFile`) or folder to the site's recycle bin.
    pub async fn delete(&self, site: &Url, entry: &DriveEntry) -> Result<(), Error> {
        if entry.folder {
            let url = api(
                site,
                &format!("_api/web/GetFolderById('{}')/recycle", guid(&entry.id)?),
            )?;
            self.send(Auth::Resource, Method::POST, url, &[], Body::Empty)
                .await
                .map(drop)
        } else {
            self.recycle(site, &entry.id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_without_system_folders() {
        let origin = Url::parse("https://c.sharepoint.com/").unwrap();
        let entries = folder_entries(
            &json!({
                "Folders": [
                    {"Name": "Forms", "ServerRelativeUrl": "/sites/t/Shared Documents/Forms", "UniqueId": "f0"},
                    {"Name": "Specs", "ServerRelativeUrl": "/sites/t/Shared Documents/General/Specs", "UniqueId": "f1", "ItemCount": 3, "TimeLastModified": "2026-10-01T00:00:00Z"}
                ],
                "Files": [
                    {"Name": "b.txt", "ServerRelativeUrl": "/sites/t/Shared Documents/General/b.txt", "Length": "12", "UniqueId": "x2", "ModifiedBy": {"Title": "Alice"}},
                    {"Name": "A.docx", "ServerRelativeUrl": "/sites/t/Shared Documents/General/A.docx", "Length": 3, "UniqueId": "x1"}
                ]
            }),
            &origin,
        );
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Specs", "A.docx", "b.txt"]);
        assert_eq!(entries[0].child_count, Some(3));
        assert_eq!(entries[2].size, Some(12));
        assert_eq!(entries[2].modified_by, "Alice");
        assert_eq!(
            entries[1].web_url,
            "https://c.sharepoint.com/sites/t/Shared%20Documents/General/A.docx"
        );
    }

    #[test]
    fn reads_remote_items_of_recent_and_shared() {
        let entries = drive_items(&json!({"value": [
            {"id": "local", "name": "notes.docx", "remoteItem": {"id": "r1", "size": 10, "webUrl": "https://c.sharepoint.com/n.docx",
              "lastModifiedBy": {"user": {"displayName": "Bob"}}, "lastModifiedDateTime": "2026-10-03T00:00:00Z", "file": {}}},
            {"id": "d1", "name": "Projet", "folder": {"childCount": 2}, "webUrl": "https://c-my.sharepoint.com/p"}
        ]}));
        assert_eq!(entries[0].id, "r1");
        assert_eq!(entries[0].modified_by, "Bob");
        assert!(entries[1].folder);
        assert_eq!(entries[1].child_count, Some(2));
    }
}
