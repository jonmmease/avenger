//! Mark-level data scope for faceted plots.

use serde::{Deserialize, Serialize};

use crate::SharingLevel;

/// How much of the current facet hierarchy a mark can see.
///
/// `Level(0)` is the normal fully filtered cell scope. Larger levels drop that
/// many innermost facet predicates. `Level(255)` is global within the local
/// facet tree, which is the broadcast/reference-mark behavior.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FacetDataScope {
    level: SharingLevel,
}

impl Default for FacetDataScope {
    fn default() -> Self {
        Self::FILTERED
    }
}

impl FacetDataScope {
    /// Fully filtered current-cell data.
    pub const FILTERED: Self = Self {
        level: SharingLevel::FREE,
    };

    /// Unfiltered data for the local facet tree.
    pub const BROADCAST: Self = Self {
        level: SharingLevel::GLOBAL,
    };

    /// Create a data scope from a raw sharing level.
    #[inline]
    pub fn level(level: u8) -> Self {
        Self {
            level: SharingLevel::from_raw(level),
        }
    }

    /// Create a data scope from an existing sharing level.
    #[inline]
    pub fn from_sharing_level(level: SharingLevel) -> Self {
        Self { level }
    }

    /// Return this scope's sharing level.
    #[inline]
    pub fn sharing_level(self) -> SharingLevel {
        self.level
    }

    /// Return this scope's raw level.
    #[inline]
    pub fn raw_level(self) -> u8 {
        self.level.raw()
    }

    /// Whether this is fully filtered current-cell data.
    #[inline]
    pub fn is_filtered(self) -> bool {
        self.level.is_free()
    }

    /// Whether this is globally broadcast within the local facet tree.
    #[inline]
    pub fn is_broadcast(self) -> bool {
        self.level.is_global()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_data_scope_constants_and_levels() {
        assert!(FacetDataScope::default().is_filtered());
        assert!(FacetDataScope::FILTERED.is_filtered());
        assert_eq!(FacetDataScope::FILTERED.raw_level(), 0);

        let row_scope = FacetDataScope::level(1);
        assert_eq!(row_scope.raw_level(), 1);
        assert_eq!(row_scope.sharing_level(), SharingLevel::from_raw(1));

        assert!(FacetDataScope::BROADCAST.is_broadcast());
        assert_eq!(FacetDataScope::BROADCAST.raw_level(), 255);
    }
}
