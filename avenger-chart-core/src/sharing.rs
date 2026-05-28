use crate::Sharing;

/// Canonical representation for child-frame/container sharing levels.
///
/// Semantics:
/// - `0` => free
/// - `N` => shared at level `N`
/// - `255` => global
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct SharingLevel(u8);

impl SharingLevel {
    pub const FREE: Self = Self(0);
    pub const GLOBAL: Self = Self(u8::MAX);

    #[inline]
    pub fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[inline]
    pub fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    pub fn is_free(self) -> bool {
        self.0 == Self::FREE.0
    }

    #[inline]
    pub fn is_global(self) -> bool {
        self.0 == Self::GLOBAL.0
    }

    #[inline]
    pub fn clamp_to_depth(self, depth: u8) -> Self {
        Self(self.0.min(depth))
    }

    #[inline]
    pub fn group_boundary(self, path_depth: u8) -> usize {
        (path_depth as usize).saturating_sub(self.0 as usize)
    }

    #[inline]
    pub fn ancestor_keep_count(self, path_len: usize, path_depth: u8) -> usize {
        let effective = self.clamp_to_depth(path_depth);
        path_len.saturating_sub(effective.0 as usize)
    }
}

impl From<u8> for SharingLevel {
    fn from(value: u8) -> Self {
        Self::from_raw(value)
    }
}

impl From<SharingLevel> for u8 {
    fn from(value: SharingLevel) -> Self {
        value.raw()
    }
}

impl From<Sharing> for SharingLevel {
    fn from(value: Sharing) -> Self {
        Self::from_raw(value.to_level())
    }
}

impl From<SharingLevel> for Sharing {
    fn from(value: SharingLevel) -> Self {
        Sharing::from_level(value.raw())
    }
}

impl PartialEq<u8> for SharingLevel {
    fn eq(&self, other: &u8) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<u8> for SharingLevel {
    fn partial_cmp(&self, other: &u8) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

/// Physical axis associated with a container coordination key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordinationAxis {
    Horizontal,
    Vertical,
    /// Children are positioned by a two-dimensional coordinate system rather
    /// than by a horizontal or vertical band lane.
    Positioned,
}
