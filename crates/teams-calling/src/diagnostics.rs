//! Local call progress only: never log tokens, SDP, participant identities or URLs.
use teams_auth::diagnostics::Log;

static CALLING: Log = Log::new("calling.log");

/// Queues the line: the file is written by one background thread, never on the caller's (async or state) thread.
pub fn record(message: &str) {
    CALLING.record(message);
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
