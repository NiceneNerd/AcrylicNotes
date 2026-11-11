//! Stable identity types shared by both frontends.
//!
//! Neither frontend may use vector positions as identity: tabs, tree nodes,
//! search results, and async effects can all change shape while work is in
//! flight (restructure plan §5.1). Every id is a u64-backed newtype so it is
//! cheap to store in keyed widget children (Reactor reconciles keyed
//! children) and trivially serializable in saved state.

use serde::{Deserialize, Serialize};

/// A u64-backed stable identifier.
pub trait Id:
    Copy + Clone + PartialEq + Eq + core::hash::Hash + core::fmt::Debug + core::fmt::Display
{
    /// Wrap a raw value.
    fn from_raw(raw: u64) -> Self;
    /// The raw value (for keyed widgets and serialization).
    fn raw(self) -> u64;
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(u64);

        impl $name {
            /// Wrap a raw value.
            #[must_use]
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            /// The raw value (for keyed widgets and serialization).
            #[must_use]
            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }

        impl Id for $name {
            fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            fn raw(self) -> u64 {
                self.0
            }
        }
    };
}

define_id! {
    /// An open tab. Stable across reordering, closing other tabs, and async
    /// work started on the tab's behalf.
    TabId
}

define_id! {
    /// A note in the vault index.
    NoteId
}

define_id! {
    /// A modal dialog or banner instance, so at most one instance of each
    /// decision survives stale results.
    DialogId
}

define_id! {
    /// One vault-wide search request. Results from older requests are stale
    /// and must be dropped.
    SearchRequestId
}

define_id! {
    /// A node in the file tree view model (folder or note).
    TreeNodeId
}

define_id! {
    /// A revision of an editor document's text. Async results keyed by a
    /// revision must be ignored when the text moved on.
    EditorRevision
}

define_id! {
    /// One emitted effect. Every async effect that can race carries one so a
    /// late completion can be checked against the current state.
    EffectId
}

define_id! {
    /// An attachment/file entry rendered in the reading view.
    AttachmentId
}

/// A monotonically increasing allocator for one id kind.
///
/// Frontends hold one per kind in native state; the shared reducer holds one
/// per kind in `AppState`. Allocation never repeats within a session, so ids
/// remain unique even after items are removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocator {
    next_raw: u64,
}

impl IdAllocator {
    /// An allocator that hands out ids starting at 0.
    #[must_use]
    pub const fn new() -> Self {
        Self { next_raw: 0 }
    }

    /// Allocate the next id of type `T`.
    pub fn allocate<T: Id>(&mut self) -> T {
        let id = T::from_raw(self.next_raw);
        self.next_raw += 1;
        id
    }

    /// The raw value that the next allocation will return. Useful for
    /// asserting exact sequences in tests.
    #[must_use]
    pub const fn peek(&self) -> u64 {
        self.next_raw
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_opaque_wrappers() {
        let tab = TabId::new(7);
        assert_eq!(tab.raw(), 7);
        assert_eq!(tab.to_string(), "TabId(7)");
        // Two ids of the same kind with the same raw value are equal.
        assert_eq!(tab, TabId::from_raw(7));
        // Ordering follows the raw value (useful for BTree keyed storage).
        assert!(TabId::new(3) < tab);
    }

    #[test]
    fn ids_round_trip_through_serialization() {
        let id = SearchRequestId::new(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let back: SearchRequestId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn allocator_hands_out_monotonic_ids() {
        let mut allocator = IdAllocator::new();
        assert_eq!(allocator.peek(), 0);
        let first: TabId = allocator.allocate();
        let second: TabId = allocator.allocate();
        let third: TabId = allocator.allocate();
        assert_eq!(first.raw(), 0);
        assert_eq!(second.raw(), 1);
        assert_eq!(third.raw(), 2);
        // Distinct kinds never collide by construction (separate allocators).
        let mut note_allocator = IdAllocator::new();
        let note: NoteId = note_allocator.allocate();
        assert_eq!(note.raw(), 0);
    }

    #[test]
    fn allocator_round_trips_through_serialization() {
        let mut allocator = IdAllocator::new();
        let _a: EffectId = allocator.allocate();
        let _b: EffectId = allocator.allocate();
        let json = serde_json::to_string(&allocator).unwrap();
        let mut restored: IdAllocator = serde_json::from_str(&json).unwrap();
        let c: EffectId = restored.allocate();
        assert_eq!(c.raw(), 2);
    }
}
