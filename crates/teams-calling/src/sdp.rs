//! WebRTC SDP ⇄ NGC SDP (`toMsSdp` / `fromMsSdp` chains); web client features off by default (simulcast, RED…) aren't ported.

/// Whether an SDP is an offer or an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Offer,
    Answer,
}

const LABEL_AUDIO: &str = "main-audio";
const LABEL_VIDEO: &str = "main-video";
const LABEL_SHARING: &str = "applicationsharing-video";

/// `/` ↔ `\` encoded header extensions (`MSSDP_ENCODED_URI`).
const ENCODED_EXTENSIONS: [&str; 2] = [
    "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time",
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
];
/// Extensions removed before sending (`v6`).
const UNSENT_EXTENSIONS: [&str; 2] = [
    "urn:ietf:params:rtp-hdrext:csrc-audio-level",
    "http://www.webrtc.org/experiments/rtp-hdrext/playout-delay",
];
/// Extensions kept from the service (`o3`).
const SUPPORTED_EXTENSIONS: [&str; 8] = [
    "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time",
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
    "http://www.webrtc.org/experiments/rtp-hdrext/video-layers-allocation00",
    "http://www.webrtc.org/experiments/rtp-hdrext/video-layers-allocation00-non-advertised",
    "urn:ietf:params:rtp-hdrext:sdes:mid",
    "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id",
    "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id",
    "https://aomediacodec.github.io/av1-rtp-spec/#dependency-descriptor-rtp-header-extension",
];
/// Video feedback collapsed to `a=rtcp-fb:* …` (`reduceRtcpFbInSdp`).
const COLLAPSED_FEEDBACK: [&str; 5] = ["goog-remb", "transport-cc", "ccm fir", "nack", "nack pli"];
const ALLOWED_VIDEO_CODECS: [&str; 3] = ["H264", "AV1", "rtx"];
/// `h264SendProfileOverride`.
const H264_PROFILE: &str = "42C02A";
/// `maxBandwidthInKbps` − `audioBandwidthInKbps`.
const VIDEO_BANDWIDTH_KBPS: u32 = 7000 - 90;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    kind: String,
    port: String,
    protocol: String,
    payloads: Vec<String>,
    /// Every line after `m=`, without line ending.
    lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sdp {
    session: Vec<String>,
    media: Vec<Section>,
}

impl Section {
    fn parse(m: &str) -> Option<Self> {
        let mut parts = m.strip_prefix("m=")?.split_whitespace();
        Some(Self {
            kind: parts.next()?.to_owned(),
            port: parts.next()?.to_owned(),
            protocol: parts.next()?.to_owned(),
            payloads: parts.map(str::to_owned).collect(),
            lines: Vec::new(),
        })
    }

    fn m_line(&self) -> String {
        let mut line = format!("m={} {} {}", self.kind, self.port, self.protocol);
        for payload in &self.payloads {
            line.push(' ');
            line.push_str(payload);
        }
        line
    }

    fn rejected(&self) -> bool {
        self.port == "0"
    }

    /// Values of `a=<name>:…` (or bare `a=<name>`, as an empty string).
    fn attrs<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.lines.iter().filter_map(move |line| attr(line, name))
    }

    fn attr<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.attrs(name).next()
    }

    /// Keeps only the lines for which `keep` is true.
    fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.lines.retain(|line| keep(line));
    }

    fn remove_attr(&mut self, name: &str) {
        self.retain(|line| attr(line, name).is_none());
    }

    /// Inserts after `c=`/`b=` lines, where the web client's custom attributes go.
    fn insert_front(&mut self, line: String) {
        let at = self
            .lines
            .iter()
            .position(|l| !(l.starts_with("c=") || l.starts_with("b=") || l.starts_with("i=")))
            .unwrap_or(self.lines.len());
        self.lines.insert(at, line);
    }

    fn direction(&self) -> Option<&'static str> {
        ["sendrecv", "sendonly", "recvonly", "inactive"]
            .into_iter()
            .find(|d| self.lines.iter().any(|l| l == &format!("a={d}")))
    }

    fn sends(&self) -> bool {
        matches!(self.direction(), Some("sendrecv" | "sendonly") | None)
    }

    /// `rtpmap` codec name of each payload type.
    fn codec(&self, payload: &str) -> Option<&str> {
        self.rtpmap(payload)
            .map(|rest| rest.split('/').next().unwrap_or(rest))
    }

    /// Payload types whose codec is `name`.
    fn payloads_of(&self, name: &str) -> Vec<String> {
        self.payloads
            .iter()
            .filter(|p| self.codec(p).is_some_and(|c| c.eq_ignore_ascii_case(name)))
            .cloned()
            .collect()
    }

    fn rtpmap(&self, payload: &str) -> Option<&str> {
        self.attrs("rtpmap").find_map(|v| {
            let (pt, rest) = v.split_once(' ')?;
            (pt == payload).then_some(rest)
        })
    }

    /// Removes payload types and every line that refers to them.
    fn drop_payloads(&mut self, dropped: &[String]) {
        if dropped.is_empty() {
            return;
        }
        self.payloads.retain(|p| !dropped.contains(p));
        self.retain(|line| {
            !["rtpmap", "fmtp", "rtcp-fb"].iter().any(|name| {
                attr(line, name).is_some_and(|v| {
                    dropped
                        .iter()
                        .any(|p| v.split(' ').next() == Some(p.as_str()))
                })
            })
        });
    }

    fn drop_codec(&mut self, name: &str) {
        let dropped: Vec<String> = self
            .payloads
            .iter()
            .filter(|p| self.rtpmap(p).is_some_and(|r| r.eq_ignore_ascii_case(name)))
            .cloned()
            .collect();
        self.drop_payloads(&dropped);
    }
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix("a=")?.strip_prefix(name)?;
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix(':')
    }
}

