//! Explicitly composed native authoring schemas paired with Rust lowerers.

use std::{any::Any, collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use avenger_chart::{layout::LayoutSpec, plot::CompiledPlot, prelude::*};
use avenger_chart_core::{
    CompiledDataTransform, CoordinateSystem, DataContext, DataTransformCompileContext,
    MarkDataMode, SubplotChildPlotSpec,
};
use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, SchemaVersion, ValueShape,
};
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod builtins;

/// Schema-validated value independent of parser AST types.
#[derive(Clone)]
pub enum ResolvedValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Scalar(ScalarValue),
    Expr(Expr),
    Channel(ChannelValue),
    Query(String),
    Array(Vec<ResolvedValue>),
    Object(IndexMap<String, ResolvedValue>),
    DataFrame(Box<DataFrame>),
    Param(Param),
}

impl fmt::Debug for ResolvedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean(value) => f.debug_tuple("Boolean").field(value).finish(),
            Self::Integer(value) => f.debug_tuple("Integer").field(value).finish(),
            Self::Number(value) => f.debug_tuple("Number").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Scalar(value) => f.debug_tuple("Scalar").field(value).finish(),
            Self::Expr(_) => f.write_str("Expr(..)"),
            Self::Channel(value) => f.debug_tuple("Channel").field(value).finish(),
            Self::Query(value) => f.debug_tuple("Query").field(value).finish(),
            Self::Array(value) => f.debug_tuple("Array").field(value).finish(),
            Self::Object(value) => f.debug_tuple("Object").field(value).finish(),
            Self::DataFrame(_) => f.write_str("DataFrame(..)"),
            Self::Param(value) => f.debug_tuple("Param").field(&value.name).finish(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedDeclaration {
    pub kind: String,
    pub properties: IndexMap<String, ResolvedValue>,
}

impl ResolvedDeclaration {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            properties: IndexMap::new(),
        }
    }

    pub fn property(mut self, name: impl Into<String>, value: ResolvedValue) -> Self {
        self.properties.insert(name.into(), value);
        self
    }

    pub fn get(&self, name: &str) -> Result<&ResolvedValue, RegistryError> {
        self.properties
            .get(name)
            .ok_or_else(|| RegistryError::Lowering {
                kind: self.kind.clone(),
                message: format!("missing resolved property '{name}'"),
            })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedChildPlot {
    pub plot: Box<ResolvedPlot>,
    pub placement: ResolvedDeclaration,
}

#[derive(Clone)]
pub struct ResolvedTransformStage {
    pub scope: CoordinationScope,
    pub transform: Box<dyn CompiledDataTransform>,
}

impl fmt::Debug for ResolvedTransformStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedTransformStage")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedMarkGroup {
    pub id: Option<String>,
    pub component_kind: Option<String>,
    pub data: Option<DataFrame>,
    pub transforms: Vec<ResolvedTransformStage>,
    pub marks: Vec<ResolvedMark>,
}

impl ResolvedMarkGroup {
    pub fn new() -> Self {
        Self {
            id: None,
            component_kind: None,
            data: None,
            transforms: Vec::new(),
            marks: Vec::new(),
        }
    }
}

