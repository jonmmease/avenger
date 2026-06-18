use std::{marker::PhantomData, sync::Arc};

use datafusion::dataframe::DataFrame;

use crate::{
    AvengerChartError, CoordinateSystemCore, CoordinationScope, DataContext, DataTransform,
    DataTransformCompileContext, FacetDataScope, Mark, MarkDataMode, ScaleInferenceHint, StoreData,
    validate_structural_id,
};

/// A recursive chart-layer container for marks that share data preparation.
///
/// `MarkGroup` is authoring infrastructure only. It does not create scenegraph
/// groups or independent render surfaces; compilation flattens primitive child
/// marks while retaining group data metadata for runtime preparation. External
/// compound marks can lower themselves into `MarkGroup` values with generated
/// primitive marks and transforms.
#[derive(Clone)]
pub struct MarkGroup<C: CoordinateSystemCore> {
    pub(crate) id: Option<String>,
    pub(crate) data: DataContext,
    pub(crate) data_mode: MarkDataMode,
    pub(crate) facet_data_scope: FacetDataScope,
    pub(crate) children: Vec<PlotMark<C>>,
    pub(crate) scale_inference_hints: Vec<ScaleInferenceHint>,
    _phantom: PhantomData<fn() -> C>,
}

impl<C: CoordinateSystemCore> Default for MarkGroup<C> {
    fn default() -> Self {
        Self {
            id: None,
            data: DataContext::default(),
            data_mode: MarkDataMode::Inherit,
            facet_data_scope: FacetDataScope::FILTERED,
            children: Vec::new(),
            scale_inference_hints: Vec::new(),
            _phantom: PhantomData,
        }
    }
}

impl<C: CoordinateSystemCore> MarkGroup<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a structural id for this group.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set explicit data for this group.
    pub fn data(mut self, dataframe: DataFrame) -> Self {
        self.data = DataContext::new(dataframe);
        self.data_mode = MarkDataMode::Inherit;
        self
    }

    /// Set this group's data source to rows from a mutable chart store.
    pub fn data_store(mut self, data: StoreData) -> Self {
        self.data = DataContext::store_data(data);
        self.data_mode = MarkDataMode::Inherit;
        self
    }

    /// Apply a data transform and configure this group using the output handle.
    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_free(transform, f)
    }

    /// Apply a data transform at fully filtered/free facet scope.
    pub fn transform_free<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    /// Apply a data transform at a specific logical facet sharing level.
    pub fn transform_level<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Level(level), transform, f)
    }

    /// Apply a data transform at shared/global facet scope.
    pub fn transform_shared<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Shared, transform, f)
    }

    /// Apply a data transform at the specified facet sharing scope.
    pub fn transform_with_scope<T, F>(
        mut self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled_transform, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("Failed to build data transform");
        self.data = self.data.with_transform_stage(scope, compiled_transform);
        f(self, output)
    }

    /// Apply a no-output data transform and configure this group without a
    /// dummy output argument.
    pub fn transform_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_free_no_output(transform, f)
    }

    /// Apply a no-output data transform at fully filtered/free facet scope.
    pub fn transform_free_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Free, transform, f)
    }

    /// Apply a no-output data transform at a specific logical facet sharing level.
    pub fn transform_level_no_output<T, F>(self, level: u8, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Level(level), transform, f)
    }

    /// Apply a no-output data transform at shared/global facet scope.
    pub fn transform_shared_no_output<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope_no_output(CoordinationScope::Shared, transform, f)
    }

    /// Apply a no-output data transform at the specified facet sharing scope.
    pub fn transform_with_scope_no_output<T, F>(
        self,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform<Output = ()>,
        F: FnOnce(Self) -> Self,
    {
        self.transform_with_scope(scope, transform, |group, ()| f(group))
    }

    /// Control how this group's inherited data is selected in faceted plots.
    pub fn facet_data_scope(mut self, scope: FacetDataScope) -> Self {
        self.facet_data_scope = scope;
        self
    }

    /// Seed this group with an existing authoring data context.
    ///
    /// This is intended for compound marks that collect ordinary mark data and
    /// transforms on their public builder, then lower into a root `MarkGroup`
    /// without losing that accumulated data state.
    pub fn with_data_context(mut self, data: DataContext, data_mode: MarkDataMode) -> Self {
        self.data = data;
        self.data_mode = data_mode;
        self
    }

    /// Set the faceting data scope by level.
    pub fn facet_data_level(mut self, level: u8) -> Self {
        self.facet_data_scope = FacetDataScope::level(level);
        self
    }

    /// Make this group's data branch broadcast to every facet.
    pub fn broadcast_to_facets(mut self) -> Self {
        self.facet_data_scope = FacetDataScope::BROADCAST;
        self
    }

    /// Add a primitive mark, group, or compound mark expansion to this group.
    pub fn mark<M>(mut self, mark: M) -> Self
    where
        M: IntoPlotMark<C>,
    {
        self.children.extend(mark.into_plot_marks());
        self
    }

    pub fn id_ref(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn data_context(&self) -> &DataContext {
        &self.data
    }

    pub fn data_mode(&self) -> MarkDataMode {
        self.data_mode
    }

    pub fn facet_data_scope_value(&self) -> FacetDataScope {
        self.facet_data_scope
    }

    pub fn children(&self) -> &[PlotMark<C>] {
        &self.children
    }

    /// Add a scale type inference hint for descendant primitive marks.
    ///
    /// Compound marks use hints when their generated primitive marks do not
    /// fully express the semantic scale preference. Explicit user-authored
    /// scale configuration still wins over hints.
    pub fn scale_inference_hint(mut self, hint: ScaleInferenceHint) -> Self {
        self.scale_inference_hints.push(hint);
        self
    }

    #[doc(hidden)]
    pub fn scale_inference_hints(&self) -> &[ScaleInferenceHint] {
        &self.scale_inference_hints
    }

    #[doc(hidden)]
    pub fn validate_id(&self) -> Result<(), AvengerChartError> {
        if let Some(id) = self.id.as_deref() {
            validate_structural_id("mark group", id)?;
        }
        Ok(())
    }
}

/// Public conversion trait accepted by `Plot::mark` and `MarkGroup::mark`.
///
/// Primitive marks, mark groups, and future compound mark builders all lower to
/// one or more recursive plot elements.
pub trait IntoPlotMark<C: CoordinateSystemCore>: Send + Sync + 'static {
    fn into_plot_marks(self) -> Vec<PlotMark<C>>;
}