impl Sdp {
    fn parse(sdp: &str) -> Self {
        let mut session = Vec::new();
        let mut media: Vec<Section> = Vec::new();
        for line in sdp.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
            if line.starts_with("m=")
                && let Some(section) = Section::parse(line)
            {
                media.push(section);
            } else if let Some(section) = media.last_mut() {
                section.lines.push(line.to_owned());
            } else {
                session.push(line.to_owned());
            }
        }
        Self { session, media }
    }

    fn write(&self) -> String {
        let mut lines = self.session.clone();
        for section in &self.media {
            lines.push(section.m_line());
            lines.extend(section.lines.iter().cloned());
        }
        let mut out = lines.join("\r\n");
        out.push_str("\r\n");
        out
    }

    fn set_session_attr(&mut self, name: &str, line: String) {
        self.session.retain(|l| attr(l, name).is_none());
        self.session.push(line);
    }

    /// `a=group:BUNDLE` with the mids of every live line.
    fn rebuild_bundle(&mut self) {
        let mids: Vec<&str> = self
            .media
            .iter()
            .filter(|m| !m.rejected())
            .filter_map(|m| m.attr("mid"))
            .collect();
        let line = format!("a=group:BUNDLE {}", mids.join(" "));
        self.session.retain(|l| !l.starts_with("a=group:BUNDLE"));
        if !mids.is_empty() {
            // Groups go right after `t=`, before other session attributes.
            let at = self
                .session
                .iter()
                .position(|l| l.starts_with("t="))
                .map(|i| i + 1)
                .unwrap_or(self.session.len());
            self.session.insert(at, line);
        }
    }

    /// Label of each line: audio; video; the last video is screen sharing when there are two.
    fn labels(&self) -> Vec<&'static str> {
        let videos = self.media.iter().filter(|m| m.kind == "video").count();
        let mut seen_video = 0;
        self.media
            .iter()
            .map(|m| match m.kind.as_str() {
                "audio" => LABEL_AUDIO,
                "video" => {
                    seen_video += 1;
                    if videos > 1 && seen_video == videos {
                        LABEL_SHARING
                    } else {
                        LABEL_VIDEO
                    }
                }
                _ => "data",
            })
            .collect()
    }
}

/// `candidate:<foundation> <component> <transport> <priority> <ip> <port> typ <type> …`
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    foundation: String,
    component: String,
    transport: String,
    priority: String,
    ip: String,
    port: String,
    kind: String,
    related: Option<(String, String)>,
    tcp_type: Option<String>,
}

impl Candidate {
    fn parse(value: &str) -> Option<Self> {
        let tokens: Vec<&str> = value.split_whitespace().collect();
        if tokens.len() < 8 || tokens[6] != "typ" {
            return None;
        }
        let mut candidate = Self {
            foundation: tokens[0].into(),
            component: tokens[1].into(),
            transport: tokens[2].into(),
            priority: tokens[3].into(),
            ip: tokens[4].into(),
            port: tokens[5].into(),
            kind: tokens[7].into(),
            related: None,
            tcp_type: None,
        };
        let mut rest = tokens[8..].iter();
        let (mut raddr, mut rport) = (None, None);
        while let (Some(key), Some(value)) = (rest.next(), rest.next()) {
            match *key {
                "raddr" => raddr = Some(value.to_string()),
                "rport" => rport = Some(value.to_string()),
                "tcptype" => candidate.tcp_type = Some(value.to_string()),
                // generation, ufrag, network-id, network-cost: dropped.
                _ => {}
            }
        }
        candidate.related = raddr.zip(rport);
        Some(candidate)
    }

    fn write(&self) -> String {
        let mut out = format!(
            "{} {} {} {} {} {} typ {}",
            self.foundation,
            self.component,
            self.transport,
            self.priority,
            self.ip,
            self.port,
            self.kind
        );
        if let Some((raddr, rport)) = &self.related {
            out.push_str(&format!(" raddr {raddr} rport {rport}"));
        }
        if let Some(tcp) = &self.tcp_type {
            out.push_str(&format!(" tcptype {tcp}"));
        }
        out
    }

    /// `tcp` + `tcptype active` → `tcp-act` (and back).
    fn into_ms(mut self) -> Self {
        if self.transport.eq_ignore_ascii_case("tcp")
            && let Some(kind) = self.tcp_type.take()
        {
            self.transport = match kind.as_str() {
                "active" => "tcp-act",
                "passive" => "tcp-pass",
                _ => "tcp-so",
            }
            .into();
        }
        self
    }

    fn into_webrtc(mut self) -> Self {
        let tcp = match self.transport.to_ascii_lowercase().as_str() {
            "tcp-act" => Some("active"),
            "tcp-pass" => Some("passive"),
            "tcp-so" => Some("so"),
            _ => None,
        };
        if let Some(tcp) = tcp {
            self.transport = "tcp".into();
            self.tcp_type = Some(tcp.into());
        }
        self
    }
}

fn encode_extension(line: &str) -> String {
    match attr(line, "extmap") {
        Some(value) if ENCODED_EXTENSIONS.iter().any(|uri| value.contains(uri)) => {
            line.replace('/', "\\")
        }
        _ => line.to_owned(),
    }
}

fn extmap_uri(value: &str) -> Option<&str> {
    value.split_whitespace().nth(1)
}

/// H264 receive limits announced per line (`L6`): `(max-fs, max-mbps, max-fps)`.
fn h264_limits(label: &str) -> (u32, u32, u32) {
    if label == LABEL_SHARING {
        (8160, 135000, 1500)
    } else {
        (3600, 108000, 3000)
    }
}

fn set_fmtp_params(value: &str, edit: impl Fn(&mut Vec<(String, String)>)) -> String {
    let (pt, params) = value.split_once(' ').unwrap_or((value, ""));
    let mut pairs: Vec<(String, String)> = params
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.trim().to_owned(), v.trim().to_owned())
        })
        .collect();
    edit(&mut pairs);
    let params: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("a=fmtp:{pt} {}", params.join(";"))
}

/// Local WebRTC offer or answer → NGC blob.
pub fn to_ngc(webrtc: &str, kind: Kind) -> String {
    to_ngc_in(webrtc, kind, false)
}

