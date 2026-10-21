//! Fake SharePoint / OneDrive / Substrate for demo mode: the `teams-files` routes over one in-memory tree; data is fictitious.
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use base64::Engine;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const PERSONAL: &str = "/personal/me";
const LIBRARY: &str = "/personal/me/Documents";
pub(crate) const CHAT_FOLDER: &str =
    "/personal/me/Documents/Fichiers de conversation Microsoft Teams";
/// Library of the demo team site, and the channel folders the CSA mock points to.
pub(crate) const TEAM_SITE: &str = "/sites/demo";
pub(crate) const GENERAL_FOLDER: &str = "/sites/demo/Shared Documents/Général";
pub(crate) const ENGINEERING_FOLDER: &str = "/sites/demo/Shared Documents/Développement";

#[derive(Clone)]
struct Node {
    id: String,
    path: String,
    folder: bool,
    bytes: Vec<u8>,
    modified: String,
    by: String,
}
type Store = Arc<Mutex<Vec<Node>>>;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn guid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn folder(path: &str) -> Node {
    Node {
        id: guid(),
        path: path.into(),
        folder: true,
        bytes: vec![],
        modified: "2026-10-01T09:00:00Z".into(),
        by: "Vous".into(),
    }
}
fn file(path: &str, text: &str, by: &str, modified: &str) -> Node {
    Node {
        id: guid(),
        path: path.into(),
        folder: false,
        bytes: text.as_bytes().to_vec(),
        modified: modified.into(),
        by: by.into(),
    }
}
fn seed() -> Vec<Node> {
    vec![
        folder(LIBRARY),
        folder(CHAT_FOLDER),
        file(
            &format!("{CHAT_FOLDER}/Feuille de route.txt"),
            "Feuille de route du client natif : chats, canaux, appels, fichiers.\n",
            "Vous",
            "2026-10-14T16:20:00Z",
        ),
        folder(&format!("{LIBRARY}/Projets")),
        file(
            &format!("{LIBRARY}/Projets/Spécifications.md"),
            "# Spécifications\n\nFichiers joints, onglet Partagé, navigateur OneDrive.\n",
            "Vous",
            "2026-10-12T10:00:00Z",
        ),
        file(
            &format!("{LIBRARY}/Notes.txt"),
            "Notes de réunion.\n",
            "Vous",
            "2026-10-15T08:30:00Z",
        ),
        folder("/sites/demo/Shared Documents"),
        folder(GENERAL_FOLDER),
        file(
            &format!("{GENERAL_FOLDER}/Plan produit.txt"),
            "Plan produit T4.\n",
            "Claire Martin",
            "2026-10-10T14:00:00Z",
        ),
        folder(ENGINEERING_FOLDER),
        file(
            &format!("{ENGINEERING_FOLDER}/README.md"),
            "# Engineering\n\nGuides et conventions de l’équipe.\n",
            "Bob Durand",
            "2026-10-09T11:00:00Z",
        ),
    ]
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/beta/me/files/availability", post(availability))
        .route("/personal/{*rest}", any(sharepoint))
        .route("/sites/{*rest}", any(sharepoint))
        .route("/_api/{*rest}", any(sharepoint))
        .route("/download/{id}", get(download))
        .route("/allfiles/api/me/{*rest}", get(shared_in_chat))
        .route("/api/beta/me/WorkingSetFiles", get(working_set))
        .with_state(Arc::new(Mutex::new(seed())))
}

