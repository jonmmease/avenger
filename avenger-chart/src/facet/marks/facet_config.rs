#[derive(Clone, Default)]
pub struct FacetRowChannelConfig {
    pub(crate) title: Option<String>,
}

#[derive(Clone, Default)]
pub struct FacetOptions {
    pub(crate) title: Option<String>,
}

impl FacetOptions {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
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
        self
    }
}