/// Conference receiving lines carry a placeholder source (`h6`), without which the mixer rejects the answer.
pub fn to_ngc_in(webrtc: &str, kind: Kind, conference: bool) -> String {
    let mut sdp = Sdp::parse(webrtc);
    let cname = sdp
        .media
        .iter()
        .flat_map(|m| m.attrs("ssrc"))
        .find_map(|v| v.split_once(" cname:").map(|(_, c)| c.to_owned()))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let labels = sdp.labels();

    // Session: `msid-semantic WMS *`; fingerprints move to each live line.
    let fingerprint = sdp
        .session
        .iter()
        .find(|l| attr(l, "fingerprint").is_some())
        .cloned();
    sdp.session.retain(|l| attr(l, "fingerprint").is_none());
    sdp.set_session_attr("msid-semantic", "a=msid-semantic: WMS *".into());

    // First live line's transport, for lines bundled without candidates.
    let bundle_transport = sdp.media.iter().find(|m| !m.rejected()).map(|m| {
        (
            m.port.clone(),
            m.lines.iter().find(|l| l.starts_with("c=")).cloned(),
        )
    });

    for (section, label) in sdp.media.iter_mut().zip(labels) {
        section.protocol = "RTP/SAVP".into();
        if section.rejected() {
            let mid = section.attr("mid").map(str::to_owned);
            section.payloads = vec!["34".into()];
            section.lines.clear();
            if let Some(mid) = mid {
                section.lines.push(format!("a=mid:{mid}"));
            }
            section.lines.push(format!("a=label:{label}"));
            continue;
        }
        if let Some(fingerprint) = &fingerprint
            && section.attr("fingerprint").is_none()
        {
            section.lines.push(fingerprint.clone());
        }
        section.remove_attr("label");

        // Codecs.
        section.drop_codec("CN/16000");
        section.drop_codec("L16/48000/2");
        if section.kind == "audio" {
            section.remove_attr("rtcp-rsize");
        }
        if section.kind == "video" {
            let dropped: Vec<String> = section
                .payloads
                .iter()
                .filter(|p| {
                    !section.codec(p).is_some_and(|c| {
                        ALLOWED_VIDEO_CODECS
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(c))
                    })
                })
                .cloned()
                .collect();
            section.drop_payloads(&dropped);
            // rtx whose apt target is gone goes too.
            let orphan_rtx: Vec<String> = section
                .payloads_of("rtx")
                .into_iter()
                .filter(|p| {
                    let apt = section.attrs("fmtp").find_map(|v| {
                        let (pt, params) = v.split_once(' ')?;
                        (pt == p.as_str()).then(|| {
                            params
                                .split(';')
                                .find_map(|kv| kv.trim().strip_prefix("apt="))
                        })?
                    });
                    !apt.is_some_and(|a| section.payloads.iter().any(|p| p == a))
                })
                .collect();
            section.drop_payloads(&orphan_rtx);
            // H264 first, then AV1, then rtx.
            let rank = |p: &String| match section.codec(p).map(str::to_ascii_uppercase).as_deref() {
                Some("H264") => 0,
                Some("AV1") => 1,
                _ => 2,
            };
            let mut payloads = section.payloads.clone();
            payloads.sort_by_key(rank);
            section.payloads = payloads;

            // H264 fmtp: no sps-pps-idr-in-keyframe; profile and receive limits.
            let h264 = section.payloads_of("H264");
            let (fs, mbps, fps) = h264_limits(label);
            for line in section.lines.iter_mut() {
                let Some(value) = attr(line, "fmtp") else {
                    continue;
                };
                if !h264.iter().any(|p| value.split(' ').next() == Some(p)) {
                    continue;
                }
                *line = set_fmtp_params(value, |pairs| {
                    pairs.retain(|(k, _)| k != "sps-pps-idr-in-keyframe");
                    for (key, default) in [
                        ("profile-level-id", H264_PROFILE.to_owned()),
                        ("max-fs", fs.to_string()),
                        ("max-mbps", mbps.to_string()),
                        ("max-fps", fps.to_string()),
                    ] {
                        if !pairs.iter().any(|(k, _)| k == key) {
                            pairs.push((key.to_owned(), default));
                        }
                    }
                });
            }

            // rtcp-fb collapsed to `*`.
            let present: Vec<&str> = COLLAPSED_FEEDBACK
                .into_iter()
                .filter(|fb| {
                    section
                        .attrs("rtcp-fb")
                        .any(|v| v.split_once(' ').is_some_and(|(_, f)| f == *fb))
                })
                .collect();
            section.retain(|line| {
                !attr(line, "rtcp-fb").is_some_and(|v| {
                    v.split_once(' ')
                        .is_some_and(|(_, f)| COLLAPSED_FEEDBACK.contains(&f))
                })
            });
            for fb in present {
                section.lines.push(format!("a=rtcp-fb:* {fb}"));
            }
        }

        // Header extensions.
        section.retain(|line| {
            !attr(line, "extmap")
                .and_then(extmap_uri)
                .is_some_and(|uri| UNSENT_EXTENSIONS.contains(&uri))
        });
        for line in section.lines.iter_mut() {
            *line = encode_extension(line);
        }

        // Candidates: one per (foundation, component), MS TCP names, bundled transport.
        let mut seen = std::collections::HashSet::new();
        let mut candidates = Vec::new();
        for value in section.attrs("candidate") {
            if let Some(c) = Candidate::parse(value)
                && !c.ip.ends_with(".local")
                && seen.insert((c.foundation.clone(), c.component.clone()))
            {
                candidates.push(format!("a=candidate:{}", c.into_ms().write()));
            }
        }
        let at = section
            .lines
            .iter()
            .position(|l| l.starts_with("a=candidate:"));
        section.retain(|line| !line.starts_with("a=candidate:"));
        if let Some(at) = at {
            for (i, line) in candidates.iter().enumerate() {
                section
                    .lines
                    .insert((at + i).min(section.lines.len()), line.clone());
            }
        } else if let Some((port, c)) = &bundle_transport {
            section.port = port.clone();
            if let Some(c) = c {
                section.lines.retain(|l| !l.starts_with("c="));
                section.lines.insert(0, c.clone());
            }
        }

        // a=rtcp: the m-line port on offers, absent on answers.
        section.remove_attr("rtcp");
        if kind == Kind::Offer {
            section.insert_front(format!("a=rtcp:{}", section.port));
        }

        if conference && matches!(section.direction(), Some("recvonly" | "inactive")) {
            let placeholder = if section.kind == "audio" {
                "4195875351"
            } else {
                "1"
            };
            let ssrc = section
                .attr("ssrc")
                .and_then(|v| v.split_whitespace().next())
                .unwrap_or(placeholder)
                .to_owned();
            section.remove_attr("ssrc");
            section.lines.push(format!("a=ssrc:{ssrc} cname:{cname}"));
        }

        // SSRC range from the first source; ssrc/msid lines stay.
        if let Some(ssrc) = section
            .attr("ssrc")
            .and_then(|v| v.split_whitespace().next())
            .map(str::to_owned)
        {
            section.insert_front(format!("a=x-ssrc-range:{ssrc}-{ssrc}"));
        }
        let feedback = match section.kind.as_str() {
            "audio" => Some("a=x-signaling-fb:* x-message app recv:dsh"),
            "video" => Some("a=x-signaling-fb:* x-message app send:src recv:src,vc"),
            _ => None,
        };
        if let Some(feedback) = feedback {
            section.insert_front(feedback.into());
        }
        section.lines.push(format!("a=label:{label}"));
    }
    sdp.rebuild_bundle();
    sdp.write()
}

