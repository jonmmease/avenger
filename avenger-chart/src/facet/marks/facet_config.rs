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
    pub(crate) position: Option<String>,
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
    /// Note: Free and Shared are normalized to Level(0) and Level(255) internally.
    pub fn with_scale_sharing(mut self, mode: ScaleSharing) -> Self {
        // Normalize Free/Shared to Level representation for internal consistency
        self.scale_sharing = Some(mode.to_normalized());
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

    /// Set the position of the facet labels.
    ///
    /// - `"top"` (default): Labels appear above the plot area
    /// - `"bottom"`: Labels appear below the plot area
    pub fn position(mut self, position: impl Into<String>) -> Self {
        self.position = Some(position.into());
        self
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
    pub(crate) position: Option<String>,
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
        self.position = opts.position;
        self
    }
}
