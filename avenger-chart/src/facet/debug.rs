#[inline]
pub(crate) fn env_layout_overlay_enabled() -> bool {
    std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
}

#[inline]
pub(crate) fn resolve_layout_overlay_enabled(option_enabled: bool) -> bool {
    resolve_layout_overlay_enabled_with_env(env_layout_overlay_enabled(), option_enabled)
}

#[inline]
pub(crate) fn resolve_layout_overlay_enabled_with_env(
    env_enabled: bool,
    option_enabled: bool,
) -> bool {
    env_enabled || option_enabled
}

#[cfg(test)]
mod tests {
    use super::resolve_layout_overlay_enabled_with_env;

    #[test]
    fn debug_layout_env_override_is_respected() {
        assert!(!resolve_layout_overlay_enabled_with_env(false, false));
        assert!(resolve_layout_overlay_enabled_with_env(false, true));
        assert!(resolve_layout_overlay_enabled_with_env(true, false));
        assert!(resolve_layout_overlay_enabled_with_env(true, true));
    }
}
