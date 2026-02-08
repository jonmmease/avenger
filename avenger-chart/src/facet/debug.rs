#[inline]
pub(crate) fn layout_enabled() -> bool {
    std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok()
}
