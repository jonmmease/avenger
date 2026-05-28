/// Hierarchical sharing modes for coordinated chart state.
///
/// The same `Free` / `Level(N)` / `Shared` vocabulary is used for scale
/// domains, facet slots, guides, legends, and scoped params.
///
/// # Implementation Note: Unified UNION Semantics
///
/// All sharing modes (`Shared`, `Free`, and `Level(N)`) use UNION semantics via
/// `extend_with_domain_extents`. This ensures that local domains can only grow
/// (never shrink) when shared extents are applied.
///
/// While `Shared` is semantically equivalent to `Level(u8::MAX)`, they currently
/// follow different code paths internally for historical reasons. When you need to
/// check if a mode represents "fully shared" behavior, use [`Sharing::is_fully_shared`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sharing {
    /// Share across all facets.
    /// Equivalent to Level(u8::MAX)
    Shared,
    /// Independent per facet.
    /// Equivalent to Level(0)
    Free,
    /// Hierarchical level-based sharing for nested facets.
    /// Level(0) = Free (independent per cell)
    /// Level(1) = Share with immediate parent facet
    /// Level(N) = Share N levels up in the hierarchy
    /// Level(u8::MAX) = Shared (global across all facets)
    #[serde(rename = "level")]
    Level(u8),
}

impl From<bool> for Sharing {
    fn from(v: bool) -> Self {
        if v { Sharing::Shared } else { Sharing::Free }
    }
}

impl Sharing {
    /// Convert this sharing mode to a level value
    ///
    /// - Free -> 0
    /// - Level(n) -> n
    /// - Shared -> u8::MAX
    pub fn to_level(self) -> u8 {
        match self {
            Sharing::Free => 0,
            Sharing::Level(n) => n,
            Sharing::Shared => u8::MAX,
        }
    }

    /// Create a Sharing from a level value
    ///
    /// Returns normalized Level values for internal consistency:
    /// - 0 -> Level(0)
    /// - u8::MAX -> Level(255)
    /// - n -> Level(n)
    pub fn from_level(level: u8) -> Self {
        Sharing::Level(level)
    }

    /// Check if this is free (independent per facet cell)
    ///
    /// Returns true for Free and Level(0)
    pub fn is_free(self) -> bool {
        self.to_level() == 0
    }

    /// Check if this sharing mode shares with the parent container.
    ///
    /// Returns true for Level(1+), Shared.
    /// Returns false for Free, Level(0).
    ///
    /// This is useful for determining whether nested state should coordinate
    /// with its parent container.
    pub fn should_share_with_parent(self) -> bool {
        self.to_level() > 0
    }

    /// Check if this is fully shared (global across all facets).
    ///
    /// Returns true for Shared and Level(u8::MAX).
    /// These modes share domains across all nesting levels.
    pub fn is_fully_shared(self) -> bool {
        matches!(self, Sharing::Shared | Sharing::Level(u8::MAX))
    }

    /// Normalize to Level representation for internal consistency.
    ///
    /// - Free -> Level(0)
    /// - Shared -> Level(u8::MAX)
    /// - Level(n) -> Level(n) (unchanged)
    ///
    /// This ensures all internal code only needs to handle the Level variant.
    pub fn to_normalized(self) -> Self {
        match self {
            Sharing::Free => Sharing::Level(0),
            Sharing::Shared => Sharing::Level(u8::MAX),
            level => level,
        }
    }
}