impl Default for ResolvedMarkGroup {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedMark {
    Native(ResolvedDeclaration),
    Group(ResolvedMarkGroup),
}

impl From<ResolvedDeclaration> for ResolvedMark {
    fn from(value: ResolvedDeclaration) -> Self {
        Self::Native(value)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedRootFurnishings {
    pub title: Option<Expr>,
    pub subtitle: Option<Expr>,
    pub layout: Option<LayoutSpec>,
    pub params: Vec<(Param, CoordinationScope)>,
    pub selections: Vec<Selection>,
    pub stores: Vec<Store>,
}

#[derive(Clone, Debug)]
pub struct ResolvedPlot {
    pub coordinate: ResolvedDeclaration,
    pub data: Option<DataFrame>,
    pub marks: Vec<ResolvedMark>,
    pub tools: Vec<ResolvedDeclaration>,
    pub widgets: Vec<ResolvedDeclaration>,
    pub children: Vec<ResolvedChildPlot>,
    pub furnishings: ResolvedRootFurnishings,
}

impl ResolvedPlot {
    pub fn new(coordinate_kind: impl Into<String>) -> Self {
        Self {
            coordinate: ResolvedDeclaration::new(coordinate_kind),
            data: None,
            marks: Vec::new(),
            tools: Vec::new(),
            widgets: Vec::new(),
            children: Vec::new(),
            furnishings: ResolvedRootFurnishings::default(),
        }
    }
}

pub struct LoweredTransform {
    pub transform: Box<dyn CompiledDataTransform>,
    pub outputs: BTreeMap<String, Expr>,
}

pub type NativeTransformLowerer = Arc<
    dyn Fn(
            &ResolvedDeclaration,
            DataTransformCompileContext,
        ) -> Result<LoweredTransform, RegistryError>
        + Send
        + Sync,
>;
pub type NativeWidgetLowerer =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<WidgetAttachment, RegistryError> + Send + Sync>;
pub type NativeObjectLowerer = Arc<
    dyn Fn(&ResolvedDeclaration) -> Result<Box<dyn Any + Send + Sync>, RegistryError> + Send + Sync,
>;

type NativeMarkLowerer<C> =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError> + Send + Sync>;
type NativeToolLowerer<C> =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError> + Send + Sync>;
type CoordinateLowerer<C> =
    Arc<dyn Fn(&ResolvedDeclaration) -> Result<C, RegistryError> + Send + Sync>;
type ChildPlotLowerer<C> = Arc<
    dyn Fn(
            Plot<C>,
            Box<dyn SubplotChildPlotSpec>,
            &ResolvedDeclaration,
        ) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync,
>;
type PlotPropertyLowerer<C> =
    Arc<dyn Fn(Plot<C>, &ResolvedPlot) -> Result<Plot<C>, RegistryError> + Send + Sync>;

struct TypedMarkEntry<C: CoordinateSystem> {
    schema: KindSchema,
    lowerer: NativeMarkLowerer<C>,
}

struct TypedToolEntry<C: CoordinateSystem> {
    schema: KindSchema,
    lowerer: NativeToolLowerer<C>,
}

/// Typed host pack that preserves `Plot<C>` and `PlotMark<C>` until the whole
/// plot crosses an object-safe root/child boundary.
pub struct CoordinatePack<C: CoordinateSystem> {
    kind: String,
    schema: KindSchema,
    coordinate_lowerer: CoordinateLowerer<C>,
    marks: BTreeMap<String, TypedMarkEntry<C>>,
    tools: BTreeMap<String, TypedToolEntry<C>>,
    child_plot_lowerer: Option<ChildPlotLowerer<C>>,
    plot_property_lowerer: Option<PlotPropertyLowerer<C>>,
    duplicate_keys: Vec<NativeKindKey>,
}

impl<C: CoordinateSystem> CoordinatePack<C> {
    pub fn new(
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<C, RegistryError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind: kind.into(),
            schema,
            coordinate_lowerer: Arc::new(lowerer),
            marks: BTreeMap::new(),
            tools: BTreeMap::new(),
            child_plot_lowerer: None,
            plot_property_lowerer: None,
            duplicate_keys: Vec::new(),
        }
    }

    pub fn mark(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .marks
            .insert(
                kind,
                TypedMarkEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(lowerer),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn tool(
        mut self,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let kind = kind.into();
        if self
            .tools
            .insert(
                kind,
                TypedToolEntry {
                    schema: schema.clone(),
                    lowerer: Arc::new(lowerer),
                },
            )
            .is_some()
        {
            self.duplicate_keys.push(schema.key);
        }
        self
    }

    pub fn child_plots(
        mut self,
        lowerer: impl Fn(
            Plot<C>,
            Box<dyn SubplotChildPlotSpec>,
            &ResolvedDeclaration,
        ) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.child_plot_lowerer = Some(Arc::new(lowerer));
        self
    }

    pub fn plot_properties(
        mut self,
        lowerer: impl Fn(Plot<C>, &ResolvedPlot) -> Result<Plot<C>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.plot_property_lowerer = Some(Arc::new(lowerer));
        self
    }
}

#[async_trait]
trait ErasedCoordinatePack: Send + Sync {
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn schemas(&self) -> Vec<KindSchema>;
    fn validate_registration(&self) -> Result<(), RegistryError>;
    fn lower_child(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError>;
    async fn compile_root(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError>;
}

impl<C: CoordinateSystem> CoordinatePack<C> {
    fn lower_mark(
        &self,
        registry: &NativeRegistry,
        resolved: &ResolvedMark,
    ) -> Result<Vec<PlotMark<C>>, RegistryError> {
        match resolved {
            ResolvedMark::Native(declaration) => {
                let entry = self.marks.get(&declaration.kind).ok_or_else(|| {
                    RegistryError::UnknownMarkPair {
                        coordinate: self.kind.clone(),
                        mark: declaration.kind.clone(),
                    }
                })?;
                registry.validate(&entry.schema.key, declaration)?;
                (entry.lowerer)(declaration)
            }
            ResolvedMark::Group(resolved) => {
                let mut data = resolved
                    .data
                    .clone()
                    .map(DataContext::new)
                    .unwrap_or_default();
                for stage in &resolved.transforms {
                    data = data.with_transform_stage(stage.scope, stage.transform.clone());
                }
                let mut group =
                    MarkGroup::<C>::new().with_data_context(data, MarkDataMode::Inherit);
                if let Some(id) = &resolved.id {
                    group = group.id(id.clone());
                }
                if let Some(component_kind) = &resolved.component_kind {
                    group = group.component_kind(component_kind.clone());
                }
                for child in &resolved.marks {
                    for mark in self.lower_mark(registry, child)? {
                        group = group.mark(mark);
                    }
                }
                Ok(vec![PlotMark::from_group(group)])
            }
        }
    }

    fn lower_typed(
        &self,
        registry: &NativeRegistry,
        resolved: &ResolvedPlot,
    ) -> Result<Plot<C>, RegistryError> {
        registry.validate(
            &NativeKindKey::new(NativeKindNamespace::Coordinate, &self.kind),
            &resolved.coordinate,
        )?;
        let coordinate = (self.coordinate_lowerer)(&resolved.coordinate)?;
        let mut plot = Plot::with_coord(coordinate);
        if let Some(data) = &resolved.data {
            plot = plot.data(data.clone());
        }
        for declaration in &resolved.marks {
            for mark in self.lower_mark(registry, declaration)? {
                plot = plot.mark(mark);
            }
        }
        for declaration in &resolved.tools {
            let entry = self.tools.get(&declaration.kind).ok_or_else(|| {
                RegistryError::UnknownToolPair {
                    coordinate: self.kind.clone(),
                    tool: declaration.kind.clone(),
                }
            })?;
            registry.validate(&entry.schema.key, declaration)?;
            plot = plot.tool_arc((entry.lowerer)(declaration)?);
        }
        for declaration in &resolved.widgets {
            plot = plot.widget_attachment(registry.lower_widget(declaration)?);
        }
        for child in &resolved.children {
            let lowerer =
                self.child_plot_lowerer
                    .as_ref()
                    .ok_or_else(|| RegistryError::Lowering {
                        kind: self.kind.clone(),
                        message: "coordinate does not accept child plots".to_string(),
                    })?;
            plot = lowerer(
                plot,
                registry.lower_child_plot(&child.plot)?,
                &child.placement,
            )?;
        }
        if let Some(lowerer) = &self.plot_property_lowerer {
            plot = lowerer(plot, resolved)?;
        }
        Ok(plot)
    }
}

#[async_trait]
impl<C: CoordinateSystem> ErasedCoordinatePack for CoordinatePack<C> {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn schemas(&self) -> Vec<KindSchema> {
        std::iter::once(self.schema.clone())
            .chain(self.marks.values().map(|entry| entry.schema.clone()))
            .chain(self.tools.values().map(|entry| entry.schema.clone()))
            .collect()
    }

    fn validate_registration(&self) -> Result<(), RegistryError> {
        if let Some(key) = self.duplicate_keys.first() {
            return Err(RegistryError::DuplicateSchema(key.clone()));
        }
        let expected_coordinate = NativeKindKey::new(NativeKindNamespace::Coordinate, &self.kind);
        if self.schema.key != expected_coordinate {
            return Err(RegistryError::SchemaLowererMismatch(
                self.schema.key.clone(),
            ));
        }
        for (kind, entry) in &self.marks {
            let expected = NativeKindKey::mark(&self.kind, kind);
            if entry.schema.key != expected {
                return Err(RegistryError::SchemaLowererMismatch(
                    entry.schema.key.clone(),
                ));
            }
        }
        for (kind, entry) in &self.tools {
            if entry.schema.key.namespace != NativeKindNamespace::Tool
                || entry.schema.key.kind != *kind
                || !entry.schema.compatible_coordinates.contains(&self.kind)
            {
                return Err(RegistryError::SchemaLowererMismatch(
                    entry.schema.key.clone(),
                ));
            }
        }
        Ok(())
    }

    fn lower_child(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError> {
        Ok(Box::new(self.lower_typed(registry, plot)?))
    }

    async fn compile_root(
        &self,
        registry: &NativeRegistry,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError> {
        let mut chart = Chart::from_plot(self.lower_typed(registry, plot)?);
        if let Some(title) = &plot.furnishings.title {
            chart = chart.title(title.clone());
        }
        if let Some(subtitle) = &plot.furnishings.subtitle {
            chart = chart.subtitle(subtitle.clone());
        }
        if let Some(layout) = &plot.furnishings.layout {
            chart = chart.layout_spec(layout.clone());
        }
        for (param, sharing) in &plot.furnishings.params {
            chart = chart.param_with_sharing(param.clone(), *sharing);
        }
        for selection in &plot.furnishings.selections {
            chart = chart.selection(selection.clone());
        }
        for store in &plot.furnishings.stores {
            chart = chart.store(store.clone());
        }
        Ok(chart.compile(session_context).await?)
    }
}

struct TransformEntry {
    schema: KindSchema,
    lowerer: NativeTransformLowerer,
}

struct WidgetEntry {
    schema: KindSchema,
    lowerer: NativeWidgetLowerer,
}

struct ObjectEntry {
    schema: KindSchema,
    lowerer: NativeObjectLowerer,
}

pub struct NativeRegistryBuilder {
    language_major: u32,
    profile_label: String,
    coordinates: BTreeMap<String, Box<dyn ErasedCoordinatePack>>,
    transforms: BTreeMap<String, TransformEntry>,
    widgets: BTreeMap<String, WidgetEntry>,
    objects: BTreeMap<NativeKindKey, ObjectEntry>,
}

impl NativeRegistryBuilder {
    pub fn new(language_major: u32, profile_label: impl Into<String>) -> Self {
        Self {
            language_major,
            profile_label: profile_label.into(),
            coordinates: BTreeMap::new(),
            transforms: BTreeMap::new(),
            widgets: BTreeMap::new(),
            objects: BTreeMap::new(),
        }
    }

    pub fn register_coordinate_pack<C: CoordinateSystem>(
        &mut self,
        pack: CoordinatePack<C>,
    ) -> Result<(), RegistryError> {
        let kind = pack.kind.clone();
        if self.coordinates.contains_key(&kind) {
            return Err(RegistryError::DuplicateCoordinate(kind));
        }
        self.coordinates.insert(kind, Box::new(pack));
        Ok(())
    }

    /// Add a mark lowerer to an already registered typed coordinate pack.
    ///
    /// This is the downstream extension path when a host registers built-ins
    /// first and then invokes independent extension registration functions.
    pub fn register_mark<C: CoordinateSystem>(
        &mut self,
        coordinate_kind: &str,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Vec<PlotMark<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RegistryError> {
        let pack = self
            .coordinates
            .get_mut(coordinate_kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
        let pack = pack
            .as_any_mut()
            .downcast_mut::<CoordinatePack<C>>()
            .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                coordinate: coordinate_kind.to_string(),
                expected: std::any::type_name::<C>().to_string(),
            })?;
        let kind = kind.into();
        if pack.marks.contains_key(&kind) {
            return Err(RegistryError::DuplicateSchema(schema.key));
        }
        pack.marks.insert(
            kind,
            TypedMarkEntry {
                schema,
                lowerer: Arc::new(lowerer),
            },
        );
        Ok(())
    }

    /// Add a tool lowerer to an already registered typed coordinate pack.
    pub fn register_tool<C: CoordinateSystem>(
        &mut self,
        coordinate_kind: &str,
        kind: impl Into<String>,
        schema: KindSchema,
        lowerer: impl Fn(&ResolvedDeclaration) -> Result<Arc<dyn ChartTool<C>>, RegistryError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RegistryError> {
        let pack = self
            .coordinates
            .get_mut(coordinate_kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(coordinate_kind.to_string()))?;
        let pack = pack
            .as_any_mut()
            .downcast_mut::<CoordinatePack<C>>()
            .ok_or_else(|| RegistryError::CoordinatePackTypeMismatch {
                coordinate: coordinate_kind.to_string(),
                expected: std::any::type_name::<C>().to_string(),
            })?;
        let kind = kind.into();
        if pack.tools.contains_key(&kind) {
            return Err(RegistryError::DuplicateSchema(schema.key));
        }
        pack.tools.insert(
            kind,
            TypedToolEntry {
                schema,
                lowerer: Arc::new(lowerer),
            },
        );
        Ok(())
    }

    pub fn register_transform(
        &mut self,
        schema: KindSchema,
        lowerer: NativeTransformLowerer,
    ) -> Result<(), RegistryError> {
        let kind = schema.key.kind.clone();
        if schema.key.namespace != NativeKindNamespace::Transform {
            return Err(RegistryError::SchemaLowererMismatch(schema.key));
        }
        if self.transforms.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Transform,
                kind,
            });
        }
        self.transforms
            .insert(kind, TransformEntry { schema, lowerer });
        Ok(())
    }

    pub fn register_widget(
        &mut self,
        schema: KindSchema,
        lowerer: NativeWidgetLowerer,
    ) -> Result<(), RegistryError> {
        let kind = schema.key.kind.clone();
        if schema.key.namespace != NativeKindNamespace::Widget {
            return Err(RegistryError::SchemaLowererMismatch(schema.key));
        }
        if self.widgets.contains_key(&kind) {
            return Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Widget,
                kind,
            });
        }
        self.widgets.insert(kind, WidgetEntry { schema, lowerer });
        Ok(())
    }

    pub fn register_object(
        &mut self,
        schema: KindSchema,
        lowerer: NativeObjectLowerer,
    ) -> Result<(), RegistryError> {
        let key = schema.key.clone();
        if !matches!(
            key.namespace,
            NativeKindNamespace::Scale
                | NativeKindNamespace::Axis
                | NativeKindNamespace::Legend
                | NativeKindNamespace::Layout
        ) {
            return Err(RegistryError::SchemaLowererMismatch(key));
        }
        if self.objects.contains_key(&key) {
            return Err(RegistryError::DuplicateKind {
                namespace: key.namespace,
                kind: key.kind,
            });
        }
        self.objects.insert(key, ObjectEntry { schema, lowerer });
        Ok(())
    }

    pub fn build(self) -> Result<NativeRegistry, RegistryError> {
        let mut entries = BTreeMap::new();
        for pack in self.coordinates.values() {
            pack.validate_registration()?;
            for schema in pack.schemas() {
                let key = schema.key.clone();
                if entries.insert(key.clone(), schema).is_some() {
                    return Err(RegistryError::DuplicateSchema(key));
                }
            }
        }
        for entry in self.transforms.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        for entry in self.widgets.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        for entry in self.objects.values() {
            insert_schema(&mut entries, entry.schema.clone())?;
        }
        let snapshot = NativeSchemaSnapshot {
            version: SchemaVersion {
                major: self.language_major,
                minor: 0,
            },
            profile_label: self.profile_label,
            entries,
        };
        snapshot.validate_docs()?;
        let canonical = snapshot.canonical_json()?;
        let profile_id = NativeRegistryProfileId(format!(
            "avenger-native-{}-{:x}",
            self.language_major,
            Sha256::digest(&canonical)
        ));
        Ok(NativeRegistry {
            snapshot,
            profile_id,
            coordinates: self
                .coordinates
                .into_iter()
                .map(|(kind, pack)| (kind, Arc::from(pack)))
                .collect(),
            transforms: self.transforms,
            widgets: self.widgets,
            objects: self.objects,
        })
    }
}

fn insert_schema(
    entries: &mut BTreeMap<NativeKindKey, KindSchema>,
    schema: KindSchema,
) -> Result<(), RegistryError> {
    let key = schema.key.clone();
    if entries.insert(key.clone(), schema).is_some() {
        return Err(RegistryError::DuplicateSchema(key));
    }
    Ok(())
}

pub struct NativeRegistry {
    snapshot: NativeSchemaSnapshot,
    profile_id: NativeRegistryProfileId,
    coordinates: BTreeMap<String, Arc<dyn ErasedCoordinatePack>>,
    transforms: BTreeMap<String, TransformEntry>,
    widgets: BTreeMap<String, WidgetEntry>,
    objects: BTreeMap<NativeKindKey, ObjectEntry>,
}

impl NativeRegistry {
    pub fn builder() -> NativeRegistryBuilder {
        NativeRegistryBuilder::new(1, "custom")
    }

    pub fn snapshot(&self) -> &NativeSchemaSnapshot {
        &self.snapshot
    }

    pub fn profile_id(&self) -> &NativeRegistryProfileId {
        &self.profile_id
    }

    pub fn canonical_schema_json(&self) -> Result<Vec<u8>, RegistryError> {
        Ok(self.snapshot.canonical_json()?)
    }

    pub async fn compile_root(
        &self,
        plot: &ResolvedPlot,
        session_context: &SessionContext,
    ) -> Result<CompiledPlot, RegistryError> {
        let pack = self
            .coordinates
            .get(&plot.coordinate.kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(plot.coordinate.kind.clone()))?;
        pack.compile_root(self, plot, session_context).await
    }

    pub fn lower_child_plot(
        &self,
        plot: &ResolvedPlot,
    ) -> Result<Box<dyn SubplotChildPlotSpec>, RegistryError> {
        let pack = self
            .coordinates
            .get(&plot.coordinate.kind)
            .ok_or_else(|| RegistryError::UnknownCoordinate(plot.coordinate.kind.clone()))?;
        pack.lower_child(self, plot)
    }

    pub fn lower_transform(
        &self,
        declaration: &ResolvedDeclaration,
        context: DataTransformCompileContext,
    ) -> Result<LoweredTransform, RegistryError> {
        let entry =
            self.transforms
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Transform,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        (entry.lowerer)(declaration, context)
    }

    pub fn lower_widget(
        &self,
        declaration: &ResolvedDeclaration,
    ) -> Result<WidgetAttachment, RegistryError> {
        let entry =
            self.widgets
                .get(&declaration.kind)
                .ok_or_else(|| RegistryError::UnknownKind {
                    namespace: NativeKindNamespace::Widget,
                    kind: declaration.kind.clone(),
                })?;
        self.validate(&entry.schema.key, declaration)?;
        (entry.lowerer)(declaration)
    }

    pub fn lower_object(
        &self,
        key: &NativeKindKey,
        declaration: &ResolvedDeclaration,
    ) -> Result<Box<dyn Any + Send + Sync>, RegistryError> {
        let entry = self
            .objects
            .get(key)
            .ok_or_else(|| RegistryError::UnknownKind {
                namespace: key.namespace,
                kind: key.kind.clone(),
            })?;
        self.validate(key, declaration)?;
        (entry.lowerer)(declaration)
    }

    pub fn validate(
        &self,
        key: &NativeKindKey,
        declaration: &ResolvedDeclaration,
    ) -> Result<(), RegistryError> {
        let schema = self
            .snapshot
            .entries
            .get(key)
            .ok_or_else(|| RegistryError::UnknownKind {
                namespace: key.namespace,
                kind: key.kind.clone(),
            })?;
        if declaration.kind != schema.key.kind {
            return Err(RegistryError::Lowering {
                kind: declaration.kind.clone(),
                message: format!("expected kind '{}'", schema.key.kind),
            });
        }
        for (name, property) in &schema.properties {
            match declaration.properties.get(name) {
                Some(value) => validate_value_shape(value, &property.shape, name)?,
                None if property.required && property.default.is_none() => {
                    return Err(RegistryError::MissingProperty {
                        kind: declaration.kind.clone(),
                        property: name.clone(),
                    });
                }
                None => {}
            }
        }
        for (name, channel) in &schema.channels {
            match declaration.properties.get(name) {
                Some(value) => validate_value_shape(value, &channel.shape, name)?,
                None if channel.required => {
                    return Err(RegistryError::MissingProperty {
                        kind: declaration.kind.clone(),
                        property: name.clone(),
                    });
                }
                None => {}
            }
        }
        for name in declaration.properties.keys() {
            if !schema.properties.contains_key(name) && !schema.channels.contains_key(name) {
                return Err(RegistryError::UnknownProperty {
                    kind: declaration.kind.clone(),
                    property: name.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_value_shape(
    value: &ResolvedValue,
    shape: &ValueShape,
    property: &str,
) -> Result<(), RegistryError> {
    let matches = match (value, shape) {
        (_, ValueShape::Any) => true,
        (ResolvedValue::Boolean(_), ValueShape::Boolean) => true,
        (ResolvedValue::Integer(_), ValueShape::Integer) => true,
        (ResolvedValue::Number(_), ValueShape::Number) => true,
        (ResolvedValue::String(_), ValueShape::String | ValueShape::ScalarBinding) => true,
        (ResolvedValue::Param(_), ValueShape::ScalarBinding) => true,
        (ResolvedValue::String(value), ValueShape::Atom { values }) => {
            values.iter().any(|candidate| candidate.value == *value)
        }
        (
            ResolvedValue::Boolean(_)
            | ResolvedValue::Integer(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::String(_)
            | ResolvedValue::Scalar(_)
            | ResolvedValue::Expr(_)
            | ResolvedValue::Channel(_),
            ValueShape::SqlExpression,
        ) => true,
        (ResolvedValue::Query(_) | ResolvedValue::String(_), ValueShape::SqlQuery) => true,
        (ResolvedValue::DataFrame(_), ValueShape::TableBinding) => true,
        (ResolvedValue::Array(values), ValueShape::Array(inner)) => values
            .iter()
            .all(|value| validate_value_shape(value, inner, property).is_ok()),
        (ResolvedValue::Object(values), ValueShape::Object(fields)) => {
            values.iter().all(|(name, value)| {
                fields
                    .get(name)
                    .is_some_and(|field| validate_value_shape(value, &field.shape, name).is_ok())
            }) && fields.iter().all(|(name, field)| {
                !field.required || field.default.is_some() || values.contains_key(name)
            })
        }
        (
            ResolvedValue::Scalar(_),
            ValueShape::Boolean | ValueShape::Integer | ValueShape::Number | ValueShape::String,
        ) => true,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(RegistryError::InvalidPropertyType {
            property: property.to_string(),
            expected: format!("{shape:?}"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeRegistryProfileId(String);

impl NativeRegistryProfileId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error(transparent)]
    Chart(#[from] AvengerChartError),
    #[error(transparent)]
    Schema(#[from] avenger_chart_schema::SchemaError),
    #[error("duplicate coordinate '{0}'")]
    DuplicateCoordinate(String),
    #[error("duplicate {namespace:?} kind '{kind}'")]
    DuplicateKind {
        namespace: NativeKindNamespace,
        kind: String,
    },
    #[error("duplicate native schema entry {0:?}")]
    DuplicateSchema(NativeKindKey),
    #[error("schema/lowerer namespace mismatch for {0:?}")]
    SchemaLowererMismatch(NativeKindKey),
    #[error("unknown coordinate '{0}'")]
    UnknownCoordinate(String),
    #[error("coordinate pack '{coordinate}' does not use expected Rust type '{expected}'")]
    CoordinatePackTypeMismatch {
        coordinate: String,
        expected: String,
    },
    #[error("unknown mark '{mark}' for coordinate '{coordinate}'")]
    UnknownMarkPair { coordinate: String, mark: String },
    #[error("unknown tool '{tool}' for coordinate '{coordinate}'")]
    UnknownToolPair { coordinate: String, tool: String },
    #[error("unknown {namespace:?} kind '{kind}'")]
    UnknownKind {
        namespace: NativeKindNamespace,
        kind: String,
    },
    #[error("kind '{kind}' is missing required property '{property}'")]
    MissingProperty { kind: String, property: String },
    #[error("kind '{kind}' has unknown property '{property}'")]
    UnknownProperty { kind: String, property: String },
    #[error("property '{property}' expected {expected}")]
    InvalidPropertyType { property: String, expected: String },
    #[error("failed to lower '{kind}': {message}")]
    Lowering { kind: String, message: String },
}

pub(crate) fn expr_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Expr, RegistryError> {
    match declaration.get(name)? {
        ResolvedValue::Expr(expr) => Ok(expr.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "SQL expression".to_string(),
        }),
    }
}

pub(crate) fn string_property(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<String, RegistryError> {
    match declaration.get(name)? {
        ResolvedValue::String(value) | ResolvedValue::Query(value) => Ok(value.clone()),
        _ => Err(RegistryError::InvalidPropertyType {
            property: name.to_string(),
            expected: "string".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use avenger_chart::prelude::{
        Cartesian, CompiledWidget, CoordinationScope, FacetColumn, FacetColumnSubplotChannels,
        IntoPlotMark, PanScrollZoom, Subplot, Symbol, ToolExportTarget,
    };
    use avenger_chart_core::ParamRef;
    use avenger_chart_external_test::{
        external_compound_mark::ExternalMeanPoint,
        external_coord_system::{Cube, Isometric},
        external_mark::HexBin,
    };
    use avenger_chart_schema::ChannelSchema;
    use datafusion::logical_expr::{col, lit};

    use super::*;

    fn object(fields: impl IntoIterator<Item = (&'static str, ResolvedValue)>) -> ResolvedValue {
        ResolvedValue::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        )
    }

    fn symbol() -> ResolvedDeclaration {
        ResolvedDeclaration::new("symbol")
            .property("x", ResolvedValue::Expr(col("x")))
            .property("y", ResolvedValue::Expr(col("y")))
    }

    fn radio_widget() -> ResolvedDeclaration {
        ResolvedDeclaration::new("radio_button_list")
            .property("id", ResolvedValue::String("region".to_string()))
            .property(
                "items",
                ResolvedValue::Array(vec![
                    object([
                        ("value", ResolvedValue::String("north".to_string())),
                        ("label", ResolvedValue::String("North".to_string())),
                    ]),
                    object([
                        ("value", ResolvedValue::String("south".to_string())),
                        ("label", ResolvedValue::String("South".to_string())),
                    ]),
                ]),
            )
            .property("position", ResolvedValue::String("left".to_string()))
    }

    #[test]
    fn bootstrap_schema_is_deterministic_and_profile_is_schema_derived() {
        let left = builtins::bootstrap_registry().unwrap();
        let right = builtins::bootstrap_registry().unwrap();
        assert_eq!(
            left.canonical_schema_json().unwrap(),
            right.canonical_schema_json().unwrap()
        );
        assert_eq!(left.profile_id(), right.profile_id());

        let mut builder = NativeRegistryBuilder::new(1, builtins::BOOTSTRAP_PROFILE_LABEL);
        builtins::register_bootstrap_builtins(&mut builder).unwrap();
        builder
            .register_mark::<Cartesian>(
                "cartesian",
                "external_symbol",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "external_symbol"),
                    "External symbol fixture.",
                ),
                |_| Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks()),
            )
            .unwrap();
        let extended = builder.build().unwrap();
        assert_ne!(left.profile_id(), extended.profile_id());
    }

    #[test]
    fn checked_bootstrap_schema_and_documentation_do_not_drift() {
        let registry = builtins::bootstrap_registry().unwrap();
        if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            fs::write(
                root.join("snapshots/bootstrap-schema.json"),
                serde_json::to_string_pretty(registry.snapshot()).unwrap() + "\n",
            )
            .unwrap();
            fs::write(
                root.join("docs/bootstrap-native-kinds.md"),
                registry.snapshot().markdown_reference(),
            )
            .unwrap();
            return;
        }
        let checked: NativeSchemaSnapshot =
            serde_json::from_str(include_str!("../snapshots/bootstrap-schema.json")).unwrap();
        assert_eq!(registry.snapshot(), &checked);
        assert_eq!(
            registry.snapshot().markdown_reference(),
            include_str!("../docs/bootstrap-native-kinds.md")
        );
    }

    #[tokio::test]
    async fn bootstrap_vertical_slice_compiles_evaluates_and_serializes() {
        let registry = builtins::bootstrap_registry().unwrap();
        let context = SessionContext::new();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.data = Some(context.sql("SELECT 10.0 AS x, 20.0 AS y").await.unwrap());
        plot.marks.push(symbol().into());
        plot.tools.push(ResolvedDeclaration::new("pan_scroll_zoom"));
        plot.widgets.push(radio_widget());
        let compiled = registry.compile_root(&plot, &context).await.unwrap();

        assert_eq!(compiled.marks().len(), 1);
        assert_eq!(compiled.widgets().len(), 1);
        assert_eq!(compiled.tool_behaviors().len(), 2);
        let pan = compiled
            .tool_behaviors()
            .iter()
            .find(|behavior| behavior.component_kind == "pan_scroll_zoom")
            .unwrap();
        assert_eq!(pan.component_kind, "pan_scroll_zoom");
        assert!(pan.exports.iter().any(|export| export.alias == "x_domain"));
        assert!(pan.exports.iter().any(|export| export.alias == "y_domain"));

        let CompiledWidget::Composed(widget) = &compiled.widgets()[0].widget else {
            panic!("bootstrap widget must use the composed widget path")
        };
        assert_eq!(widget.kind, "radio-button-list");
        assert!(widget.behavior_exports.iter().any(|export| {
            export.alias == "value" && matches!(&export.target, ToolExportTarget::Param(_))
        }));

        let bytes = bincode::serialize(&compiled).unwrap();
        let decoded: CompiledPlot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.widgets().len(), 1);
        decoded.evaluate(&context, None).await.unwrap();
    }

    #[test]
    fn schema_validation_precedes_lowering() {
        let called = Arc::new(AtomicBool::new(false));
        let called_by_lowerer = called.clone();
        let mark_schema = KindSchema::new(
            NativeKindKey::mark("cartesian", "strict"),
            "Strict validation fixture.",
        )
        .channel(ChannelSchema {
            name: "x".to_string(),
            required: true,
            shape: ValueShape::SqlExpression,
            docs: "Required x expression.".to_string(),
        });
        let pack = CoordinatePack::new(
            "cartesian",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
                "Cartesian validation fixture.",
            ),
            |_| Ok(Cartesian::new()),
        )
        .mark("strict", mark_schema, move |_| {
            called_by_lowerer.store(true, Ordering::SeqCst);
            Ok(Symbol::<Cartesian>::new().x(0.0).y(0.0).into_plot_marks())
        });
        let mut builder = NativeRegistry::builder();
        builder.register_coordinate_pack(pack).unwrap();
        let registry = builder.build().unwrap();
        let mut plot = ResolvedPlot::new("cartesian");
        plot.marks.push(
            ResolvedDeclaration::new("strict")
                .property("x", ResolvedValue::Object(IndexMap::new()))
                .into(),
        );
        assert!(matches!(
            registry.lower_child_plot(&plot),
            Err(RegistryError::InvalidPropertyType { .. })
        ));
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn duplicate_and_mismatched_registrations_are_rejected() {
        let coordinate_schema = || {
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "cartesian"),
                "Cartesian fixture.",
            )
        };
        let pack =
            || CoordinatePack::new("cartesian", coordinate_schema(), |_| Ok(Cartesian::new()));
        let mut builder = NativeRegistry::builder();
        builder.register_coordinate_pack(pack()).unwrap();
        assert!(matches!(
            builder.register_coordinate_pack(pack()),
            Err(RegistryError::DuplicateCoordinate(_))
        ));
        assert!(matches!(
            builder.register_mark::<FacetColumn>(
                "cartesian",
                "wrong_rust_type",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "wrong_rust_type"),
                    "Wrong typed-pack fixture.",
                ),
                |_| Ok(Vec::new()),
            ),
            Err(RegistryError::CoordinatePackTypeMismatch { .. })
        ));
        assert!(matches!(
            builder.register_mark::<Cartesian>(
                "missing",
                "dot",
                KindSchema::new(
                    NativeKindKey::mark("missing", "dot"),
                    "Missing coordinate fixture.",
                ),
                |_| Ok(Vec::new()),
            ),
            Err(RegistryError::UnknownCoordinate(kind)) if kind == "missing"
        ));

        let duplicate_mark = pack()
            .mark(
                "same",
                KindSchema::new(NativeKindKey::mark("cartesian", "same"), "First mark."),
                |_| Ok(Symbol::<Cartesian>::new().into_plot_marks()),
            )
            .mark(
                "same",
                KindSchema::new(NativeKindKey::mark("cartesian", "same"), "Second mark."),
                |_| Ok(Symbol::<Cartesian>::new().into_plot_marks()),
            );
        let mut builder = NativeRegistry::builder();
        builder.register_coordinate_pack(duplicate_mark).unwrap();
        assert!(matches!(
            builder.build(),
            Err(RegistryError::DuplicateSchema(_))
        ));

        let tool_schema = || {
            let mut schema = KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Tool, "same_tool"),
                "Duplicate tool fixture.",
            );
            schema
                .compatible_coordinates
                .insert("cartesian".to_string());
            schema
        };
        let duplicate_tool = pack()
            .tool("same_tool", tool_schema(), |_| {
                Ok(Arc::new(PanScrollZoom::cartesian()))
            })
            .tool("same_tool", tool_schema(), |_| {
                Ok(Arc::new(PanScrollZoom::cartesian()))
            });
        let mut builder = NativeRegistry::builder();
        builder.register_coordinate_pack(duplicate_tool).unwrap();
        assert!(matches!(
            builder.build(),
            Err(RegistryError::DuplicateSchema(_))
        ));

        let transform_schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Transform, "same_transform"),
            "Duplicate transform fixture.",
        );
        let transform_lowerer: NativeTransformLowerer = Arc::new(|_, _| unreachable!());
        let mut builder = NativeRegistry::builder();
        builder
            .register_transform(transform_schema.clone(), transform_lowerer.clone())
            .unwrap();
        assert!(matches!(
            builder.register_transform(transform_schema, transform_lowerer),
            Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Transform,
                ..
            })
        ));

        let widget_schema = KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Widget, "same_widget"),
            "Duplicate widget fixture.",
        );
        let widget_lowerer: NativeWidgetLowerer = Arc::new(|_| unreachable!());
        let mut builder = NativeRegistry::builder();
        builder
            .register_widget(widget_schema.clone(), widget_lowerer.clone())
            .unwrap();
        assert!(matches!(
            builder.register_widget(widget_schema, widget_lowerer),
            Err(RegistryError::DuplicateKind {
                namespace: NativeKindNamespace::Widget,
                ..
            })
        ));

        let mismatch = CoordinatePack::new(
            "cartesian",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "polar"),
                "Mismatched coordinate fixture.",
            ),
            |_| Ok(Cartesian::new()),
        );
        let mut builder = NativeRegistry::builder();
        builder.register_coordinate_pack(mismatch).unwrap();
        assert!(matches!(
            builder.build(),
            Err(RegistryError::SchemaLowererMismatch(_))
        ));
    }

