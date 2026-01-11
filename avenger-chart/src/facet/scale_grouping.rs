//! Scale sharing tests for facets
//!
//! This module previously contained GridFacet scale grouping infrastructure
//! that has been removed. It now only contains tests for ScaleSharing.

#[cfg(test)]
mod tests {
    use crate::channel::config_traits::ScaleSharing;


    #[test]
    fn test_scale_sharing_from_bool() {
        assert_eq!(ScaleSharing::from(true), ScaleSharing::Shared);
        assert_eq!(ScaleSharing::from(false), ScaleSharing::Free);
    }

    #[test]
    fn test_scale_sharing_serde() {
        // Test serialization
        let shared = ScaleSharing::Shared;
        let json = serde_json::to_string(&shared).unwrap();
        assert_eq!(json, "\"shared\"");

        let free = ScaleSharing::Free;
        let json = serde_json::to_string(&free).unwrap();
        assert_eq!(json, "\"free\"");

        // Test deserialization
        let shared: ScaleSharing = serde_json::from_str("\"shared\"").unwrap();
        assert_eq!(shared, ScaleSharing::Shared);

        let free: ScaleSharing = serde_json::from_str("\"free\"").unwrap();
        assert_eq!(free, ScaleSharing::Free);
    }

    #[test]
    fn test_scale_sharing_to_level() {
        // Free => 0
        assert_eq!(ScaleSharing::Free.to_level(), 0);

        // Level(n) => n for various values
        assert_eq!(ScaleSharing::Level(0).to_level(), 0);
        assert_eq!(ScaleSharing::Level(1).to_level(), 1);
        assert_eq!(ScaleSharing::Level(2).to_level(), 2);
        assert_eq!(ScaleSharing::Level(10).to_level(), 10);
        assert_eq!(ScaleSharing::Level(u8::MAX).to_level(), u8::MAX);

        // Shared => u8::MAX
        assert_eq!(ScaleSharing::Shared.to_level(), u8::MAX);
    }

    #[test]
    fn test_scale_sharing_from_level() {
        // 0 => Free
        assert_eq!(ScaleSharing::from_level(0), ScaleSharing::Free);

        // u8::MAX => Shared
        assert_eq!(ScaleSharing::from_level(u8::MAX), ScaleSharing::Shared);

        // 1..254 => Level(n)
        assert_eq!(ScaleSharing::from_level(1), ScaleSharing::Level(1));
        assert_eq!(ScaleSharing::from_level(2), ScaleSharing::Level(2));
        assert_eq!(ScaleSharing::from_level(127), ScaleSharing::Level(127));
        assert_eq!(ScaleSharing::from_level(254), ScaleSharing::Level(254));
    }

    #[test]
    fn test_scale_sharing_level_round_trip() {
        // Test that from_level(to_level(x)) preserves semantics
        // Note: Level(0) round-trips to Free, Level(u8::MAX) round-trips to Shared
        // This is by design - they are semantically equivalent

        // Free <-> 0
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Free.to_level()),
            ScaleSharing::Free
        );

        // Shared <-> u8::MAX
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Shared.to_level()),
            ScaleSharing::Shared
        );

        // Level(n) for intermediate values
        for n in [1u8, 2, 10, 100, 200, 254] {
            assert_eq!(
                ScaleSharing::from_level(ScaleSharing::Level(n).to_level()),
                ScaleSharing::Level(n)
            );
        }

        // Level(0) normalizes to Free
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(0).to_level()),
            ScaleSharing::Free
        );

        // Level(u8::MAX) normalizes to Shared
        assert_eq!(
            ScaleSharing::from_level(ScaleSharing::Level(u8::MAX).to_level()),
            ScaleSharing::Shared
        );
    }

    #[test]
    fn test_scale_sharing_should_share_with_parent() {
        // Free and Level(0) should NOT share with parent
        assert!(!ScaleSharing::Free.should_share_with_parent());
        assert!(!ScaleSharing::Level(0).should_share_with_parent());

        // Level(1+) should share with parent
        assert!(ScaleSharing::Level(1).should_share_with_parent());
        assert!(ScaleSharing::Level(2).should_share_with_parent());
        assert!(ScaleSharing::Level(10).should_share_with_parent());

        // Shared and Level(u8::MAX) should share
        assert!(ScaleSharing::Shared.should_share_with_parent());
        assert!(ScaleSharing::Level(u8::MAX).should_share_with_parent());

    }

    #[test]
    fn test_scale_sharing_is_fully_shared() {
        // Only Shared and Level(u8::MAX) are fully shared
        assert!(ScaleSharing::Shared.is_fully_shared());
        assert!(ScaleSharing::Level(u8::MAX).is_fully_shared());

        // Free and Level(0..254) are NOT fully shared
        assert!(!ScaleSharing::Free.is_fully_shared());
        assert!(!ScaleSharing::Level(0).is_fully_shared());
        assert!(!ScaleSharing::Level(1).is_fully_shared());
        assert!(!ScaleSharing::Level(100).is_fully_shared());
        assert!(!ScaleSharing::Level(254).is_fully_shared());
    }

    #[test]
    fn test_scale_sharing_is_free() {
        // Only Free and Level(0) are free
        assert!(ScaleSharing::Free.is_free());
        assert!(ScaleSharing::Level(0).is_free());

        // Everything else is not free
        assert!(!ScaleSharing::Level(1).is_free());
        assert!(!ScaleSharing::Level(100).is_free());
        assert!(!ScaleSharing::Level(u8::MAX).is_free());
        assert!(!ScaleSharing::Shared.is_free());
    }

    #[test]
    fn test_scale_sharing_level_serde() {
        // Test Level variant serialization
        let level1 = ScaleSharing::Level(1);
        let json = serde_json::to_string(&level1).unwrap();
        assert_eq!(json, "{\"level\":1}");

        let level42 = ScaleSharing::Level(42);
        let json = serde_json::to_string(&level42).unwrap();
        assert_eq!(json, "{\"level\":42}");

        let level_max = ScaleSharing::Level(u8::MAX);
        let json = serde_json::to_string(&level_max).unwrap();
        assert_eq!(json, "{\"level\":255}");

        // Test Level variant deserialization
        let level1: ScaleSharing = serde_json::from_str("{\"level\":1}").unwrap();
        assert_eq!(level1, ScaleSharing::Level(1));

        let level42: ScaleSharing = serde_json::from_str("{\"level\":42}").unwrap();
        assert_eq!(level42, ScaleSharing::Level(42));

        let level_max: ScaleSharing = serde_json::from_str("{\"level\":255}").unwrap();
        assert_eq!(level_max, ScaleSharing::Level(255));

        // Level(0) serializes as level:0, distinct from "free"
        let level0 = ScaleSharing::Level(0);
        let json = serde_json::to_string(&level0).unwrap();
        assert_eq!(json, "{\"level\":0}");

        let level0: ScaleSharing = serde_json::from_str("{\"level\":0}").unwrap();
        assert_eq!(level0, ScaleSharing::Level(0));
    }

}
