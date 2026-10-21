//! Deferred work from the state machine: futures resolving to a message, plus effects only the native shell can perform.
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Work the macOS app performs on behalf of the core.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum Effect {
    /// Copy a value to the general pasteboard.
    Copy {
        value: String,
    },
    /// Show a banner; `id` replaces an earlier one with the same id, and `conversation_id` groups a conversation's banners.
    Notify {
        id: String,
        conversation_id: String,
        reply_chain_id: Option<String>,
        title: String,
        subtitle: Option<String>,
        body: String,
        /// `None`: silent.
        sound: Option<crate::app::notifications::NotificationSound>,
        urgent: bool,
        /// Offer a quick reply field.
        can_reply: bool,
    },
    /// Remove the delivered banners of this conversation (it was read).
    WithdrawNotifications {
        conversation_id: String,
    },
    /// Show the Microsoft sign-in window at this authorization URL.
    OpenLogin {
        url: String,
    },
    /// Close the sign-in window if it is still open.
    CloseLogin,
    /// Load this `prompt=none` URL in an invisible web view sharing the sign-in cookies; answer `LoginReturned` or `SilentLoginFailed`.
    SilentLogin {
        url: String,
    },
    /// Delete the Microsoft cookies kept for silent sign-ins.
    ForgetLogin,
    /// The emoticon catalog arrived or changed language: fetch `composer_emojis` again.
    EmojiCatalogChanged,
    /// The sticker catalog arrived or changed language: fetch `sticker_categories` again.
    StickerCatalogChanged,
    /// Show this downloaded file in Quick Look.
    QuickLook {
        path: String,
    },
    /// A download the user asked for is on disk.
    Downloaded {
        path: String,
    },
    /// Create the call's peer connection, answering `remote_offer` or else making an offer; the SDP returns as `CallAction::LocalDescription`.
    MediaPrepare {
        call_id: String,
        ice_servers: Vec<crate::app::call::IceServerView>,
        video: bool,
        /// Start with the microphone off.
        muted: bool,
        /// Play the other participants on this device.
        speaker: bool,
        /// Video m-lines for cameras (ours included) when offering; the sharing line comes on top.
        camera_lines: u32,
        remote_offer: Option<String>,
    },
    /// The other side's answer to our offer.
    MediaRemoteDescription {
        call_id: String,
        sdp: String,
    },
    /// Early media received before the final acceptance.
    MediaRemoteProvisionalDescription {
        call_id: String,
        sdp: String,
    },
    MediaSetMuted {
        call_id: String,
        muted: bool,
    },
    MediaSetCamera {
        call_id: String,
        on: bool,
    },
    MediaSetSpeaker {
        call_id: String,
        on: bool,
    },
    /// Bounds the encoder of one of our sending lines and asks a keyframe (`controlVideoStreaming`).
    MediaSendLimits {
        call_id: String,
        /// WebRTC mid of the line.
        mid: String,
        sharing: bool,
        max_fs: u32,
        max_fps: f64,
        max_bitrate: u32,
        keyframe: bool,
    },
    /// Captures the screen on the sharing line, or stops: a renegotiation follows.
    MediaSetScreenShare {
        call_id: String,
        on: bool,
    },
    /// The service renegotiates: answer this offer (`CallAction::RenegotiationAnswer`).
    MediaRemoteOffer {
        call_id: String,
        sdp: String,
    },
    /// The network changed: restart ICE, then send the new offer (`LocalDescription`).
    MediaRestartIce {
        call_id: String,
    },
    /// Stop capture and close the peer connection.
    MediaClose {
        call_id: String,
    },
}

/// Hands messages from a long-running job (a push channel) back to the core.
pub struct Sink<M>(std::sync::Arc<dyn Fn(M) + Send + Sync>);
impl<M> Clone for Sink<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<M> Sink<M> {
    pub fn new(send: impl Fn(M) + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(send))
    }
    pub fn send(&self, msg: M) {
        (self.0)(msg)
    }
}

pub type Subscription<M> = Box<dyn FnOnce(Sink<M>) -> BoxFuture<()> + Send>;

pub enum Job<M> {
    Future(BoxFuture<M>),
    Effect(Effect),
    /// A job that may send any number of messages until it ends.
    Subscribe(Subscription<M>),
    /// A follow-up message, applied right after the one that returned it.
    Done(M),
}

#[must_use]
pub struct Task<M>(pub Vec<Job<M>>);

impl<M: Send + 'static> Task<M> {
    pub fn none() -> Self {
        Self(Vec::new())
    }
    pub fn perform<T: Send + 'static>(
        future: impl Future<Output = T> + Send + 'static,
        map: impl FnOnce(T) -> M + Send + 'static,
    ) -> Self {
        Self(vec![Job::Future(Box::pin(
            async move { map(future.await) },
        ))])
    }
    pub fn effect(effect: Effect) -> Self {
        Self(vec![Job::Effect(effect)])
    }
    /// Applies `msg` as soon as the current message is handled, before any diff.
    pub fn done(msg: M) -> Self {
        Self(vec![Job::Done(msg)])
    }
    pub fn subscribe<F: Future<Output = ()> + Send + 'static>(
        start: impl FnOnce(Sink<M>) -> F + Send + 'static,
    ) -> Self {
        Self(vec![Job::Subscribe(Box::new(move |sink| {
            Box::pin(start(sink))
        }))])
    }
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self(tasks.into_iter().flat_map(|t| t.0).collect())
    }
    #[cfg(test)]
    pub fn effects(&self) -> impl Iterator<Item = &Effect> {
        self.0.iter().filter_map(|job| match job {
            Job::Effect(e) => Some(e),
            Job::Future(_) | Job::Subscribe(_) | Job::Done(_) => None,
        })
    }
}