    #[tokio::test]
    async fn bootstrap_symbol_channel_inventory_tracks_compiled_symbol() {
        let registry = builtins::bootstrap_registry().unwrap();
        let schema = registry
            .snapshot()
            .entries
            .get(&NativeKindKey::mark("cartesian", "symbol"))
            .unwrap();
        let context = SessionContext::new();
        let compiled = registry
            .compile_root(
                &ResolvedPlot {
                    coordinate: ResolvedDeclaration::new("cartesian"),
                    data: None,
                    marks: vec![symbol().into()],
                    tools: Vec::new(),
                    widgets: Vec::new(),
                    children: Vec::new(),
                    furnishings: ResolvedRootFurnishings::default(),
                },
                &context,
            )
            .await
            .unwrap();
        let rust_channels: std::collections::BTreeSet<_> = compiled.marks()[0]
            .supported_channels()
            .into_iter()
            .map(|channel| channel.name.to_string())
            .collect();
        assert_eq!(
            schema
                .channels
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            rust_channels
        );
    }

    #[test]
    fn transforms_lower_with_output_handles() {
        let registry = builtins::bootstrap_registry().unwrap();
        let context = DataTransformCompileContext::new(CoordinationScope::Shared);
        registry
            .lower_transform(
                &ResolvedDeclaration::new("filter")
                    .property("predicate", ResolvedValue::Expr(col("x").gt(lit(0)))),
                context,
            )
            .unwrap();
        registry
            .lower_transform(
                &ResolvedDeclaration::new("sql").property(
                    "query",
                    ResolvedValue::Query("SELECT * FROM input".to_string()),
                ),
                context,
            )
            .unwrap();
        let aggregate = registry
            .lower_transform(
                &ResolvedDeclaration::new("aggregate").property(
                    "measures",
                    ResolvedValue::Array(vec![object([
                        ("name", ResolvedValue::String("total".to_string())),
                        ("op", ResolvedValue::String("sum".to_string())),
                        ("expr", ResolvedValue::Expr(col("x"))),
                    ])]),
                ),
                context,
            )
            .unwrap();
        assert_eq!(aggregate.outputs.keys().collect::<Vec<_>>(), vec!["total"]);
    }

