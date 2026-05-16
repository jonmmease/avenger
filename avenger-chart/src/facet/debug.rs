use crate::render::LayoutDebugOverlayMode;

#[inline]
pub(crate) fn env_layout_overlay_enabled() -> bool {
    std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
}

#[inline]
pub(crate) fn resolve_layout_overlay_mode(
    option_mode: LayoutDebugOverlayMode,
) -> LayoutDebugOverlayMode {
    resolve_layout_overlay_mode_with_env(env_layout_overlay_enabled(), option_mode)
}

#[inline]
pub(crate) fn resolve_layout_overlay_mode_with_env(
    env_enabled: bool,
    option_mode: LayoutDebugOverlayMode,
) -> LayoutDebugOverlayMode {
    if option_mode.enabled() {
        option_mode
    } else if env_enabled {
        LayoutDebugOverlayMode::Components
    } else {
        LayoutDebugOverlayMode::Off
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_layout_overlay_mode_with_env;
    use crate::render::LayoutDebugOverlayMode;

    #[test]
    fn debug_layout_env_override_is_respected() {
        assert_eq!(
            resolve_layout_overlay_mode_with_env(false, LayoutDebugOverlayMode::Off),
            LayoutDebugOverlayMode::Off
        );
        assert_eq!(
            resolve_layout_overlay_mode_with_env(false, LayoutDebugOverlayMode::Components),
            LayoutDebugOverlayMode::Components
        );
        assert_eq!(
            resolve_layout_overlay_mode_with_env(true, LayoutDebugOverlayMode::Off),
            LayoutDebugOverlayMode::Components
        );
        assert_eq!(
            resolve_layout_overlay_mode_with_env(true, LayoutDebugOverlayMode::AllocationDemand),
            LayoutDebugOverlayMode::AllocationDemand
        );
    }
}
