//! Typed client for the Teams chat service; response types keep unknown fields in `extra`.

mod apps;
mod client;
mod csa;
mod error;
pub mod extensions;
mod fun;
mod media;
mod new_chat;
mod notifications;
mod ooo;
mod path;
mod people;
mod presence;
mod reactions;
pub mod receipts;
mod search;
mod service;
mod teams_admin;
pub mod types;
mod url_preview;

pub use apps::AppDefinition;
pub use client::{ChatClient, MessagesQuery};
pub use csa::*;
pub use error::Error;
pub use extensions::{
    ExtensionAnswer, ExtensionCommand, ExtensionItem, ExtensionParameter, ExtensionTarget,
    ExtensionsLookup, MessageExtension, TaskModule,
};
pub use fun::{Gif, giphy_rating};
pub use notifications::{
    ChannelNotificationSettings, ChannelSettingsResult, FollowState, NssChoice,
    PartialChannelSettings,
};
pub use ooo::{AutomaticReplies, ExternalAudience, RepliesStatus};
pub use people::Person;
pub use presence::{ChosenAvailability, NOTE_LIMIT as STATUS_NOTE_LIMIT, Presence};
pub use reactions::Reaction;
pub use search::{MessageHit, SearchFilters, SearchIn, SearchPage};
pub use teams_admin::{GalleryPage, GalleryTeam, NewTeam, TeamVisibility};
pub use types::*;
pub use url_preview::UrlInfo;