    #[test]
    fn bootstrap_widget_schema_records_authoring_runtime_mapping() {
        let registry = builtins::bootstrap_registry().unwrap();
        let schema = registry
            .snapshot()
            .entries
            .get(&NativeKindKey::new(
                NativeKindNamespace::Widget,
                "radio_button_list",
            ))
            .unwrap();
        assert_eq!(schema.runtime_kind.as_deref(), Some("radio-button-list"));
        assert_eq!(
            schema.parts["focus_ring"].runtime_alias.as_deref(),
            Some("focus-ring")
        );
        assert_eq!(schema.exports["value"].value_kind, "param<item_scalar>");
    }

    #[tokio::test]
    async fn generated_and_supplied_widget_value_share_the_same_resolved_slot() {
        fn exported_value(plot: &CompiledPlot) -> ParamRef {
            let CompiledWidget::Composed(widget) = &plot.widgets()[0].widget else {
                panic!("expected composed widget")
            };
            widget
                .behavior_exports
                .iter()
                .find_map(|export| match (&*export.alias, &export.target) {
                    ("value", ToolExportTarget::Param(id)) => Some(id.clone()),
                    _ => None,
                })
                .expect("radio value export")
        }

        let registry = builtins::bootstrap_registry().unwrap();
        let context = SessionContext::new();
        let mut generated = ResolvedPlot::new("cartesian");
        generated.widgets.push(radio_widget());
        let generated = registry.compile_root(&generated, &context).await.unwrap();

        let mut supplied_declaration = radio_widget();
        supplied_declaration.properties.insert(
            "value_param".to_string(),
            ResolvedValue::Param(Param::new("external_value", "north")),
        );
        let mut supplied = ResolvedPlot::new("cartesian");
        supplied.widgets.push(supplied_declaration);
        let supplied = registry.compile_root(&supplied, &context).await.unwrap();

        assert_eq!(exported_value(&generated), exported_value(&supplied));
    }

