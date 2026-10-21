//! `properties.files` of an outgoing message: a JSON string listing the files and their deep links.
use crate::{Shared, UploadedFile};
use serde_json::{Map, Value, json};

/// `fileChicletState.serviceName`: `p2p` for a chat, `teams` for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceName {
    P2p,
    Teams,
}
impl ServiceName {
    fn wire(self) -> &'static str {
        match self {
            Self::P2p => "p2p",
            Self::Teams => "teams",
        }
    }
}

/// One attachment ready to be referenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReference {
    pub file: UploadedFile,
    pub shared: Shared,
    pub service: ServiceName,
}

const OFFICE: [&str; 12] = [
    "doc", "docx", "docm", "dotx", "xls", "xlsx", "xlsm", "xlsb", "ppt", "pptx", "pptm", "ppsx",
];

/// Lower-case extension, the `type` / `fileType` of the reference.
pub fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

impl FileReference {
    pub fn to_value(&self) -> Value {
        let f = &self.file;
        let id = if f.file_id.is_empty() {
            f.item_id.clone().unwrap_or_default()
        } else {
            f.file_id.clone()
        };
        let kind = extension(&f.name);
        let open_url = (self.service == ServiceName::Teams
            && OFFICE.contains(&kind.as_str())
            && !f.file_id.is_empty())
        .then(|| {
            format!(
                "{}/_layouts/15/Doc.aspx?action=edit&sourcedoc={{{}}}",
                f.site_url.trim_end_matches('/'),
                f.file_id
            )
        });
        // Keys left `undefined` by the web client vanish from its JSON: only set them here.
        let mut info = Map::new();
        if let Some(item) = &f.item_id {
            info.insert("itemId".into(), json!(item));
        }
        info.insert("fileUrl".into(), json!(f.object_url));
        info.insert("siteUrl".into(), json!(f.site_url));
        info.insert("serverRelativeUrl".into(), json!(""));
        if let Some(url) = &self.shared.share_url {
            info.insert("shareUrl".into(), json!(url));
        }
        if let Some(sid) = &self.shared.share_id {
            info.insert("shareId".into(), json!(sid));
        }
        let mut value = json!({
            "itemid": id,
            "fileName": f.name,
            "fileType": kind,
            "fileInfo": info,
            "fileChicletState": {"serviceName": self.service.wire(), "state": "active"},
            "@type": "http://schema.skype.com/File",
            "version": 2,
            "id": id,
            "baseUrl": f.site_url,
            "objectUrl": f.object_url,
            "openUrl": open_url,
            "type": kind,
            "title": f.name,
            "state": "active",
            "chicletBreadcrumbs": null,
            "providerData": "",
            "botFileProperties": {},
            "isUploadError": false,
            "progressComplete": null,
            "partialSuccessError": self.shared.partial_error,
        });
        if let Some(scope) = &self.shared.permission_scope {
            value["permissionScope"] = json!(scope);
        }
        if let Some(ids) = &f.sharepoint_ids {
            let mut ids = ids.clone();
            if let (Some(map), Some(drive)) = (ids.as_object_mut(), &f.drive_id) {
                map.insert("driveId".into(), json!(drive));
            }
            value["sharepointIds"] = ids;
        }
        value
    }
}

/// The `properties.files` string for these attachments.
pub fn files_property(files: &[FileReference]) -> String {
    Value::Array(files.iter().map(FileReference::to_value).collect()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> UploadedFile {
        UploadedFile {
            file_id: "0fb7e6b8-1b9f-4c3e-9b39-6a0bc1f7c111".into(),
            item_id: Some("01ABC".into()),
            name: "Rapport.DOCX".into(),
            site_url: "https://c-my.sharepoint.com/personal/a/".into(),
            object_url: "https://c-my.sharepoint.com/personal/a/Documents/Microsoft Teams Chat Files/Rapport.DOCX".into(),
            ..Default::default()
        }
    }

    #[test]
    fn serializes_the_v2_schema_read_back_by_to_files() {
        let reference = FileReference {
            file: file(),
            shared: Shared {
                share_url: Some("https://c-my.sharepoint.com/:w:/g/personal/a/E1".into()),
                share_id: Some("perm".into()),
                permission_scope: Some("users".into()),
                partial_error: None,
            },
            service: ServiceName::P2p,
        };
        let v = reference.to_value();
        assert_eq!(v["@type"], "http://schema.skype.com/File");
        assert_eq!(v["version"], 2);
        assert_eq!(v["id"], v["itemid"]);
        assert_eq!(v["type"], "docx");
        assert_eq!(v["fileChicletState"]["serviceName"], "p2p");
        assert_eq!(v["fileInfo"]["shareId"], "perm");
        assert_eq!(v["fileInfo"]["itemId"], "01ABC");
        assert_eq!(v["openUrl"], Value::Null);
        assert_eq!(v["partialSuccessError"], Value::Null);
        // Round trip through the string form used on the wire.
        let parsed: Value = serde_json::from_str(&files_property(&[reference])).unwrap();
        assert_eq!(parsed[0]["objectUrl"], v["objectUrl"]);
    }

    #[test]
    fn channel_office_files_open_in_the_editor() {
        let v = FileReference {
            file: file(),
            shared: Shared::default(),
            service: ServiceName::Teams,
        }
        .to_value();
        assert_eq!(
            v["openUrl"],
            "https://c-my.sharepoint.com/personal/a/_layouts/15/Doc.aspx?action=edit&sourcedoc={0fb7e6b8-1b9f-4c3e-9b39-6a0bc1f7c111}"
        );
        assert!(v["fileInfo"].get("shareUrl").is_none());
    }
}
