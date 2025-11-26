use crate::channel::config_traits::ScaleSharing;

#[derive(Clone, Default)]
pub struct FacetRowChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) spacing: Option<f32>,
    pub(crate) scale_sharing: Option<ScaleSharing>,
}

#[derive(Clone, Default)]
pub struct FacetOptions {
    pub(crate) title: Option<String>,
    pub(crate) spacing: Option<f32>,
    pub(crate) scale_sharing: Option<ScaleSharing>,
}

impl FacetOptions {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = Some(spacing);
        self
    }

    /// Configure scale sharing mode for this facet variable's domain.
    ///
    /// For nested facets (e.g., FacetRow inside FacetColumn):
    /// - `Shared`: All outer cells use the same domain computed from the full dataset.
    ///   This creates a grid-like structure where empty cells may appear.
    /// - `Free` (default): Each outer cell computes its own domain from filtered data.
    ///   Different outer cells may have different numbers of inner cells.
    pub fn with_scale_sharing(mut self, mode: ScaleSharing) -> Self {
        self.scale_sharing = Some(mode);
        self
    }

    /// Share this facet variable's domain across all facets (compute from full dataset)
    pub fn share_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Shared)
    }

    /// Make this facet variable's domain independent per facet (compute from filtered data)
    pub fn free_scale(self) -> Self {
        self.with_scale_sharing(ScaleSharing::Free)
    }

    /// Share domain within each row (across columns)
    pub fn share_scale_in_rows(self) -> Self {
        self.with_scale_sharing(ScaleSharing::SharedInRow)
    }

    /// Share domain within each column (across rows)
    pub fn share_scale_in_columns(self) -> Self {
        self.with_scale_sharing(ScaleSharing::SharedInColumn)
    }
}

impl FacetRowChannelConfig {
    pub fn facet<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetOptions) -> FacetOptions,
    {
        let opts = f(FacetOptions::default());
        self.title = opts.title;
        self.spacing = opts.spacing;
        self.scale_sharing = opts.scale_sharing;
        self
    }
}

#[derive(Clone, Default)]
pub struct FacetColChannelConfig {
    pub(crate) title: Option<String>,
    pub(crate) spacing: Option<f32>,
    pub(crate) scale_sharing: Option<ScaleSharing>,
}

impl FacetColChannelConfig {
    pub fn facet<F>(mut self, f: F) -> Self
    where
        F: FnOnce(FacetOptions) -> FacetOptions,
    {
        let opts = f(FacetOptions::default());
        self.title = opts.title;
        self.spacing = opts.spacing;
        self.scale_sharing = opts.scale_sharing;
        self
    }
}