    #[tokio::test]
    async fn downstream_extensions_cross_coordinate_and_typetag_boundaries() {
        fn register_extension(builder: &mut NativeRegistryBuilder) -> Result<(), RegistryError> {
            builder.register_mark::<Cartesian>(
                "cartesian",
                "external_hexbin",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "external_hexbin"),
                    "A Cartesian mark implemented by a downstream crate.",
                ),
                |_| Ok(HexBin::<Cartesian>::new().x(1.0).y(1.0).into_plot_marks()),
            )?;
            builder.register_mark::<Cartesian>(
                "cartesian",
                "external_mean_point",
                KindSchema::new(
                    NativeKindKey::mark("cartesian", "external_mean_point"),
                    "A downstream aggregate-backed compound mark.",
                ),
                |_| Ok(ExternalMeanPoint::new(lit("all"), lit(2.0)).into_plot_marks()),
            )
        }

        let external_coordinate = CoordinatePack::new(
            "external_isometric",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
                "A downstream isometric coordinate pack fixture.",
            ),
            |_| Ok(Isometric::new()),
        )
        .mark(
            "external_cube",
            KindSchema::new(
                NativeKindKey::mark("external_isometric", "external_cube"),
                "A cube implemented by a downstream crate.",
            ),
            |_| {
                Ok(Cube::<Isometric>::new()
                    .iso_x(1.0)
                    .iso_y(2.0)
                    .iso_z(3.0)
                    .into_plot_marks())
            },
        );

        let external_container = CoordinatePack::new(
            "external_facet_column",
            KindSchema::new(
                NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
                "A downstream mixed-coordinate child container.",
            ),
            |_| Ok(FacetColumn),
        )
        .child_plots(|plot, child, placement| {
            let column = match placement.get("column")? {
                ResolvedValue::Expr(expr) => expr.clone(),
                _ => {
                    return Err(RegistryError::InvalidPropertyType {
                        property: "column".to_string(),
                        expected: "SQL expression".to_string(),
                    });
                }
            };
            Ok(plot.mark(Subplot::<FacetColumn>::new(child).column(column)))
        });

        let mut builder = NativeRegistryBuilder::new(1, "downstream-fixture");
        builtins::register_bootstrap_builtins(&mut builder).unwrap();
        register_extension(&mut builder).unwrap();
        builder
            .register_coordinate_pack(external_coordinate)
            .unwrap();
        builder
            .register_coordinate_pack(external_container)
            .unwrap();
        let registry = builder.build().unwrap();
        let context = SessionContext::new();

        let mut augmented = ResolvedPlot::new("cartesian");
        augmented
            .marks
            .push(ResolvedDeclaration::new("external_hexbin").into());
        augmented
            .marks
            .push(ResolvedDeclaration::new("external_mean_point").into());
        let compiled = registry.compile_root(&augmented, &context).await.unwrap();
        assert_eq!(compiled.marks().len(), 2);
        assert_eq!(compiled.marks()[0].mark_type(), "hexbin");
        let decoded: CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        assert_eq!(decoded.marks()[0].mark_type(), "hexbin");

        let mut external = ResolvedPlot::new("external_isometric");
        external
            .marks
            .push(ResolvedDeclaration::new("external_cube").into());
        let compiled = registry.compile_root(&external, &context).await.unwrap();
        assert_eq!(compiled.marks().len(), 1);
        assert_eq!(compiled.marks()[0].mark_type(), "cube");
        let decoded: CompiledPlot =
            bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap();
        assert_eq!(decoded.marks()[0].mark_type(), "cube");

        let mut built_in_child = ResolvedPlot::new("cartesian");
        built_in_child
            .marks
            .push(ResolvedDeclaration::new("external_hexbin").into());
        let mut parent = ResolvedPlot::new("external_facet_column");
        parent.children.push(ResolvedChildPlot {
            plot: Box::new(built_in_child),
            placement: ResolvedDeclaration::new("child")
                .property("column", ResolvedValue::Expr(lit("built_in"))),
        });
        parent.children.push(ResolvedChildPlot {
            plot: Box::new(external),
            placement: ResolvedDeclaration::new("child")
                .property("column", ResolvedValue::Expr(lit("custom"))),
        });
        let compiled = registry.compile_root(&parent, &context).await.unwrap();
        let bytes = bincode::serialize(&compiled).unwrap();
        let decoded: CompiledPlot = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.marks().len(), 2);
    }
}