/// Lines keep the offer's media type and label: libwebrtc answers `x-data` as `application`, which the service rejects.
pub fn answer_to_ngc(webrtc: &str, offer: &str, conference: bool) -> String {
    let offer = Sdp::parse(offer);
    let mut sdp = Sdp::parse(&to_ngc_in(webrtc, Kind::Answer, conference));
    for (section, offered) in sdp.media.iter_mut().zip(&offer.media) {
        // `a6`: the data line keeps the service's type; a rejected line keeps only its mid and label.
        if section.kind != offered.kind {
            section.kind = offered.kind.clone();
            if !section.rejected() {
                section.protocol = offered.protocol.clone();
                section.payloads = offered.payloads.clone();
            }
        }
        if offered.rejected() && !section.rejected() {
            section.port = "0".into();
            section.payloads = vec!["34".into()];
            section.retain(|l| attr(l, "mid").is_some() || attr(l, "label").is_some());
        }
        if let Some(label) = offered.attr("label") {
            let label = format!("a=label:{label}");
            section.remove_attr("label");
            section.lines.push(label);
        }
        // Back to the service's mid (see `mid_renames`); none where it had none, so a reused mid appears once.
        match offered.attr("mid") {
            Some(mid) => {
                let mid = format!("a=mid:{mid}");
                section.remove_attr("mid");
                section.lines.push(mid);
            }
            None if section.rejected() => section.remove_attr("mid"),
            None => {}
        }
    }
    // `from_ngc` adds `extmap-allow-mixed` for libwebrtc: an answer only echoes what was offered.
    sdp.session.retain(|line| {
        attr(line, "extmap-allow-mixed").is_none()
            || offer
                .session
                .iter()
                .any(|l| attr(l, "extmap-allow-mixed").is_some())
    });
    sdp.rebuild_bundle();
    sdp.write()
}

/// mids of each line of `sdp`, by position.
fn mids(sdp: Option<&str>) -> Vec<Option<String>> {
    sdp.map(|l| {
        Sdp::parse(l)
            .media
            .iter()
            .map(|m| m.attr("mid").map(str::to_owned))
            .collect()
    })
    .unwrap_or_default()
}

/// Renames service mids that reuse one of our rejected lines' mids, which libwebrtc refuses ("Duplicate a=mid value").
pub fn mid_renames(ngc: &str, local: Option<&str>) -> Vec<(String, String)> {
    let sdp = Sdp::parse(ngc);
    let local_mids = mids(local);
    let ours = |index: usize| local_mids.get(index).cloned().flatten();
    // Lines without a mid get ours back by position (`from_ngc`): those mids are taken.
    let mut taken: std::collections::HashSet<String> = sdp
        .media
        .iter()
        .enumerate()
        .filter(|(_, m)| m.attr("mid").is_none())
        .filter_map(|(i, _)| ours(i))
        .collect();
    let mut renames = Vec::new();
    for (index, section) in sdp.media.iter().enumerate() {
        let Some(mid) = section.attr("mid") else {
            continue;
        };
        if ours(index).as_deref() != Some(mid) && taken.contains(mid) {
            let mut fresh = format!("{mid}n");
            while taken.contains(&fresh) {
                fresh.push('n');
            }
            taken.insert(fresh.clone());
            renames.push((mid.to_owned(), fresh));
        } else {
            taken.insert(mid.to_owned());
        }
    }
    renames
}

/// A video m-line the service accepted: a participant's camera or screen can be asked on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSlot {
    pub mid: String,
    /// `a=x-source-streamid`: the `streamMsid` of source requests.
    pub stream_id: u32,
    pub sharing: bool,
}

impl VideoSlot {
    /// `(max-fs, max-mbps, max-fps)`, smaller as more cameras show, like `multiviewResolutionLimits`.
    pub fn receive_limits(&self, cameras: usize) -> (u32, u32, u32) {
        if self.sharing {
            return h264_limits(LABEL_SHARING);
        }
        let (_, _, fps) = h264_limits(LABEL_VIDEO);
        let fs = match cameras {
            0..=2 => 3600,
            3..=4 => 2040,
            _ => 920,
        };
        (fs, fs * fps / 100, fps)
    }
}

/// Live video lines of an NGC description, mids by position when the service leaves them out.
pub fn video_slots(ngc: &str) -> Vec<VideoSlot> {
    Sdp::parse(ngc)
        .media
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == "video" && !m.rejected() && m.direction() != Some("inactive"))
        .filter_map(|(index, m)| {
            Some(VideoSlot {
                mid: m
                    .attr("mid")
                    .map_or_else(|| index.to_string(), str::to_owned),
                stream_id: m.attr("x-source-streamid")?.trim().parse().ok()?,
                sharing: m.attr("label") == Some(LABEL_SHARING),
            })
        })
        .collect()
}

