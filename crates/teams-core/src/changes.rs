//! Which parts of the screen changed: changing a [`Tracked`] cell marks the parts it feeds, so the next diff rebuilds only those.
use std::ops::{Deref, DerefMut};

/// Parts of the screen a change may touch; building a part is the costly step of a diff, so untouched parts are skipped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Parts(pub(crate) u32);
impl Parts {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const AVATARS: Self = Self(1);
    pub(crate) const CALENDAR: Self = Self(1 << 1);
    pub(crate) const SHELL: Self = Self(1 << 2);
    pub(crate) const SIDEBAR: Self = Self(1 << 3);
    /// The open conversation's header, without its messages.
    pub(crate) const CONVERSATION: Self = Self(1 << 4);
    pub(crate) const MEMBERS: Self = Self(1 << 5);
    pub(crate) const FILES: Self = Self(1 << 6);
    pub(crate) const NEW_CHAT: Self = Self(1 << 7);
    pub(crate) const COMPOSER: Self = Self(1 << 8);
    pub(crate) const IMAGES: Self = Self(1 << 9);
    pub(crate) const CALLS: Self = Self(1 << 10);
    pub(crate) const CALL: Self = Self(1 << 11);
    pub(crate) const BROWSER: Self = Self(1 << 12);
    pub(crate) const NOTIFICATIONS: Self = Self(1 << 13);
    /// Messages, posts and thread replies: the costly part to build.
    pub(crate) const MESSAGES: Self = Self(1 << 14);
    pub(crate) const TYPING: Self = Self(1 << 15);
    /// The user's own status, status note and automatic replies.
    pub(crate) const ME: Self = Self(1 << 16);
    /// Dialogs that manage chats and teams: chat settings, new team or channel, gallery.
    pub(crate) const MANAGE: Self = Self(1 << 17);
    /// The full search page and its results.
    pub(crate) const RESULTS: Self = Self(1 << 18);
    /// The GIF and sticker picker.
    pub(crate) const GIFS: Self = Self(1 << 19);
    pub(crate) const ALL: Self = Self(u32::MAX);
    pub(crate) fn has(self, part: Self) -> bool {
        self.0 & part.0 != 0
    }
}
impl std::ops::BitOr for Parts {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
impl std::ops::BitOrAssign for Parts {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// The parts a [`Tracked`] cell feeds, as its type parameter.
pub(crate) mod feeds {
    use super::Parts;
    pub(crate) const ALL: u32 = Parts::ALL.0;
    pub(crate) const SHELL: u32 = Parts::SHELL.0;
    pub(crate) const SHELL_SIDEBAR: u32 = Parts::SHELL.0 | Parts::SIDEBAR.0;
    pub(crate) const MESSAGES: u32 = Parts::MESSAGES.0;
    pub(crate) const TYPING: u32 = Parts::TYPING.0;
    /// The message being edited: its row, and the shell that disables other actions.
    pub(crate) const EDITING: u32 = Parts::SHELL.0 | Parts::MESSAGES.0;
    pub(crate) const SIDEBAR: u32 = Parts::SIDEBAR.0;
    pub(crate) const CONVERSATION: u32 = Parts::CONVERSATION.0;
    pub(crate) const COMPOSER: u32 = Parts::COMPOSER.0;
    pub(crate) const COMPOSER_MEMBERS: u32 = Parts::COMPOSER.0 | Parts::MEMBERS.0;
    pub(crate) const NEW_CHAT: u32 = Parts::NEW_CHAT.0;
    /// Downloaded previews: their state in messages, their pixels kept by Swift.
    pub(crate) const IMAGES: u32 = Parts::MESSAGES.0 | Parts::IMAGES.0;
    /// The search filters chats, teams, activities, messages and files.
    pub(crate) const SEARCH: u32 = Parts::SHELL.0
        | Parts::SIDEBAR.0
        | Parts::CONVERSATION.0
        | Parts::MESSAGES.0
        | Parts::FILES.0
        | Parts::BROWSER.0;
    /// The activity feed and its unread count.
    pub(crate) const ACTIVITY: u32 = Parts::SHELL.0 | Parts::SIDEBAR.0;
    /// The calendar, and the meeting times activities show.
    pub(crate) const CALENDAR: u32 = Parts::CALENDAR.0 | Parts::SIDEBAR.0;
    pub(crate) const CALLS: u32 = Parts::CALLS.0 | Parts::CALL.0;
    pub(crate) const CALL: u32 = Parts::CALL.0;
    /// Transfers and pending attachments show in the composer, file lists and messages.
    pub(crate) const FILES: u32 =
        Parts::COMPOSER.0 | Parts::FILES.0 | Parts::BROWSER.0 | Parts::MESSAGES.0;
    /// Presence of the people in direct chats: their row and the open chat's header.
    pub(crate) const PRESENCE: u32 = Parts::SIDEBAR.0 | Parts::CONVERSATION.0;
    /// Settings, and the mute and follow marks of chats, channels and posts.
    pub(crate) const ME: u32 = Parts::ME.0;
    pub(crate) const MANAGE: u32 = Parts::MANAGE.0;
    pub(crate) const RESULTS: u32 = Parts::RESULTS.0;
    pub(crate) const GIFS: u32 = Parts::GIFS.0;
    pub(crate) const NOTIFICATIONS: u32 =
        Parts::NOTIFICATIONS.0 | Parts::SIDEBAR.0 | Parts::CONVERSATION.0 | Parts::MESSAGES.0;
}

/// State plus the parts built from it (`PARTS`, see [`feeds`]): any mutable access marks them all, so no change goes unnoticed.
#[derive(Debug, Default)]
pub(crate) struct Tracked<T, const PARTS: u32> {
    value: T,
    changed: Parts,
}
impl<T, const PARTS: u32> Tracked<T, PARTS> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value,
            changed: Parts::NONE,
        }
    }
    /// Mutable access for a change known to touch only `parts`, a subset of `PARTS`, where precision matters.
    pub(crate) fn narrowly(&mut self, parts: Parts) -> &mut T {
        self.changed |= Parts(parts.0 & PARTS);
        &mut self.value
    }
    /// Marks all of `PARTS` without a change, for a value whose meaning moved with time (a deadline now past).
    pub(crate) fn touch(&mut self) {
        self.changed = Parts(PARTS);
    }
    /// The parts changed since the previous call.
    pub(crate) fn take_changes(&mut self) -> Parts {
        std::mem::take(&mut self.changed)
    }
}
impl<T: PartialEq<U>, U, const PARTS: u32> PartialEq<U> for Tracked<T, PARTS> {
    fn eq(&self, other: &U) -> bool {
        self.value == *other
    }
}
impl<T, const PARTS: u32> Deref for Tracked<T, PARTS> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T, const PARTS: u32> DerefMut for Tracked<T, PARTS> {
    fn deref_mut(&mut self) -> &mut T {
        self.changed = Parts(PARTS);
        &mut self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_mark_their_parts_and_reads_do_not() {
        let mut cell: Tracked<Vec<u8>, { feeds::SHELL_SIDEBAR }> = Tracked::new(vec![]);
        assert!(cell.is_empty());
        assert_eq!(cell.take_changes(), Parts::NONE);
        cell.push(1);
        assert_eq!(cell.take_changes(), Parts::SHELL | Parts::SIDEBAR);
        assert_eq!(cell.take_changes(), Parts::NONE, "taken once");
        cell.narrowly(Parts::SIDEBAR | Parts::MESSAGES).push(2);
        assert_eq!(
            cell.take_changes(),
            Parts::SIDEBAR,
            "never beyond what it feeds"
        );
    }
}
