use crate::channel::config_traits::ScaleSharing;

/// Canonical internal representation for facet sharing semantics.
///
/// Semantics:
/// - `0` => Free
/// - `N` => Level(N)
/// - `255` => Global (fully shared)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SharingLevel(u8);

impl SharingLevel {
    pub(crate) const FREE: Self = Self(0);
    pub(crate) const GLOBAL: Self = Self(u8::MAX);

    #[inline]
    pub(crate) fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[inline]
    pub(crate) fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    pub(crate) fn is_free(self) -> bool {
        self.0 == Self::FREE.0
    }

    #[inline]
    pub(crate) fn is_global(self) -> bool {
        self.0 == Self::GLOBAL.0
    }

    #[inline]
    pub(crate) fn clamp_to_depth(self, depth: u8) -> Self {
        Self(self.0.min(depth))
    }

    #[inline]
    pub(crate) fn group_boundary(self, facet_depth: u8) -> usize {
        (facet_depth as usize).saturating_sub(self.0 as usize)
    }

    #[inline]
    pub(crate) fn ancestor_keep_count(self, path_len: usize, facet_depth: u8) -> usize {
        let effective = self.clamp_to_depth(facet_depth);
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

impl From<ScaleSharing> for SharingLevel {
    fn from(value: ScaleSharing) -> Self {
        Self::from_raw(value.to_level())
    }
}

impl From<SharingLevel> for ScaleSharing {
    fn from(value: SharingLevel) -> Self {
        // Keep normalized internal representation.
        ScaleSharing::from_level(value.raw())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_level_raw_roundtrip() {
        for raw in [0_u8, 1, 7, 254, 255] {
            let level = SharingLevel::from_raw(raw);
            assert_eq!(level.raw(), raw);
            assert_eq!(u8::from(level), raw);
        }
    }

    #[test]
    fn sharing_level_free_and_global_detection() {
        assert!(SharingLevel::FREE.is_free());
        assert!(!SharingLevel::FREE.is_global());
        assert!(SharingLevel::GLOBAL.is_global());
        assert!(!SharingLevel::GLOBAL.is_free());
    }

    #[test]
    fn sharing_level_clamp_to_depth() {
        assert_eq!(SharingLevel::from_raw(0).clamp_to_depth(3).raw(), 0);
        assert_eq!(SharingLevel::from_raw(2).clamp_to_depth(3).raw(), 2);
        assert_eq!(SharingLevel::from_raw(5).clamp_to_depth(3).raw(), 3);
        assert_eq!(SharingLevel::GLOBAL.clamp_to_depth(4).raw(), 4);
    }

    #[test]
    fn sharing_level_scale_sharing_conversions_normalized() {
        assert_eq!(
            SharingLevel::from(ScaleSharing::Free),
            SharingLevel::from_raw(0)
        );
        assert_eq!(
            SharingLevel::from(ScaleSharing::Shared),
            SharingLevel::GLOBAL
        );
        assert_eq!(
            SharingLevel::from(ScaleSharing::Level(3)),
            SharingLevel::from_raw(3)
        );

        let back_to_scale: ScaleSharing = SharingLevel::GLOBAL.into();
        assert_eq!(back_to_scale, ScaleSharing::Level(255));
    }
}