/// NGC blob from the service → SDP libwebrtc accepts; `local` (our own description) restores missing mids.
pub fn from_ngc(ngc: &str, kind: Kind, local: Option<&str>) -> String {
    let mut sdp = Sdp::parse(ngc);
    let local_mids = mids(local);
    let has_fingerprint = sdp
        .session
        .iter()
        .chain(sdp.media.iter().flat_map(|m| m.lines.iter()))
        .any(|l| attr(l, "fingerprint").is_some());
    let session_fingerprint = sdp
        .session
        .iter()
        .find(|l| attr(l, "fingerprint").is_some())
        .cloned();
    let mut any_streams = false;
    let renames = mid_renames(ngc, local);

    for (index, section) in sdp.media.iter_mut().enumerate() {
        let label = section.attr("label").map(str::to_owned);
        if let Some((_, fresh)) = section
            .attr("mid")
            .and_then(|mid| renames.iter().find(|(old, _)| old == mid))
        {
            let fresh = format!("a=mid:{fresh}");
            section.remove_attr("mid");
            section.lines.push(fresh);
        }
        if section.attr("mid").is_none()
            && let Some(Some(mid)) = local_mids.get(index)
        {
            section.lines.push(format!("a=mid:{mid}"));
        }
        if section.kind == "x-data" {
            // No data channel is offered: keep the line rejected.
            section.kind = "application".into();
            section.port = "0".into();
            section.protocol = "UDP/DTLS/SCTP".into();
            section.payloads = vec!["webrtc-datachannel".into()];
            let mid = section.attr("mid").map(str::to_owned);
            section.lines.clear();
            section.lines.push("c=IN IP4 0.0.0.0".into());
            if let Some(mid) = mid {
                section.lines.push(format!("a=mid:{mid}"));
            }
            section.lines.push("a=sctp-port:5000".into());
            section.lines.push("a=inactive".into());
            continue;
        }
        section.protocol = if has_fingerprint {
            "UDP/TLS/RTP/SAVPF".into()
        } else {
            "RTP/SAVPF".into()
        };
        if section.rejected() {
            if section.direction().is_none() {
                section.lines.push("a=inactive".into());
            }
            if section.payloads.is_empty() {
                section.payloads.push("0".into());
            }
            section.retain(|l| !(l.starts_with("a=x-") || attr(l, "label").is_some()));
            continue;
        }

        // Streams: `x-ssrc-range` → explicit ssrc with cname and msid.
        let prefix = match label.as_deref() {
            Some(LABEL_SHARING) => "applicationsharingVideo",
            Some(LABEL_VIDEO) => "mainVideo",
            Some(LABEL_AUDIO) => "mainAudio",
            _ if section.kind == "video" => "mainVideo",
            _ => "mainAudio",
        };
        let has_ssrc = section.attr("ssrc").is_some();
        if !has_ssrc && section.sends() {
            // An FID group may come without a=ssrc: libwebrtc needs both sources, with the same stream identity.
            let mut sources: Vec<String> = if section.kind == "video" {
                section
                    .attrs("ssrc-group")
                    .find_map(|group| group.strip_prefix("FID "))
                    .map(|group| group.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if sources.is_empty()
                && let Some(min) = section
                    .attr("x-ssrc-range")
                    .and_then(|range| range.split('-').next())
            {
                sources.push(min.to_owned());
            }
            let cname = uuid::Uuid::new_v4();
            if let Some(primary) = sources.first().cloned() {
                for source in sources {
                    section.lines.push(format!("a=ssrc:{source} cname:{cname}"));
                    section.lines.push(format!(
                        "a=ssrc:{source} msid:{prefix}-{primary} {prefix}-{primary}"
                    ));
                }
            }
        }
        any_streams |= section.attr("ssrc").is_some();

        // DTLS: fingerprint per line, actpass on offers, no SDES alongside DTLS.
        if let Some(fingerprint) = &session_fingerprint
            && section.attr("fingerprint").is_none()
        {
            section.lines.push(fingerprint.clone());
        }
        if section.attr("fingerprint").is_some() {
            section.remove_attr("crypto");
            if kind == Kind::Offer {
                section.remove_attr("setup");
                section.lines.push("a=setup:actpass".into());
            }
        }
        section.remove_attr("cryptoscale");

        // Candidates: IPv6 ones merged, MS TCP names back to `tcptype`.
        let candidates: Vec<String> = section
            .attrs("candidate")
            .chain(section.attrs("x-candidate-ipv6"))
            .filter_map(Candidate::parse)
            .map(|c| format!("a=candidate:{}", c.into_webrtc().write()))
            .collect();
        section.retain(|l| !l.starts_with("a=candidate:"));
        section.lines.extend(candidates);
        section.remove_attr("rtcp");

        // Header extensions: decoded, then only the supported ones.
        for line in section.lines.iter_mut() {
            if attr(line, "extmap").is_some() {
                *line = line.replace('\\', "/");
            }
        }
        section.retain(|line| {
            attr(line, "extmap")
                .and_then(extmap_uri)
                .is_none_or(|uri| SUPPORTED_EXTENSIONS.contains(&uri))
        });

        // Codecs.
        section.drop_codec("CN/16000");
        section.drop_codec("G722/8000/2");
        if section.kind == "audio" {
            section.remove_attr("rtcp-rsize");
            let opus = section.payloads_of("opus");
            for line in section.lines.iter_mut() {
                if let Some(value) = attr(line, "fmtp")
                    && opus.iter().any(|p| value.split(' ').next() == Some(p))
                {
                    *line = set_fmtp_params(value, |pairs| {
                        if !pairs.iter().any(|(k, _)| k == "usedtx") {
                            pairs.push(("usedtx".into(), "1".into()));
                        }
                    });
                }
            }
        }
        if section.kind == "video" {
            let h264 = section.payloads_of("H264");
            for line in section.lines.iter_mut() {
                if let Some(value) = attr(line, "fmtp")
                    && h264.iter().any(|p| value.split(' ').next() == Some(p))
                {
                    *line = set_fmtp_params(value, |pairs| {
                        pairs.retain(|(k, _)| {
                            [
                                "level-asymmetry-allowed",
                                "packetization-mode",
                                "profile-level-id",
                            ]
                            .contains(&k.as_str())
                        });
                        match pairs.iter_mut().find(|(k, _)| k == "profile-level-id") {
                            Some((_, v)) => *v = H264_PROFILE.into(),
                            None => pairs.push(("profile-level-id".into(), H264_PROFILE.into())),
                        }
                    });
                }
            }
            section.lines.retain(|l| !l.starts_with("b="));
            let at = section
                .lines
                .iter()
                .position(|l| !l.starts_with("c="))
                .unwrap_or(section.lines.len());
            section
                .lines
                .insert(at, format!("b=AS:{VIDEO_BANDWIDTH_KBPS}"));
        }

        // MS-only attributes the engine does not need.
        section.retain(|l| {
            !(l.starts_with("a=x-")
                || attr(l, "label").is_some()
                || attr(l, "rtcp-fb").is_some_and(|v| v.contains("x-message")))
        });
    }
    sdp.session
        .retain(|l| attr(l, "fingerprint").is_none() && !l.starts_with("a=x-"));
    if any_streams {
        sdp.set_session_attr("msid-semantic", "a=msid-semantic: WMS *".into());
    }
    if !sdp.session.iter().any(|l| l == "a=extmap-allow-mixed") {
        sdp.session.push("a=extmap-allow-mixed".into());
    }
    sdp.rebuild_bundle();
    sdp.write()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_service_line_reusing_one_of_our_mids_is_renamed_and_answered_with_its_own() {
        let local = "v=0\r\ns=-\r\nt=0 0\r\n\
            m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:2\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:3\r\n";
        let offer = "v=0\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0 3\r\n\
            m=audio 3478 RTP/SAVP 111\r\na=label:main-audio\r\na=mid:0\r\na=x-ssrc-range:1-1\r\n\
            m=video 0 RTP/SAVP 96\r\n\
            m=video 0 RTP/SAVP 96\r\n\
            m=video 0 RTP/SAVP 96\r\n\
            m=video 3481 RTP/SAVP 96\r\na=label:applicationsharing-video\r\na=mid:3\r\na=sendonly\r\na=x-ssrc-range:5-5\r\n";
        assert_eq!(
            mid_renames(offer, Some(local)),
            [("3".to_owned(), "3n".to_owned())]
        );
        let webrtc = from_ngc(offer, Kind::Offer, Some(local));
        let mids: Vec<&str> = webrtc
            .lines()
            .filter_map(|l| l.strip_prefix("a=mid:"))
            .collect();
        assert_eq!(mids, ["0", "1", "2", "3", "3n"]);
        assert!(webrtc.contains("a=group:BUNDLE 0 3n"), "{webrtc}");

        let answer = "v=0\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0 3n\r\n\
            m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\n\
            m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\n\
            m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:2\r\n\
            m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:3\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:3n\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n";
        let ngc = answer_to_ngc(answer, offer, false);
        assert!(ngc.contains("a=group:BUNDLE 0 3\r\n"), "{ngc}");
        assert!(!ngc.contains("3n"), "{ngc}");
        assert_eq!(ngc.matches("a=mid:3\r\n").count(), 1, "{ngc}");
    }

    #[test]
    fn live_video_lines_with_a_stream_id_are_slots() {
        let ngc = "v=0\r\ns=-\r\nt=0 0\r\n\
            m=audio 3478 RTP/SAVP 111\r\na=mid:0\r\n\
            m=video 3478 RTP/SAVP 96\r\na=x-source-streamid:2042\r\na=label:main-video\r\na=recvonly\r\n\
            m=video 0 RTP/SAVP 96\r\na=mid:2\r\na=x-source-streamid:2043\r\n\
            m=video 3478 RTP/SAVP 96\r\na=mid:3\r\na=x-source-streamid:2044\r\na=label:applicationsharing-video\r\n";
        assert_eq!(
            video_slots(ngc),
            [
                VideoSlot {
                    mid: "1".into(),
                    stream_id: 2042,
                    sharing: false
                },
                VideoSlot {
                    mid: "3".into(),
                    stream_id: 2044,
                    sharing: true
                },
            ]
        );
    }

    #[test]
    fn receiving_lines_of_a_conference_carry_a_placeholder_source() {
        let webrtc = "v=0\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0 1\r\n\
            m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\na=ssrc:77 cname:abc\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:1\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n";
        let ngc = to_ngc_in(webrtc, Kind::Offer, true);
        assert!(ngc.contains("a=ssrc:1 cname:abc\r\n"), "{ngc}");
        assert!(ngc.contains("a=x-ssrc-range:1-1\r\n"), "{ngc}");
        assert!(ngc.contains("a=x-ssrc-range:77-77\r\n"));
        assert!(
            !to_ngc(webrtc, Kind::Offer).contains("a=ssrc:1 "),
            "1:1 calls: unchanged"
        );
    }

    #[test]
    fn answers_keep_the_media_types_and_labels_of_the_service_offer() {
        let offer = "v=0\r\no=- 1 1 IN IP4 10.0.0.1\r\ns=-\r\nt=0 0\r\n\
            m=audio 3478 RTP/SAVP 111\r\na=label:main-audio\r\na=mid:0\r\n\
            m=video 3478 RTP/SAVP 96\r\na=label:main-video\r\na=mid:1\r\n\
            m=video 3478 RTP/SAVP 96\r\na=label:applicationsharing-video\r\na=mid:2\r\n\
            m=video 3478 RTP/SAVP 96\r\na=label:video-2\r\na=mid:3\r\n\
            m=x-data 3478 RTP/SAVP 127\r\na=label:data\r\na=mid:4\r\n";
        let webrtc = from_ngc(offer, Kind::Offer, None);
        assert!(webrtc.contains("m=application 0 UDP/DTLS/SCTP"));
        // libwebrtc's answer: data line rejected as `application`.
        let answer = "v=0\r\no=- 2 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
            m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=sendrecv\r\na=rtpmap:111 opus/48000/2\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:1\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:2\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n\
            m=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:3\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\n\
            m=application 0 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\na=mid:4\r\n";
        let ngc = answer_to_ngc(answer, offer, false);
        let labels: Vec<&str> = ngc
            .lines()
            .filter_map(|l| l.strip_prefix("a=label:"))
            .collect();
        assert_eq!(
            labels,
            [
                "main-audio",
                "main-video",
                "applicationsharing-video",
                "video-2",
                "data"
            ]
        );
        assert!(ngc.contains("m=x-data 0 RTP/SAVP 34\r\n"), "{ngc}");
        assert!(!ngc.contains("m=application"));
        let mixed = answer.replace("t=0 0\r\n", "t=0 0\r\na=extmap-allow-mixed\r\n");
        assert!(!answer_to_ngc(&mixed, offer, false).contains("extmap-allow-mixed"));
    }

    /// Shape of a libwebrtc M144 offer: audio, camera video, inactive sharing video.
    const OFFER: &str = "v=0\r
o=- 4611731400430051336 2 IN IP4 127.0.0.1\r
s=-\r
t=0 0\r
a=group:BUNDLE 0 1 2\r
a=extmap-allow-mixed\r
a=msid-semantic: WMS local\r
m=audio 51234 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126\r
c=IN IP4 203.0.113.5\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=candidate:1 1 udp 2122260223 192.168.1.10 51234 typ host generation 0 network-id 1\r
a=candidate:1 1 udp 2122260223 192.168.1.10 51234 typ host generation 0 network-id 2\r
a=candidate:2 1 udp 1686052607 203.0.113.5 51234 typ srflx raddr 192.168.1.10 rport 51234 generation 0\r
a=candidate:3 1 tcp 1518280447 192.168.1.10 9 typ host tcptype active generation 0\r
a=candidate:4 1 udp 2122260223 abcd.local 5000 typ host\r
a=ice-ufrag:uf\r
a=ice-pwd:pw\r
a=ice-options:trickle\r
a=fingerprint:sha-256 AB:CD\r
a=setup:actpass\r
a=mid:0\r
a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r
a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01\r
a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid\r
a=extmap:5 urn:ietf:params:rtp-hdrext:csrc-audio-level\r
a=sendrecv\r
a=msid:local audio0\r
a=rtcp-mux\r
a=rtcp-rsize\r
a=rtpmap:111 opus/48000/2\r
a=rtcp-fb:111 transport-cc\r
a=fmtp:111 minptime=10;useinbandfec=1\r
a=rtpmap:63 red/48000/2\r
a=fmtp:63 111/111\r
a=rtpmap:9 G722/8000\r
a=rtpmap:0 PCMU/8000\r
a=rtpmap:8 PCMA/8000\r
a=rtpmap:13 CN/8000\r
a=rtpmap:110 telephone-event/48000\r
a=rtpmap:126 telephone-event/8000\r
a=ssrc:1234567890 cname:abc\r
a=ssrc:1234567890 msid:local audio0\r
m=video 9 UDP/TLS/RTP/SAVPF 96 97 102 103 45\r
c=IN IP4 0.0.0.0\r
a=rtcp:9 IN IP4 0.0.0.0\r
a=ice-ufrag:uf\r
a=ice-pwd:pw\r
a=fingerprint:sha-256 AB:CD\r
a=setup:actpass\r
a=mid:1\r
a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time\r
a=extmap:6 http://www.webrtc.org/experiments/rtp-hdrext/playout-delay\r
a=sendrecv\r
a=msid:local video0\r
a=rtcp-mux\r
a=rtcp-rsize\r
a=rtpmap:96 VP8/90000\r
a=rtcp-fb:96 nack\r
a=rtpmap:97 rtx/90000\r
a=fmtp:97 apt=96\r
a=rtpmap:102 H264/90000\r
a=rtcp-fb:102 goog-remb\r
a=rtcp-fb:102 transport-cc\r
a=rtcp-fb:102 ccm fir\r
a=rtcp-fb:102 nack\r
a=rtcp-fb:102 nack pli\r
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r
a=rtpmap:103 rtx/90000\r
a=fmtp:103 apt=102\r
a=rtpmap:45 AV1/90000\r
a=ssrc-group:FID 111 222\r
a=ssrc:111 cname:abc\r
a=ssrc:222 cname:abc\r
m=video 9 UDP/TLS/RTP/SAVPF 102 103\r
c=IN IP4 0.0.0.0\r
a=ice-ufrag:uf\r
a=ice-pwd:pw\r
a=fingerprint:sha-256 AB:CD\r
a=setup:actpass\r
a=mid:2\r
a=inactive\r
a=rtcp-mux\r
a=rtpmap:102 H264/90000\r
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r
a=rtpmap:103 rtx/90000\r
a=fmtp:103 apt=102\r
";

    fn section<'a>(sdp: &'a str, label: &str) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut current = Vec::new();
        for line in sdp.split("\r\n") {
            if line.starts_with("m=") {
                if current.contains(&format!("a=label:{label}").as_str()) {
                    out = current;
                }
                current = Vec::new();
            }
            current.push(line);
        }
        if current.iter().any(|l| *l == format!("a=label:{label}")) {
            out = current;
        }
        out
    }

    #[test]
    fn outbound_offer_follows_the_ms_sdp_rules() {
        let ngc = to_ngc(OFFER, Kind::Offer);
        assert!(ngc.contains("a=msid-semantic: WMS *\r\n"));
        assert!(ngc.contains("a=group:BUNDLE 0 1 2\r\n"));

        let audio = section(&ngc, "main-audio");
        assert_eq!(audio[0], "m=audio 51234 RTP/SAVP 111 63 9 0 8 13 110 126");
        assert!(audio.contains(&"a=x-signaling-fb:* x-message app recv:dsh"));
        assert!(audio.contains(&"a=x-ssrc-range:1234567890-1234567890"));
        assert!(audio.contains(&"a=rtcp:51234"));
        assert!(
            audio.contains(&"a=ssrc:1234567890 msid:local audio0"),
            "ssrc lines are kept"
        );
        assert!(!audio.contains(&"a=rtcp-rsize"));
        // Candidates: deduplicated, no generation/network-id, TCP renamed, mDNS dropped.
        let candidates: Vec<_> = audio
            .iter()
            .filter(|l| l.starts_with("a=candidate:"))
            .collect();
        assert_eq!(
            candidates,
            [
                &"a=candidate:1 1 udp 2122260223 192.168.1.10 51234 typ host",
                &"a=candidate:2 1 udp 1686052607 203.0.113.5 51234 typ srflx raddr 192.168.1.10 rport 51234",
                &"a=candidate:3 1 tcp-act 1518280447 192.168.1.10 9 typ host",
            ]
        );
        assert!(audio.contains(
            &"a=extmap:2 http:\\\\www.webrtc.org\\experiments\\rtp-hdrext\\abs-send-time"
        ));
        assert!(audio.iter().all(|l| !l.contains("csrc-audio-level")));
        assert!(audio.contains(&"a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level"));

        let video = section(&ngc, "main-video");
        assert_eq!(
            video[0], "m=video 51234 RTP/SAVP 102 45 103",
            "bundled port, H264 first, VP8 gone"
        );
        assert!(video.contains(&"c=IN IP4 203.0.113.5"));
        assert!(
            video.iter().all(|l| !l.contains("VP8")
                && !l.contains("apt=96")
                && !l.contains("playout-delay"))
        );
        assert!(video.contains(&"a=x-signaling-fb:* x-message app send:src recv:src,vc"));
        assert!(video.contains(&"a=x-ssrc-range:111-111"));
        for fb in ["goog-remb", "transport-cc", "ccm fir", "nack", "nack pli"] {
            assert!(
                video.contains(&format!("a=rtcp-fb:* {fb}").as_str()),
                "{fb}"
            );
        }
        assert!(video.iter().all(|l| !l.starts_with("a=rtcp-fb:102")));
        assert!(video.contains(&"a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f;max-fs=3600;max-mbps=108000;max-fps=3000"));

        let sharing = section(&ngc, "applicationsharing-video");
        assert!(sharing.contains(&"a=inactive"));
        assert!(
            sharing
                .iter()
                .any(|l| l.ends_with("max-fs=8160;max-mbps=135000;max-fps=1500"))
        );
    }

    #[test]
    fn answers_drop_rtcp_and_rejected_lines_shrink_to_mid_and_label() {
        let answer = OFFER.replace(
            "m=video 9 UDP/TLS/RTP/SAVPF 102 103\r\n",
            "m=video 0 UDP/TLS/RTP/SAVPF 102 103\r\n",
        );
        let ngc = to_ngc(&answer, Kind::Answer);
        assert!(!ngc.contains("a=rtcp:"));
        let sharing = section(&ngc, "applicationsharing-video");
        assert_eq!(
            sharing,
            [
                "m=video 0 RTP/SAVP 34",
                "a=mid:2",
                "a=label:applicationsharing-video",
                ""
            ]
        );
        assert!(ngc.contains("a=group:BUNDLE 0 1\r\n"));
    }

    /// Plausible service answer: MS attributes, ranges instead of ssrc lines, no BUNDLE.
    const ANSWER: &str = "v=0\r
o=- 0 1 IN IP4 52.114.0.1\r
s=session\r
c=IN IP4 52.114.0.1\r
b=CT:10000\r
t=0 0\r
a=x-mediabw:main-video send=4000;recv=4000\r
m=audio 3478 RTP/SAVP 111 13 101\r
c=IN IP4 52.114.0.1\r
a=x-ssrc-range:5000-5000\r
a=x-signaling-fb:* x-message app recv:dsh\r
a=rtcp:3478\r
a=candidate:1 1 UDP 54001 52.114.0.1 3478 typ relay raddr 10.0.0.1 rport 3478\r
a=candidate:2 1 tcp-pass 33553407 52.114.0.1 443 typ relay raddr 10.0.0.1 rport 443\r
a=x-candidate-ipv6:3 1 UDP 54000 2603:1063::1 3478 typ relay\r
a=cryptoscale:1 client AES_CM_128_HMAC_SHA1_80 inline:x\r
a=crypto:2 AES_CM_128_HMAC_SHA1_80 inline:abc|2^31\r
a=ice-ufrag:srv\r
a=ice-pwd:srvpwd\r
a=fingerprint:sha-256 11:22\r
a=setup:active\r
a=mid:0\r
a=extmap:2 http:\\\\www.webrtc.org\\experiments\\rtp-hdrext\\abs-send-time\r
a=extmap:7 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r
a=sendrecv\r
a=rtcp-mux\r
a=rtpmap:111 opus/48000/2\r
a=fmtp:111 useinbandfec=1\r
a=rtpmap:13 CN/16000\r
a=rtpmap:101 telephone-event/8000\r
a=label:main-audio\r
m=video 3478 RTP/SAVP 102 103\r
a=x-ssrc-range:6000-6099\r
a=rtcp-fb:* x-message app send:dsh\r
a=mid:1\r
a=fingerprint:sha-256 11:22\r
a=setup:active\r
a=sendrecv\r
a=rtpmap:102 H264/90000\r
a=fmtp:102 packetization-mode=1;profile-level-id=64001f;max-fs=8160\r
a=rtpmap:103 rtx/90000\r
a=fmtp:103 apt=102\r
a=label:main-video\r
m=video 0 RTP/SAVP 34\r
a=mid:2\r
a=label:applicationsharing-video\r
";

    #[test]
    fn inbound_answer_becomes_standard_webrtc() {
        let sdp = from_ngc(ANSWER, Kind::Answer, None);
        assert!(sdp.contains("a=group:BUNDLE 0 1\r\n"));
        assert!(sdp.contains("a=msid-semantic: WMS *\r\n"));
        assert!(sdp.contains("a=extmap-allow-mixed\r\n"));
        assert!(!sdp.contains("x-mediabw"));
        assert!(!sdp.contains("a=x-"));
        assert!(!sdp.contains("a=label"));
        assert!(!sdp.contains("cryptoscale"));
        assert!(!sdp.contains("a=crypto"), "DTLS wins over SDES");
        assert!(!sdp.contains("x-message"));

        let lines: Vec<&str> = sdp.split("\r\n").collect();
        assert!(
            lines.contains(&"m=audio 3478 UDP/TLS/RTP/SAVPF 111 101"),
            "CN/16000 removed"
        );
        assert!(
            lines.contains(&"a=setup:active"),
            "answers keep the service's role"
        );
        assert!(lines.contains(&"a=ssrc:5000 msid:mainAudio-5000 mainAudio-5000"));
        assert!(lines.iter().any(|l| l.starts_with("a=ssrc:5000 cname:")));
        assert!(lines.contains(&"a=ssrc:6000 msid:mainVideo-6000 mainVideo-6000"));
        assert!(lines.contains(&"a=candidate:2 1 tcp 33553407 52.114.0.1 443 typ relay raddr 10.0.0.1 rport 443 tcptype passive"));
        assert!(lines.contains(&"a=candidate:3 1 UDP 54000 2603:1063::1 3478 typ relay"));
        assert!(
            lines
                .contains(&"a=extmap:2 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time")
        );
        assert!(
            !sdp.contains("ssrc-audio-level"),
            "unsupported extensions dropped"
        );
        assert!(lines.contains(&"a=fmtp:111 useinbandfec=1;usedtx=1"));
        assert!(lines.contains(&"a=fmtp:102 packetization-mode=1;profile-level-id=42C02A"));
        assert!(lines.contains(&"b=AS:6910"));
        assert!(lines.contains(&"m=video 0 UDP/TLS/RTP/SAVPF 34"));
        assert!(!sdp.contains("a=rtcp:"));
    }

    #[test]
    fn incoming_video_declares_both_sources_of_a_fid_group() {
        let ngc = ANSWER.replace(
            "a=x-ssrc-range:6000-6099",
            "a=x-ssrc-range:6000-6099\r\na=ssrc-group:FID 6000 6001",
        );
        let sdp = from_ngc(&ngc, Kind::Answer, None);
        assert!(sdp.contains("a=ssrc:6000 msid:mainVideo-6000 mainVideo-6000"));
        assert!(sdp.contains("a=ssrc:6001 msid:mainVideo-6000 mainVideo-6000"));
        let cname = |source| {
            sdp.lines()
                .find_map(|line| line.strip_prefix(&format!("a=ssrc:{source} cname:")))
                .unwrap()
        };
        assert_eq!(cname(6000), cname(6001));
    }

    #[test]
    fn inbound_offers_get_actpass_and_missing_mids_come_from_the_local_offer() {
        let without_mids = ANSWER.replace("a=mid:0\r\n", "").replace("a=mid:1\r\n", "");
        let sdp = from_ngc(&without_mids, Kind::Offer, Some(OFFER));
        assert!(sdp.contains("a=setup:actpass\r\n"));
        assert!(!sdp.contains("a=setup:active"));
        assert!(sdp.contains("a=mid:0\r\n") && sdp.contains("a=mid:1\r\n"));
        assert!(sdp.contains("a=group:BUNDLE 0 1\r\n"));
    }
}
