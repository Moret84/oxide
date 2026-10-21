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