/// `http://{Host}/`: the mock learns its address from the request.
pub(crate) fn base(headers: &HeaderMap) -> String {
    format!(
        "http://{}/",
        headers
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("127.0.0.1")
    )
}
fn absolute(base: &str, path: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(path.trim_start_matches('/')))
        .map(|u| u.to_string())
        .unwrap_or_default()
}
fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}
fn share_link(base: &str, id: &str) -> String {
    format!("{base}personal/me/_layouts/15/share/{id}")
}
fn item(base: &str, n: &Node) -> Value {
    let site = if n.path.starts_with(PERSONAL) {
        PERSONAL
    } else {
        TEAM_SITE
    };
    json!({
        "id": format!("01{}", n.id.replace('-', "").to_uppercase()),
        "name": name(&n.path),
        "size": n.bytes.len(),
        "webUrl": absolute(base, &n.path),
        "webDavUrl": absolute(base, &n.path),
        "lastModifiedDateTime": n.modified,
        "lastModifiedBy": {"user": {"displayName": n.by}},
        "parentReference": {"driveId": "b!demo"},
        "sharepointIds": {"listItemUniqueId": n.id, "siteUrl": absolute(base, site), "fileUrl": absolute(base, &n.path)},
        "file": {"mimeType": "text/plain"},
        "@microsoft.graph.downloadUrl": format!("{base}download/{}", n.id),
    })
}
fn spo_error(status: StatusCode, code: &str) -> Response {
    (
        status,
        axum::Json(json!({"odata.error": {"code": code, "message": {"value": code}}})),
    )
        .into_response()
}
/// `@a1='…'` values: quotes removed, doubled quotes undone.
fn alias(url: &url::Url, key: &str) -> Option<String> {
    url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| {
        v.trim_start_matches('\'')
            .trim_end_matches('\'')
            .replace("''", "'")
    })
}
fn between<'a>(path: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = path.find(start)? + start.len();
    let to = path[from..].find(end)? + from;
    Some(&path[from..to])
}
/// A free name in `folder`, Teams-style (`nom 2.ext`).
fn free_name(store: &[Node], folder: &str, wanted: &str) -> String {
    let taken = |n: &str| store.iter().any(|x| x.path == format!("{folder}/{n}"));
    if !taken(wanted) {
        return wanted.into();
    }
    let (stem, ext) = wanted.rsplit_once('.').unwrap_or((wanted, ""));
    (2..)
        .map(|i| {
            if ext.is_empty() {
                format!("{stem} {i}")
            } else {
                format!("{stem} {i}.{ext}")
            }
        })
        .find(|n| !taken(n))
        .unwrap()
}

async fn availability(headers: HeaderMap) -> axum::Json<Value> {
    let base = base(&headers);
    axum::Json(json!({
        "personalSiteUrl": absolute(&base, PERSONAL),
        "personalRootFolderUrl": absolute(&base, LIBRARY),
        "lastModifiedTime": now(),
    }))
}

