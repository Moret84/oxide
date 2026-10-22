//! Local call progress only: never log tokens, SDP, participant identities or URLs.
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{OnceLock, mpsc};

/// Queues the line: the file is written by one background thread, never on the caller's (async or state) thread.
pub fn record(message: &str) {
    static WRITER: OnceLock<Option<mpsc::Sender<String>>> = OnceLock::new();
    let Some(writer) = WRITER.get_or_init(spawn_writer) else {
        return;
    };
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = writer.send(format!("{time} {message}\n"));
}

fn spawn_writer() -> Option<mpsc::Sender<String>> {
    let directory = std::path::PathBuf::from(std::env::var_os("HOME")?).join("Library/Logs/Oxide");
    let path = directory.join("calling.log");
    let (sender, lines) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("calling-log".into())
        .spawn(move || {
            let mut file = None;
            for line in lines {
                // Reopen when the log was removed, as the per-line open did.
                if file.is_none() || !path.exists() {
                    let _ = std::fs::create_dir_all(&directory);
                    file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .mode(0o600)
                        .open(&path)
                        .ok();
                }
                if let Some(file) = file.as_mut() {
                    let _ = file.write_all(line.as_bytes());
                }
            }
        })
        .ok()?;
    Some(sender)
}

/// Codec and stream structure only; no ICE credentials, addresses, fingerprints or msids.
pub fn sdp_shape(sdp: &str) -> String {
    sdp.lines()
        .filter_map(|line| {
            if line.starts_with("a=ssrc:") {
                return line.split_whitespace().next().map(str::to_owned);
            }
            // Secrets and addresses: only that they are there.
            for name in [
                "a=fingerprint:",
                "a=ice-ufrag:",
                "a=ice-pwd:",
                "a=crypto:",
                "a=candidate:",
            ] {
                if line.starts_with(name) {
                    return Some(format!("{name}…"));
                }
            }
            [
                "m=",
                "a=mid:",
                "a=label:",
                "a=x-ssrc-range:",
                "a=ssrc-group:",
                "a=rtpmap:",
                "a=fmtp:",
                "a=group:",
                "a=sendrecv",
                "a=sendonly",
                "a=recvonly",
                "a=inactive",
                "a=rtcp-mux",
                "a=rtcp-rsize",
                "a=setup:",
                "a=x-",
                "a=extmap-allow-mixed",
                "b=",
            ]
            .iter()
            .any(|prefix| line.starts_with(prefix))
            .then(|| line.to_owned())
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Shape of a `rosterUpdate` without identities: per entry, whether its key is the person (`details.id`), its state and its endpoint modalities.
pub fn roster_shape(body: &serde_json::Value) -> String {
    let roster = body
        .get("roster")
        .or_else(|| body.get("rosterUpdate"))
        .unwrap_or(body);
    let entries: Vec<String> = roster
        .get("participants")
        .and_then(serde_json::Value::as_object)
        .map(|participants| {
            participants
                .iter()
                .map(|(key, value)| {
                    let id = value.pointer("/details/id").and_then(serde_json::Value::as_str);
                    let key_is_id = match id {
                        Some(id) if id == key => "same",
                        Some(id) if id.eq_ignore_ascii_case(key) => "case",
                        Some(_) => "differs",
                        None => "no-id",
                    };
                    let endpoints: Vec<Vec<&str>> = value
                        .get("endpoints")
                        .and_then(serde_json::Value::as_object)
                        .map(|e| {
                            e.values()
                                .map(|e| {
                                    e.as_object()
                                        .map(|o| o.keys().map(String::as_str).collect())
                                        .unwrap_or_default()
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    format!(
                        "{{key={key_is_id} state={} endpoints={endpoints:?}}}",
                        value.get("state").and_then(serde_json::Value::as_str).unwrap_or("-")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "roster type={} entries={}",
        roster.get("type").and_then(serde_json::Value::as_str).unwrap_or("-"),
        entries.join(" ")
    )
}