impl<C> IntoPlotMark<C> for MarkGroup<C>
where
    C: CoordinateSystemCore,
{
    fn into_plot_marks(self) -> Vec<PlotMark<C>> {
        vec![PlotMark::from_group(self)]
    }
}

/// Opaque recursive plot element used by chart authoring builders.
#[derive(Clone)]
pub struct PlotMark<C: CoordinateSystemCore> {
    kind: PlotMarkKind<C>,
}

impl<C: CoordinateSystemCore> PlotMark<C> {
    pub fn from_mark<M>(mark: M) -> Self
    where
        M: Mark<C> + 'static,
    {
        Self {
            kind: PlotMarkKind::Primitive(Arc::new(mark)),
        }
    }

    #[doc(hidden)]
    pub fn from_mark_arc(mark: Arc<dyn Mark<C>>) -> Self {
        Self {
            kind: PlotMarkKind::Primitive(mark),
        }
    }

    pub fn from_group(group: MarkGroup<C>) -> Self {
        Self {
            kind: PlotMarkKind::Group(group),
        }
    }

    pub fn from_invalid_argument(message: impl Into<String>) -> Self {
        Self {
            kind: PlotMarkKind::InvalidArgument(message.into()),
        }
    }

    #[doc(hidden)]
    pub fn kind(&self) -> &PlotMarkKind<C> {
        &self.kind
    }
}

/// Internal shape exposed only so the facade crate can flatten plot elements.
#[doc(hidden)]
#[derive(Clone)]
pub enum PlotMarkKind<C: CoordinateSystemCore> {
    Primitive(Arc<dyn Mark<C>>),
    Group(MarkGroup<C>),
    InvalidArgument(String),
}
