//! ULID-backed identifier newtypes. Each is a distinct type so an item id
//! cannot be passed where a fact id is expected.

/// Define a ULID newtype with `Display` (uppercase), `FromStr`
/// (case-insensitive Crockford base32), and transparent serde.
macro_rules! ulid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(ulid::Ulid);

        impl $name {
            /// Wrap a raw `Ulid`. Crate-internal — the store mints ids.
            ///
            /// Not every newtype built with this macro has a minting call
            /// site yet (e.g. `EntityId`/`FactId` before the graph write
            /// module lands), so this is allowed to look unused per-type.
            #[must_use]
            #[allow(dead_code)]
            pub(crate) const fn from_ulid(u: ulid::Ulid) -> Self { Self(u) }
            /// Underlying ULID.
            #[must_use]
            pub const fn as_ulid(&self) -> ulid::Ulid { self.0 }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Display::fmt(&self.0, f) }
        }
        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> { ulid::Ulid::from_string(s).map(Self) }
        }
    };
}
pub(crate) use ulid_id;

ulid_id!(/// Stable identifier of a graph entity.
    EntityId);
ulid_id!(/// Stable identifier of one fact revision.
    FactId);