async fn download(
    State(store): State<Store>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match store
        .lock()
        .unwrap()
        .iter()
        .find(|n| n.id == id && !n.folder)
    {
        Some(n) => n.bytes.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn sharepoint(
    State(store): State<Store>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let base = base(&headers);
    let Ok(url) = url::Url::parse(&format!("http://mock{uri}")) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8_lossy()
        .into_owned();
    let mut store = store.lock().unwrap();
    let by_id = |store: &[Node], id: &str| store.iter().position(|n| n.id == id);

    if path.contains("/_api/web/GetFolderByServerRelativePath(") && method == Method::GET {
        let folder = alias(&url, "@a1").unwrap_or_default();
        let folder = folder.trim_end_matches('/');
        if !store.iter().any(|n| n.folder && n.path == folder) {
            return spo_error(
                StatusCode::NOT_FOUND,
                "-2147024894, System.IO.FileNotFoundException",
            );
        }
        let children = |want_folder: bool| -> Vec<Value> {
            store
                .iter()
                .filter(|n| n.folder == want_folder && parent(&n.path) == folder)
                .map(|n| {
                    json!({
                        "Name": name(&n.path), "ServerRelativeUrl": n.path, "UniqueId": n.id,
                        "Length": n.bytes.len().to_string(), "TimeLastModified": n.modified,
                        "ItemCount": store.iter().filter(|c| parent(&c.path) == n.path).count(),
                        "ModifiedBy": {"Title": n.by}
                    })
                })
                .collect()
        };
        return axum::Json(json!({"Folders": children(true), "Files": children(false)}))
            .into_response();
    }
    if path.contains("/_api/web/folders/AddUsingPath(") {
        let folder = alias(&url, "@a1").unwrap_or_default();
        if !store.iter().any(|n| n.path == folder) {
            store.push(folder_node(&folder));
        }
        return axum::Json(json!({"ServerRelativeUrl": folder})).into_response();
    }
    if path.contains("/_api/v2.0/drive/root:/") && method == Method::PUT {
        let Some(relative) = between(&path, "/drive/root:/", ":/content") else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let (folder, wanted) = relative.rsplit_once('/').unwrap_or(("", relative));
        let folder = if folder.is_empty() {
            LIBRARY.to_owned()
        } else {
            format!("{LIBRARY}/{folder}")
        };
        if !store.iter().any(|n| n.path == folder) {
            store.push(folder_node(&folder));
        }
        let fresh = free_name(&store, &folder, wanted);
        let node = Node {
            id: guid(),
            path: format!("{folder}/{fresh}"),
            folder: false,
            bytes: body.to_vec(),
            modified: now(),
            by: "Vous".into(),
        };
        let value = item(&base, &node);
        store.push(node);
        return (StatusCode::CREATED, axum::Json(value)).into_response();
    }
    if path.contains("/Files/addStub(") {
        let folder = alias(&url, "@v").unwrap_or_default();
        let wanted = alias(&url, "@fn").unwrap_or_default();
        if !store.iter().any(|n| n.folder && n.path == folder) {
            return spo_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "-2147024893, System.IO.DirectoryNotFoundException",
            );
        }
        let target = format!("{folder}/{wanted}");
        if store.iter().any(|n| n.path == target) {
            return spo_error(
                StatusCode::CONFLICT,
                "-2130575257, Microsoft.SharePoint.SPException",
            );
        }
        let node = Node {
            id: guid(),
            path: target.clone(),
            folder: false,
            bytes: vec![],
            modified: now(),
            by: "Vous".into(),
        };
        let id = node.id.clone();
        store.push(node);
        return axum::Json(json!({"UniqueId": id, "ServerRelativeUrl": target})).into_response();
    }
    if let Some(id) = between(&path, "GetFileById('", "')") {
        let Some(i) = by_id(&store, id) else {
            return spo_error(StatusCode::NOT_FOUND, "-2130575338, itemNotFound");
        };
        if path.contains("/StartUpload(") {
            store[i].bytes = body.to_vec();
            return axum::Json(json!({"value": store[i].bytes.len()})).into_response();
        }
        if path.contains("/ContinueUpload(") {
            store[i].bytes.extend_from_slice(&body);
            return axum::Json(json!({"value": store[i].bytes.len()})).into_response();
        }
        if path.contains("/FinishUpload(") {
            store[i].modified = now();
            return axum::Json(json!({"UniqueId": id})).into_response();
        }
        if path.contains("/CancelUpload(") {
            return StatusCode::OK.into_response();
        }
        if path.contains("/validateUpdateListItem") {
            return rename(&mut store, i, &body);
        }
        if method == Method::DELETE {
            store.remove(i);
            return StatusCode::OK.into_response();
        }
    }
    if let Some(id) = between(&path, "GetFolderById('", "')") {
        let Some(i) = by_id(&store, id) else {
            return spo_error(StatusCode::NOT_FOUND, "-2130575338, itemNotFound");
        };
        if path.contains("/validateUpdateListItem") {
            return rename(&mut store, i, &body);
        }
        if path.ends_with("/recycle") {
            let gone = store[i].path.clone();
            store.retain(|n| n.path != gone && !n.path.starts_with(&format!("{gone}/")));
            return StatusCode::OK.into_response();
        }
    }
    if let Some(id) = between(&path, "/_api/v2.0/sites/root/items/", "/driveItem") {
        let Some(i) = by_id(&store, id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if path.ends_with("/invite") {
            return axum::Json(json!({"value": [{
                "id": format!("perm-{id}"),
                "link": {"webUrl": share_link(&base, id), "scope": "users", "type": "view"}
            }]}))
            .into_response();
        }
        return axum::Json(item(&base, &store[i])).into_response();
    }
    if let Some(share) = between(&path, "/_api/v2.0/shares/", "/driveItem") {
        let link = share
            .strip_prefix("u!")
            .and_then(|s| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(s)
                    .ok()
            })
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let found = match link.split_once("/_layouts/15/share/") {
            Some((_, id)) => store.iter().find(|n| n.id == id),
            None => url::Url::parse(&link).ok().and_then(|u| {
                let p = percent_encoding::percent_decode_str(u.path())
                    .decode_utf8_lossy()
                    .into_owned();
                store.iter().find(|n| n.path == p)
            }),
        };
        return match found {
            Some(n) => axum::Json(item(&base, n)).into_response(),
            None => spo_error(StatusCode::NOT_FOUND, "itemNotFound"),
        };
    }
    if path.contains("/_api/web/GetFileByServerRelativeUrl(") && path.contains("/$value") {
        let wanted = alias(&url, "@a1").unwrap_or_default();
        return match store.iter().find(|n| n.path == wanted && !n.folder) {
            Some(n) => n.bytes.clone().into_response(),
            None => spo_error(StatusCode::NOT_FOUND, "-2130575338, itemNotFound"),
        };
    }
    if let Some(query) = between(&path, "/_api/v2.0/drive/root/search(q='", "')") {
        let query = query.replace("''", "'").to_lowercase();
        let site = if path.starts_with(PERSONAL) {
            PERSONAL
        } else {
            TEAM_SITE
        };
        let found: Vec<Value> = store
            .iter()
            .filter(|n| n.path.starts_with(site) && name(&n.path).to_lowercase().contains(&query))
            .filter(|n| n.path != LIBRARY)
            .map(|n| {
                let mut v = item(&base, n);
                if n.folder {
                    v.as_object_mut().unwrap().remove("file");
                    v["folder"] = json!({"childCount": store.iter().filter(|c| parent(&c.path) == n.path).count()});
                }
                v
            })
            .collect();
        return axum::Json(json!({ "value": found })).into_response();
    }
    if path.ends_with("/_api/v2.0/drive/recent") {
        let mut files: Vec<&Node> = store
            .iter()
            .filter(|n| !n.folder && n.path.starts_with(PERSONAL))
            .collect();
        files.sort_by(|a, b| b.modified.cmp(&a.modified));
        return axum::Json(
            json!({"value": files.iter().take(20).map(|n| item(&base, n)).collect::<Vec<_>>()}),
        )
        .into_response();
    }
    if path.ends_with("/_api/v2.0/drive/sharedWithMe") {
        let shared: Vec<Value> = store
            .iter()
            .filter(|n| !n.folder && n.path.starts_with(TEAM_SITE))
            .map(|n| json!({"name": name(&n.path), "remoteItem": item(&base, n)}))
            .collect();
        return axum::Json(json!({"value": shared})).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
fn folder_node(path: &str) -> Node {
    Node {
        modified: now(),
        ..folder(path)
    }
}
fn rename(store: &mut [Node], i: usize, body: &[u8]) -> Response {
    let wanted = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/formValues/0/FieldValue")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let old = store[i].path.clone();
    let new = format!("{}/{wanted}", parent(&old));
    if wanted.is_empty() || store.iter().any(|n| n.path == new) {
        return axum::Json(json!({"value": [{"FieldName": "FileLeafRef", "HasException": true, "ErrorMessage": "Ce nom est déjà utilisé."}]})).into_response();
    }
    for n in store.iter_mut() {
        if n.path == old {
            n.path = new.clone();
        } else if let Some(rest) = n.path.strip_prefix(&format!("{old}/")) {
            n.path = format!("{new}/{rest}");
        }
    }
    axum::Json(json!({"value": [{"FieldName": "FileLeafRef", "HasException": false}]}))
        .into_response()
}

/// AllFiles `SharedInChat[Fast]`: in the demo, everything in the chat folder.
async fn shared_in_chat(State(store): State<Store>, headers: HeaderMap) -> axum::Json<Value> {
    let base = base(&headers);
    let store = store.lock().unwrap();
    let value: Vec<Value> = store
        .iter()
        .filter(|n| !n.folder && parent(&n.path) == CHAT_FOLDER)
        .map(|n| {
            json!({
                "name": name(&n.path),
                "file": {"extension": name(&n.path).rsplit_once('.').map(|(_, e)| e).unwrap_or("")},
                "sharepointIds": {"listItemUniqueId": n.id, "fileUrl": absolute(&base, &n.path), "siteUrl": absolute(&base, PERSONAL)},
                "callerSharingInfo": {"sharedDateTime": n.modified, "webUrl": share_link(&base, &n.id),
                    "sharedBy": {"user": {"displayName": n.by, "mriId": crate::mock::ME}}}
            })
        })
        .collect();
    axum::Json(json!({ "value": value }))
}
async fn working_set() -> axum::Json<Value> {
    axum::Json(json!({"value": [{
        "Id": "demo-link", "FileModifiedTime": "2026-10-13T09:15:00Z",
        "ItemProperties": {"WebLink": {"WebUrl": "https://www.rust-lang.org/learn", "Title": "Apprendre Rust"},
            "Shared": {"LastSharedWithMailboxOwnerByDisplayName": "Alice Martin"}},
        "UserRelationship": {"LastSharedById": "8:orgid:alice"}
    }]}))
}
