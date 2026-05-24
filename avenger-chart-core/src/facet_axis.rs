#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FacetAxis {
    Row,
    Column,
}

impl FacetAxis {
    #[inline]
    pub fn scale_name(self) -> &'static str {
        match self {
            FacetAxis::Row => "row",
            FacetAxis::Column => "column",
        }
    }

    #[inline]
    pub fn coordination_key_prefix(self) -> &'static str {
        match self {
            FacetAxis::Row => "row",
            FacetAxis::Column => "col",
        }
    }
}
