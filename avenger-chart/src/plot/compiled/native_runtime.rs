use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
    fmt,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use avenger_chart_core::{
    AvengerChartError, CompiledNativeWidgetSpec, ResolvedWidgetAxisSize, ResolvedWidgetPartStyle,
    ResolvedWidgetStyleSet, WidgetPartManifest, WidgetPresentationState, WidgetStyleProperty,
    WidgetStyleValueType,
};
use avenger_common::{cursor::CursorStyle, time::Instant};
use avenger_eventstream::runtime::{
    LogicalRect, RuntimeHostCommand, RuntimeWakeEvent, RuntimeWakeKey,
};
use avenger_eventstream::scene::SceneGraphEvent;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use super::ScopedParamAssignment;

static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWidgetDocumentId(u64);

impl NativeWidgetDocumentId {
    pub fn new() -> Self {
        let id = NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed);
        assert!(id != u64::MAX, "native widget document id space exhausted");
        Self(id)
    }
}

impl Default for NativeWidgetDocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NativeWidgetDocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeWidgetDocumentId(..)")
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NativeWidgetPlotId(Arc<str>);

impl NativeWidgetPlotId {
    pub fn from_member_path(member_path: impl Into<Arc<str>>) -> Self {
        let member_path = member_path.into();
        assert!(
            !member_path.is_empty(),
            "native widget plot member path must not be empty"
        );
        Self(member_path)
    }

    pub fn chart_root() -> Self {
        Self::from_member_path("chart-root")
    }

    pub fn member_path(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NativeWidgetPlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NativeWidgetPlotId").field(&self.0).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetNamespace {
    document_id: NativeWidgetDocumentId,
    plot_id: NativeWidgetPlotId,
}

impl NativeWidgetNamespace {
    pub fn new(document_id: NativeWidgetDocumentId, plot_id: NativeWidgetPlotId) -> Self {
        Self {
            document_id,
            plot_id,
        }
    }

    pub fn ephemeral() -> Self {
        Self::new(
            NativeWidgetDocumentId::new(),
            NativeWidgetPlotId::chart_root(),
        )
    }

    pub fn document_id(&self) -> NativeWidgetDocumentId {
        self.document_id
    }

    pub fn plot_id(&self) -> &NativeWidgetPlotId {
        &self.plot_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetInstanceKey {
    namespace: NativeWidgetNamespace,
    widget_id: Arc<str>,
}

impl NativeWidgetInstanceKey {
    pub fn new(namespace: NativeWidgetNamespace, widget_id: impl Into<Arc<str>>) -> Self {
        let widget_id = widget_id.into();
        assert!(!widget_id.is_empty(), "native widget id must not be empty");
        Self {
            namespace,
            widget_id,
        }
    }

    pub fn namespace(&self) -> &NativeWidgetNamespace {
        &self.namespace
    }

    pub fn widget_id(&self) -> &str {
        &self.widget_id
    }

    fn wake_namespace(&self) -> String {
        format!(
            "native-widget:{}:{}:{}",
            self.namespace.document_id.0,
            self.namespace.plot_id.member_path(),
            self.widget_id
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetAttachmentEpoch(u64);

impl NativeWidgetAttachmentEpoch {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Default)]
struct NativeWidgetInstanceSlotState {
    next_epoch: u64,
    active_epoch: Option<NativeWidgetAttachmentEpoch>,
    instance: Option<Arc<dyn Any + Send + Sync>>,
}

pub struct NativeWidgetInstanceSlot {
    key: NativeWidgetInstanceKey,
    state: Mutex<NativeWidgetInstanceSlotState>,
}

impl fmt::Debug for NativeWidgetInstanceSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWidgetInstanceSlot")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl NativeWidgetInstanceSlot {
    fn new(key: NativeWidgetInstanceKey) -> Self {
        Self {
            key,
            state: Mutex::new(NativeWidgetInstanceSlotState::default()),
        }
    }

    pub fn key(&self) -> &NativeWidgetInstanceKey {
        &self.key
    }

    pub fn attach(&self) -> NativeWidgetAttachmentEpoch {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        state.next_epoch = state
            .next_epoch
            .checked_add(1)
            .expect("native widget attachment epoch exhausted");
        let epoch = NativeWidgetAttachmentEpoch(state.next_epoch);
        state.active_epoch = Some(epoch);
        epoch
    }

    pub fn detach(&self, epoch: NativeWidgetAttachmentEpoch) -> bool {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if state.active_epoch == Some(epoch) {
            state.active_epoch = None;
            true
        } else {
            false
        }
    }

    pub fn is_active(&self, epoch: NativeWidgetAttachmentEpoch) -> bool {
        self.state
            .lock()
            .expect("native widget slot lock poisoned")
            .active_epoch
            == Some(epoch)
    }

    fn take_instance<T>(&self) -> Result<Option<Arc<T>>, AvengerChartError>
    where
        T: Any + Send + Sync,
    {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if state.active_epoch.is_some() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Cannot evict active native widget '{}'",
                self.key.widget_id()
            )));
        }
        state
            .instance
            .take()
            .map(|instance| {
                instance.downcast::<T>().map_err(|_| {
                    AvengerChartError::InvalidArgument(
                        NativeWidgetSlotTypeMismatch {
                            widget_id: self.key.widget_id().to_string(),
                        }
                        .to_string(),
                    )
                })
            })
            .transpose()
    }

    pub fn get_or_init<T>(
        &self,
        init: impl FnOnce() -> T,
    ) -> Result<Arc<T>, NativeWidgetSlotTypeMismatch>
    where
        T: Any + Send + Sync,
    {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if let Some(instance) = &state.instance {
            return instance
                .clone()
                .downcast::<T>()
                .map_err(|_| NativeWidgetSlotTypeMismatch {
                    widget_id: self.key.widget_id().to_string(),
                });
        }
        let instance = Arc::new(init());
        state.instance = Some(instance.clone());
        Ok(instance)
    }

    pub fn get_or_try_init<T, E>(
        &self,
        init: impl FnOnce() -> Result<T, E>,
    ) -> Result<Arc<T>, NativeWidgetSlotInitError<E>>
    where
        T: Any + Send + Sync,
    {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if let Some(instance) = &state.instance {
            return instance.clone().downcast::<T>().map_err(|_| {
                NativeWidgetSlotInitError::TypeMismatch(NativeWidgetSlotTypeMismatch {
                    widget_id: self.key.widget_id().to_string(),
                })
            });
        }
        let instance = Arc::new(init().map_err(NativeWidgetSlotInitError::Initialization)?);
        state.instance = Some(instance.clone());
        Ok(instance)
    }
}

#[derive(Debug)]
pub enum NativeWidgetSlotInitError<E> {
    TypeMismatch(NativeWidgetSlotTypeMismatch),
    Initialization(E),
}

impl<E: fmt::Display> fmt::Display for NativeWidgetSlotInitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeMismatch(error) => error.fmt(f),
            Self::Initialization(error) => error.fmt(f),
        }
    }
}

impl<E> std::error::Error for NativeWidgetSlotInitError<E> where E: std::error::Error + 'static {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeWidgetSlotTypeMismatch {
    pub widget_id: String,
}

impl fmt::Display for NativeWidgetSlotTypeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "native widget instance slot '{}' was initialized with another runtime type",
            self.widget_id
        )
    }
}

impl std::error::Error for NativeWidgetSlotTypeMismatch {}

pub trait NativeWidgetInstanceStore: Send + Sync {
    fn slot(&self, key: NativeWidgetInstanceKey) -> Arc<NativeWidgetInstanceSlot>;
    fn slots_for_namespace(
        &self,
        namespace: &NativeWidgetNamespace,
    ) -> Vec<Arc<NativeWidgetInstanceSlot>>;
    fn host_services(&self, namespace: &NativeWidgetNamespace) -> NativeWidgetHostServices;
    fn set_host_services(
        &self,
        namespace: NativeWidgetNamespace,
        services: NativeWidgetHostServices,
    );
    fn remove_namespace(&self, namespace: &NativeWidgetNamespace);
}

#[derive(Default)]
pub struct InMemoryNativeWidgetInstanceStore {
    slots: Mutex<HashMap<NativeWidgetInstanceKey, Arc<NativeWidgetInstanceSlot>>>,
    services: Mutex<HashMap<NativeWidgetNamespace, NativeWidgetHostServices>>,
}

impl InMemoryNativeWidgetInstanceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn slot_count(&self) -> usize {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .len()
    }
}

impl NativeWidgetInstanceStore for InMemoryNativeWidgetInstanceStore {
    fn slot(&self, key: NativeWidgetInstanceKey) -> Arc<NativeWidgetInstanceSlot> {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .entry(key.clone())
            .or_insert_with(|| Arc::new(NativeWidgetInstanceSlot::new(key)))
            .clone()
    }

    fn remove_namespace(&self, namespace: &NativeWidgetNamespace) {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .retain(|key, _| key.namespace() != namespace);
        self.services
            .lock()
            .expect("native widget host-services store lock poisoned")
            .remove(namespace);
    }

    fn slots_for_namespace(
        &self,
        namespace: &NativeWidgetNamespace,
    ) -> Vec<Arc<NativeWidgetInstanceSlot>> {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .iter()
            .filter(|(key, _)| key.namespace() == namespace)
            .map(|(_, slot)| slot.clone())
            .collect()
    }

    fn host_services(&self, namespace: &NativeWidgetNamespace) -> NativeWidgetHostServices {
        self.services
            .lock()
            .expect("native widget host-services store lock poisoned")
            .entry(namespace.clone())
            .or_default()
            .clone()
    }

    fn set_host_services(
        &self,
        namespace: NativeWidgetNamespace,
        services: NativeWidgetHostServices,
    ) {
        self.services
            .lock()
            .expect("native widget host-services store lock poisoned")
            .insert(namespace, services);
    }
}

/// Intrinsic size returned by a registry-backed native-widget factory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeWidgetMeasurement {
    pub width: ResolvedWidgetAxisSize,
    pub height: ResolvedWidgetAxisSize,
}

impl NativeWidgetMeasurement {
    pub const fn fixed(width: f32, height: f32) -> Self {
        Self {
            width: ResolvedWidgetAxisSize {
                min_px: width,
                preferred_px: width,
                stretch: 0.0,
            },
            height: ResolvedWidgetAxisSize {
                min_px: height,
                preferred_px: height,
                stretch: 0.0,
            },
        }
    }

    fn validate(self, widget_id: &str) -> Result<Self, AvengerChartError> {
        for (axis, size) in [("width", self.width), ("height", self.height)] {
            if !size.min_px.is_finite()
                || !size.preferred_px.is_finite()
                || !size.stretch.is_finite()
                || size.min_px < 0.0
                || size.preferred_px < size.min_px
                || size.stretch < 0.0
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Native widget '{widget_id}' returned invalid {axis} measurement {size:?}"
                )));
            }
        }
        Ok(self)
    }
}

/// Read-only inputs shared by registry measurement and instance construction.
pub struct NativeWidgetFactoryContext<'a> {
    pub params: &'a IndexMap<String, ScalarValue>,
    pub styles: &'a Arc<ResolvedWidgetStyleSet>,
    pub base_font_size: f32,
}

/// One realized native-widget environment. Frame position is deliberately
/// absent: x/y-only movement is compositor state and does not dirty a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeWidgetEnvironment {
    pub frame_size: [f32; 2],
    pub(crate) params: Arc<IndexMap<String, ScalarValue>>,
    pub styles: Arc<ResolvedWidgetStyleSet>,
    pub presentation: WidgetPresentationState,
    pub text_measurement_fingerprint: u64,
    pub registry_measurement_revision: u64,
    pub data_revision: u64,
    digest: u64,
    geometry_digest: u64,
}

impl NativeWidgetEnvironment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        frame_size: [f32; 2],
        params: Arc<IndexMap<String, ScalarValue>>,
        styles: Arc<ResolvedWidgetStyleSet>,
        presentation: WidgetPresentationState,
        text_measurement_fingerprint: u64,
        registry_measurement_revision: u64,
        data_revision: u64,
    ) -> Result<Self, AvengerChartError> {
        if frame_size
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Native widget frame size must be finite and nonnegative, got {frame_size:?}"
            )));
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut geometry_hasher = std::collections::hash_map::DefaultHasher::new();
        frame_size[0].to_bits().hash(&mut hasher);
        frame_size[1].to_bits().hash(&mut hasher);
        frame_size[0].to_bits().hash(&mut geometry_hasher);
        frame_size[1].to_bits().hash(&mut geometry_hasher);
        styles.digest.hash(&mut hasher);
        styles.geometry_digest.hash(&mut geometry_hasher);
        format!("{presentation:?}").hash(&mut hasher);
        text_measurement_fingerprint.hash(&mut hasher);
        registry_measurement_revision.hash(&mut hasher);
        data_revision.hash(&mut hasher);
        text_measurement_fingerprint.hash(&mut geometry_hasher);
        registry_measurement_revision.hash(&mut geometry_hasher);
        data_revision.hash(&mut geometry_hasher);
        let digest = hasher.finish();
        let geometry_digest = geometry_hasher.finish();
        Ok(Self {
            frame_size,
            params,
            styles,
            presentation,
            text_measurement_fingerprint,
            registry_measurement_revision,
            data_revision,
            digest,
            geometry_digest,
        })
    }

    pub const fn digest(&self) -> u64 {
        self.digest
    }

    pub const fn geometry_digest(&self) -> u64 {
        self.geometry_digest
    }
}

/// Host-neutral event delivered to one native widget after ownership and
/// coordinate localization have been resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeWidgetEvent {
    pub event: SceneGraphEvent,
    pub hit_part: Option<String>,
    pub current: Option<[f32; 2]>,
    pub start: Option<[f32; 2]>,
    pub previous: Option<[f32; 2]>,
    pub wheel_delta: Option<[f32; 2]>,
    pub frame_size: [f32; 2],
}

/// Ordered native-widget scene parts. Part ids are structural names, not
/// compile-time chart event targets.
#[derive(Clone, Default)]
pub struct NativeWidgetScene {
    pub parts: IndexMap<String, SceneMark>,
}

impl NativeWidgetScene {
    pub fn try_from_iter(
        parts: impl IntoIterator<Item = (String, SceneMark)>,
    ) -> Result<Self, AvengerChartError> {
        let mut resolved = IndexMap::new();
        for (part, mark) in parts {
            avenger_chart_core::validate_structural_id("native widget part", &part)?;
            if resolved.insert(part.clone(), mark).is_some() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Duplicate native widget scene part '{part}'"
                )));
            }
        }
        Ok(Self { parts: resolved })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeWidgetEvaluationIntent {
    #[default]
    None,
    Preview,
    Exact,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NativeWidgetFocusRequest {
    Focus {
        caret: Option<LogicalRect>,
        clipboard_payload: String,
        cancel_composition: bool,
    },
    Blur,
}

/// Mutations and host work requested by one native callback. The instance
/// reports intent; the chart/dashboard/site adapter applies it transactionally.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeWidgetDispatchOutcome {
    pub param_assignments: Vec<ScopedParamAssignment>,
    pub evaluation_intent: NativeWidgetEvaluationIntent,
    pub scene_dirty: bool,
    pub index_dirty: bool,
    pub focus: Option<NativeWidgetFocusRequest>,
    pub cursor: Option<CursorStyle>,
    pub commands: Vec<RuntimeHostCommand>,
    pub consume: bool,
}

/// One evaluation's authoritative native-widget parameter values and their
/// monotonically increasing document revisions.
#[derive(Clone, Copy, Debug)]
pub struct NativeWidgetStateSnapshot<'a> {
    values: &'a IndexMap<String, ScalarValue>,
    revisions: &'a IndexMap<String, u64>,
}

impl<'a> NativeWidgetStateSnapshot<'a> {
    fn new(
        values: &'a IndexMap<String, ScalarValue>,
        revisions: &'a IndexMap<String, u64>,
    ) -> Self {
        Self { values, revisions }
    }

    /// Read an authoritative parameter value.
    pub fn get(&self, name: &str) -> Option<&'a ScalarValue> {
        self.values.get(name)
    }

    /// Read the document revision paired with a parameter value.
    pub fn revision(&self, name: &str) -> u64 {
        self.revisions.get(name).copied().unwrap_or(0)
    }

    /// Iterate over authoritative parameter values in declaration order.
    pub fn values(&self) -> &'a IndexMap<String, ScalarValue> {
        self.values
    }
}

/// Stateful live side of a native widget. Implementations may retain editor,
/// gesture, and undo state, but document state remains in registered params.
pub trait NativeWidgetInstance: Send {
    fn on_state_sync(
        &mut self,
        _state: NativeWidgetStateSnapshot<'_>,
        _ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn on_environment_sync(
        &mut self,
        _environment: &NativeWidgetEnvironment,
        _ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn on_event(
        &mut self,
        _event: &NativeWidgetEvent,
        _ctx: &mut NativeWidgetCtx,
    ) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn scene(
        &mut self,
        environment: &NativeWidgetEnvironment,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetScene, AvengerChartError>;

    fn on_deactivate(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn on_session_detach(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        Ok(())
    }

    fn on_unmount(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        Ok(())
    }
}

/// Registered live implementation for one serialized native widget kind.
pub trait NativeWidgetFactory: Send + Sync {
    fn kind(&self) -> &'static str;

    fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32>;

    fn part_manifests(
        &self,
        _spec: &CompiledNativeWidgetSpec,
        _payload: &serde_json::Value,
    ) -> Result<Vec<WidgetPartManifest>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn measure(
        &self,
        spec: &CompiledNativeWidgetSpec,
        _payload: &serde_json::Value,
        _ctx: &NativeWidgetFactoryContext<'_>,
    ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
        Err(AvengerChartError::InvalidArgument(format!(
            "Native widget '{}' (kind '{}') requested registry measurement, but its factory does not implement measure()",
            spec.id, spec.kind
        )))
    }

    fn create(
        &self,
        spec: &CompiledNativeWidgetSpec,
        payload: &serde_json::Value,
    ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError>;
}

#[derive(Clone)]
pub struct ResolvedNativeWidgetSpec {
    pub factory: Arc<dyn NativeWidgetFactory>,
    pub payload: Arc<serde_json::Value>,
}

#[derive(Default)]
pub struct NativeWidgetRegistry {
    factories: BTreeMap<String, Arc<dyn NativeWidgetFactory>>,
}

impl NativeWidgetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, factory: F) -> Result<(), AvengerChartError>
    where
        F: NativeWidgetFactory + 'static,
    {
        self.register_arc(Arc::new(factory))
    }

    pub fn register_arc(
        &mut self,
        factory: Arc<dyn NativeWidgetFactory>,
    ) -> Result<(), AvengerChartError> {
        let kind = factory.kind();
        avenger_chart_core::validate_structural_id("native widget kind", kind)?;
        if self.factories.contains_key(kind) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate native widget factory kind '{kind}'"
            )));
        }
        self.factories.insert(kind.to_string(), factory);
        Ok(())
    }

    pub fn with_factory<F>(mut self, factory: F) -> Result<Self, AvengerChartError>
    where
        F: NativeWidgetFactory + 'static,
    {
        self.register(factory)?;
        Ok(self)
    }

    pub fn factory(&self, kind: &str) -> Option<Arc<dyn NativeWidgetFactory>> {
        self.factories.get(kind).cloned()
    }

    pub fn resolve(
        &self,
        spec: &CompiledNativeWidgetSpec,
    ) -> Result<ResolvedNativeWidgetSpec, AvengerChartError> {
        let factory =
            self.factory(&spec.kind)
                .ok_or_else(|| AvengerChartError::UnknownNativeWidgetKind {
                    widget_id: spec.id.clone(),
                    kind: spec.kind.clone(),
                })?;
        if !factory
            .supported_schema_versions()
            .contains(&spec.schema_version)
        {
            return Err(AvengerChartError::UnsupportedNativeWidgetSchemaVersion {
                widget_id: spec.id.clone(),
                kind: spec.kind.clone(),
                schema_version: spec.schema_version,
            });
        }
        let payload = Arc::new(spec.payload.parse_for(&spec.id, &spec.kind)?);
        Ok(ResolvedNativeWidgetSpec { factory, payload })
    }

    pub fn measure(
        &self,
        spec: &CompiledNativeWidgetSpec,
        ctx: &NativeWidgetFactoryContext<'_>,
    ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
        let resolved = self.resolve(spec)?;
        resolved
            .factory
            .measure(spec, resolved.payload.as_ref(), ctx)?
            .validate(&spec.id)
    }

    pub fn create_instance(
        &self,
        spec: &CompiledNativeWidgetSpec,
    ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
        let resolved = self.resolve(spec)?;
        resolved.factory.create(spec, resolved.payload.as_ref())
    }

    pub fn kinds(&self) -> impl ExactSizeIterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}

impl fmt::Debug for NativeWidgetRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWidgetRegistry")
            .field("kinds", &self.factories.keys().collect::<Vec<_>>())
            .finish()
    }
}

struct NativeWidgetLiveState {
    instance: Box<dyn NativeWidgetInstance>,
    state_fingerprint: Option<Vec<(String, u64, String)>>,
    environment_digest: Option<u64>,
    environment: Option<NativeWidgetEnvironment>,
    base_font_size: f32,
    scene: Option<NativeWidgetScene>,
    part_order: Vec<String>,
    part_kinds: HashMap<String, String>,
    pending_scene_dirty: bool,
    pending_index_dirty: bool,
    scene_rebuilds: u64,
    index_rebuilds: u64,
    detached: bool,
    unmounted: bool,
}

/// One factory-created instance retained by the namespaced instance store.
pub struct NativeWidgetInstanceHandle {
    spec_kind: String,
    schema_version: u32,
    canonical_payload: String,
    part_manifests: IndexMap<String, WidgetPartManifest>,
    state: Mutex<NativeWidgetLiveState>,
}

impl fmt::Debug for NativeWidgetInstanceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        f.debug_struct("NativeWidgetInstanceHandle")
            .field("kind", &self.spec_kind)
            .field("schema_version", &self.schema_version)
            .field("scene_rebuilds", &state.scene_rebuilds)
            .field("index_rebuilds", &state.index_rebuilds)
            .finish_non_exhaustive()
    }
}

impl NativeWidgetInstanceHandle {
    fn create(
        registry: &NativeWidgetRegistry,
        spec: &CompiledNativeWidgetSpec,
    ) -> Result<Self, AvengerChartError> {
        let resolved = registry.resolve(spec)?;
        let mut part_manifests = IndexMap::new();
        for manifest in resolved
            .factory
            .part_manifests(spec, resolved.payload.as_ref())?
        {
            avenger_chart_core::validate_structural_id(
                "native widget part manifest",
                &manifest.name,
            )?;
            if part_manifests
                .insert(manifest.name.clone(), manifest)
                .is_some()
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Native widget '{}' factory returned duplicate part manifests",
                    spec.id
                )));
            }
        }
        let part_order = part_manifests.keys().cloned().collect();
        let part_kinds = part_manifests
            .iter()
            .map(|(name, manifest)| (name.clone(), manifest.scene_mark_kind.clone()))
            .collect();
        Ok(Self {
            spec_kind: spec.kind.clone(),
            schema_version: spec.schema_version,
            canonical_payload: spec.payload.as_str().to_string(),
            part_manifests,
            state: Mutex::new(NativeWidgetLiveState {
                instance: resolved.factory.create(spec, resolved.payload.as_ref())?,
                state_fingerprint: None,
                environment_digest: None,
                environment: None,
                base_font_size: 12.0,
                scene: None,
                part_order,
                part_kinds,
                pending_scene_dirty: false,
                pending_index_dirty: false,
                scene_rebuilds: 0,
                index_rebuilds: 0,
                detached: false,
                unmounted: false,
            }),
        })
    }

    fn validate_spec(&self, spec: &CompiledNativeWidgetSpec) -> Result<(), AvengerChartError> {
        if self.spec_kind != spec.kind
            || self.schema_version != spec.schema_version
            || self.canonical_payload != spec.payload.as_str()
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Native widget '{}' reused a persistent instance key with a different kind, schema version, or payload",
                spec.id
            )));
        }
        Ok(())
    }

    pub fn sync_and_render(
        &self,
        state_values: &IndexMap<String, ScalarValue>,
        state_revisions: &IndexMap<String, u64>,
        environment: &NativeWidgetEnvironment,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetRuntimeRender, AvengerChartError> {
        let state_fingerprint = state_values
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    state_revisions.get(name).copied().unwrap_or(0),
                    format!("{value:?}"),
                )
            })
            .collect::<Vec<_>>();
        let mut state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        state.base_font_size = ctx.base_font_size;
        if state.unmounted {
            return Err(AvengerChartError::InternalError(format!(
                "Native widget '{}' was rendered after unmount",
                ctx.slot.key().widget_id()
            )));
        }

        let previous_part_count = state.part_order.len();
        let mut scene_dirty = state.scene.is_none() || state.pending_scene_dirty;
        let mut index_dirty = state.scene.is_none() || state.pending_index_dirty;
        state.pending_scene_dirty = false;
        state.pending_index_dirty = false;
        if state.state_fingerprint.as_ref() != Some(&state_fingerprint) {
            state.instance.on_state_sync(
                NativeWidgetStateSnapshot::new(state_values, state_revisions),
                ctx,
            )?;
            state.state_fingerprint = Some(state_fingerprint);
        }
        let environment_changed = state.environment_digest != Some(environment.digest());
        if environment_changed || state.detached {
            let geometry_changed = state
                .environment
                .as_ref()
                .is_none_or(|previous| previous.geometry_digest() != environment.geometry_digest());
            state.instance.on_environment_sync(environment, ctx)?;
            state.environment_digest = Some(environment.digest());
            state.environment = Some(environment.clone());
            scene_dirty |= environment_changed;
            index_dirty |= environment_changed && geometry_changed;
        }
        let mut outcome = ctx.take_outcome();
        scene_dirty |= outcome.scene_dirty;
        index_dirty |= outcome.index_dirty;

        if scene_dirty {
            let mut scene = state.instance.scene(environment, ctx)?;
            let live = &mut *state;
            validate_native_widget_scene(
                ctx.slot.key().widget_id(),
                &self.part_manifests,
                &mut live.part_order,
                &mut live.part_kinds,
                &mut scene,
            )?;
            let scene_outcome = ctx.take_outcome();
            index_dirty |= scene_outcome.index_dirty;
            merge_native_widget_outcome(&mut outcome, scene_outcome);
            state.scene = Some(scene);
            state.scene_rebuilds = state
                .scene_rebuilds
                .checked_add(1)
                .expect("native widget scene rebuild counter exhausted");
        }
        if index_dirty {
            state.index_rebuilds = state
                .index_rebuilds
                .checked_add(1)
                .expect("native widget index rebuild counter exhausted");
        }

        state.detached = false;
        let parts_changed = state.part_order.len() != previous_part_count;
        Ok(NativeWidgetRuntimeRender {
            scene: state
                .scene
                .clone()
                .expect("dirty or previously cached native scene"),
            outcome,
            scene_rebuilt: scene_dirty,
            index_rebuilt: index_dirty,
            parts_changed,
            scene_rebuilds: state.scene_rebuilds,
            index_rebuilds: state.index_rebuilds,
        })
    }

    pub fn dispatch(
        &self,
        event: &NativeWidgetEvent,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let mut state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        if state.unmounted {
            return Ok(NativeWidgetDispatchOutcome::default());
        }
        state.instance.on_event(event, ctx)?;
        let outcome = ctx.take_outcome();
        state.pending_scene_dirty |= outcome.scene_dirty;
        state.pending_index_dirty |= outcome.index_dirty;
        Ok(outcome)
    }

    pub fn on_deactivate(
        &self,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let mut state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        if !state.unmounted {
            state.instance.on_deactivate(ctx)?;
        }
        let outcome = ctx.take_outcome();
        state.pending_scene_dirty |= outcome.scene_dirty;
        state.pending_index_dirty |= outcome.index_dirty;
        Ok(outcome)
    }

    pub fn on_session_detach(
        &self,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let mut state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        if !state.unmounted && !state.detached {
            state.instance.on_session_detach(ctx)?;
            state.detached = true;
        }
        Ok(ctx.take_outcome())
    }

    pub fn on_unmount(
        &self,
        ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let mut state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        if !state.unmounted {
            if !state.detached {
                state.instance.on_session_detach(ctx)?;
                state.detached = true;
            }
            state.instance.on_unmount(ctx)?;
            state.unmounted = true;
        }
        Ok(ctx.take_outcome())
    }

    pub fn rebuild_counts(&self) -> (u64, u64) {
        let state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        (state.scene_rebuilds, state.index_rebuilds)
    }

    fn resolved_part_manifests(&self) -> Vec<WidgetPartManifest> {
        let state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        state
            .part_order
            .iter()
            .map(|part| {
                self.part_manifests
                    .get(part)
                    .cloned()
                    .unwrap_or_else(|| WidgetPartManifest {
                        name: part.clone(),
                        scene_mark_kind: state.part_kinds[part].clone(),
                        style_properties: WidgetStyleProperty::ALL.to_vec(),
                        states: Vec::new(),
                        interactive: true,
                    })
            })
            .collect()
    }

    fn environment(&self) -> Option<NativeWidgetEnvironment> {
        self.state
            .lock()
            .expect("native widget live instance lock poisoned")
            .environment
            .clone()
    }

    fn style_context(&self) -> Option<(NativeWidgetEnvironment, f32)> {
        let state = self
            .state
            .lock()
            .expect("native widget live instance lock poisoned");
        state
            .environment
            .clone()
            .map(|environment| (environment, state.base_font_size))
    }
}

#[derive(Clone)]
pub struct NativeWidgetRuntimeRender {
    pub scene: NativeWidgetScene,
    pub outcome: NativeWidgetDispatchOutcome,
    pub scene_rebuilt: bool,
    pub index_rebuilt: bool,
    pub parts_changed: bool,
    pub scene_rebuilds: u64,
    pub index_rebuilds: u64,
}

pub(crate) fn merge_native_widget_outcome(
    target: &mut NativeWidgetDispatchOutcome,
    mut additional: NativeWidgetDispatchOutcome,
) {
    target
        .param_assignments
        .append(&mut additional.param_assignments);
    target.evaluation_intent = match (target.evaluation_intent, additional.evaluation_intent) {
        (NativeWidgetEvaluationIntent::Exact, _) | (_, NativeWidgetEvaluationIntent::Exact) => {
            NativeWidgetEvaluationIntent::Exact
        }
        (NativeWidgetEvaluationIntent::Preview, _) | (_, NativeWidgetEvaluationIntent::Preview) => {
            NativeWidgetEvaluationIntent::Preview
        }
        _ => NativeWidgetEvaluationIntent::None,
    };
    target.scene_dirty |= additional.scene_dirty;
    target.index_dirty |= additional.index_dirty;
    if additional.focus.is_some() {
        target.focus = additional.focus;
    }
    if additional.cursor.is_some() {
        target.cursor = additional.cursor;
    }
    target.commands.append(&mut additional.commands);
    target.consume |= additional.consume;
}

fn validate_native_widget_scene(
    widget_id: &str,
    manifests: &IndexMap<String, WidgetPartManifest>,
    part_order: &mut Vec<String>,
    part_kinds: &mut HashMap<String, String>,
    scene: &mut NativeWidgetScene,
) -> Result<(), AvengerChartError> {
    for (part, mark) in &mut scene.parts {
        avenger_chart_core::validate_structural_id("native widget part", part)?;
        let kind = native_scene_mark_kind(mark).to_string();
        if let Some(previous) = part_kinds.get(part) {
            if previous != &kind {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Native widget '{widget_id}' changed part '{part}' from scene mark kind '{previous}' to '{kind}'"
                )));
            }
        } else {
            part_order.push(part.clone());
            part_kinds.insert(part.clone(), kind);
        }
        set_native_scene_mark_name(mark, part);
        set_native_scene_mark_interactive(
            mark,
            manifests
                .get(part)
                .is_none_or(|manifest| manifest.interactive),
        );
    }
    if scene.parts.len()
        != scene
            .parts
            .keys()
            .collect::<std::collections::HashSet<_>>()
            .len()
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Native widget '{widget_id}' returned duplicate scene part ids"
        )));
    }
    let mut supplied = std::mem::take(&mut scene.parts);
    scene.parts = part_order
        .iter()
        .map(|part| {
            let mark = supplied.shift_remove(part).unwrap_or_else(|| {
                SceneMark::Group(avenger_scenegraph::marks::group::SceneGroup {
                    name: part.clone(),
                    interactive: false,
                    ..Default::default()
                })
            });
            (part.clone(), mark)
        })
        .collect();
    Ok(())
}

fn native_scene_mark_kind(mark: &SceneMark) -> &'static str {
    match mark {
        SceneMark::Arc(_) => "arc",
        SceneMark::Area(_) => "area",
        SceneMark::Group(_) => "group",
        SceneMark::Image(_) => "image",
        SceneMark::Line(_) => "line",
        SceneMark::Path(_) => "path",
        SceneMark::Rect(_) => "rect",
        SceneMark::Rule(_) => "rule",
        SceneMark::Symbol(_) => "symbol",
        SceneMark::Text(_) => "text",
        SceneMark::Trail(_) => "trail",
        SceneMark::WarpedImage(_) => "warped_image",
    }
}

fn set_native_scene_mark_interactive(mark: &mut SceneMark, interactive: bool) {
    match mark {
        SceneMark::Arc(mark) => mark.interactive = interactive,
        SceneMark::Area(mark) => mark.interactive = interactive,
        SceneMark::Group(mark) => mark.interactive = interactive,
        SceneMark::Image(mark) => Arc::make_mut(mark).interactive = interactive,
        SceneMark::Line(mark) => mark.interactive = interactive,
        SceneMark::Path(mark) => mark.interactive = interactive,
        SceneMark::Rect(mark) => mark.interactive = interactive,
        SceneMark::Rule(mark) => mark.interactive = interactive,
        SceneMark::Symbol(mark) => mark.interactive = interactive,
        SceneMark::Text(mark) => Arc::make_mut(mark).interactive = interactive,
        SceneMark::Trail(mark) => mark.interactive = interactive,
        SceneMark::WarpedImage(mark) => Arc::make_mut(mark).interactive = interactive,
    }
}

fn set_native_scene_mark_name(mark: &mut SceneMark, name: &str) {
    match mark {
        SceneMark::Arc(mark) => mark.name = name.to_string(),
        SceneMark::Area(mark) => mark.name = name.to_string(),
        SceneMark::Group(mark) => mark.name = name.to_string(),
        SceneMark::Image(mark) => Arc::make_mut(mark).name = name.to_string(),
        SceneMark::Line(mark) => mark.name = name.to_string(),
        SceneMark::Path(mark) => mark.name = name.to_string(),
        SceneMark::Rect(mark) => mark.name = name.to_string(),
        SceneMark::Rule(mark) => mark.name = name.to_string(),
        SceneMark::Symbol(mark) => mark.name = name.to_string(),
        SceneMark::Text(mark) => Arc::make_mut(mark).name = name.to_string(),
        SceneMark::Trail(mark) => mark.name = name.to_string(),
        SceneMark::WarpedImage(mark) => Arc::make_mut(mark).name = name.to_string(),
    }
}

struct NativeWidgetSessionAttachment {
    slot: Arc<NativeWidgetInstanceSlot>,
    epoch: NativeWidgetAttachmentEpoch,
    instance: Arc<NativeWidgetInstanceHandle>,
}

struct NativeWidgetEvaluationRuntimeInner {
    registry: Arc<NativeWidgetRegistry>,
    store: Arc<dyn NativeWidgetInstanceStore>,
    namespace: NativeWidgetNamespace,
    attachments: Mutex<HashMap<String, Arc<NativeWidgetSessionAttachment>>>,
    services: NativeWidgetHostServices,
    sink: Arc<Mutex<Vec<RuntimeHostCommand>>>,
    evaluation_outputs: Mutex<IndexMap<String, NativeWidgetEvaluationOutput>>,
    measurement_cache: Mutex<HashMap<NativeWidgetMeasurementCacheKey, NativeWidgetMeasurement>>,
    param_revisions: Mutex<IndexMap<String, u64>>,
    now: Mutex<Instant>,
    remove_namespace_on_drop: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NativeWidgetMeasurementCacheKey {
    widget_id: String,
    kind: String,
    schema_version: u32,
    payload: String,
    geometry_style_digest: u64,
    base_font_size_bits: u32,
}

/// Session-owned bridge from pure compiled descriptors to persistent live
/// instances. Clones share the same attachment epochs and dirty caches.
#[derive(Clone)]
pub(crate) struct NativeWidgetEvaluationRuntime {
    inner: Arc<NativeWidgetEvaluationRuntimeInner>,
}

impl NativeWidgetEvaluationRuntime {
    pub(crate) fn new(
        registry: Arc<NativeWidgetRegistry>,
        store: Arc<dyn NativeWidgetInstanceStore>,
        namespace: NativeWidgetNamespace,
        remove_namespace_on_drop: bool,
    ) -> Self {
        let services = store.host_services(&namespace);
        Self {
            inner: Arc::new(NativeWidgetEvaluationRuntimeInner {
                registry,
                store,
                namespace,
                attachments: Mutex::new(HashMap::new()),
                services,
                sink: Arc::new(Mutex::new(Vec::new())),
                evaluation_outputs: Mutex::new(IndexMap::new()),
                measurement_cache: Mutex::new(HashMap::new()),
                param_revisions: Mutex::new(IndexMap::new()),
                now: Mutex::new(Instant::now()),
                remove_namespace_on_drop,
            }),
        }
    }

    pub(crate) fn begin_evaluation(&self) {
        self.inner
            .evaluation_outputs
            .lock()
            .expect("native widget evaluation output lock poisoned")
            .clear();
        self.inner
            .sink
            .lock()
            .expect("native widget runtime command sink poisoned")
            .clear();
    }

    pub(crate) fn set_now(&self, now: Instant) {
        *self
            .inner
            .now
            .lock()
            .expect("native widget runtime clock lock poisoned") = now;
    }

    pub(crate) fn set_param_revisions(&self, revisions: IndexMap<String, u64>) {
        *self
            .inner
            .param_revisions
            .lock()
            .expect("native widget parameter revision lock poisoned") = revisions;
    }

    pub(crate) fn now(&self) -> Instant {
        *self
            .inner
            .now
            .lock()
            .expect("native widget runtime clock lock poisoned")
    }

    pub(crate) fn measure(
        &self,
        spec: &CompiledNativeWidgetSpec,
        ctx: &NativeWidgetFactoryContext<'_>,
    ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
        let key = NativeWidgetMeasurementCacheKey {
            widget_id: spec.id.clone(),
            kind: spec.kind.clone(),
            schema_version: spec.schema_version,
            payload: spec.payload.as_str().to_string(),
            geometry_style_digest: ctx.styles.geometry_digest,
            base_font_size_bits: ctx.base_font_size.to_bits(),
        };
        if let Some(measurement) = self
            .inner
            .measurement_cache
            .lock()
            .expect("native widget measurement cache lock poisoned")
            .get(&key)
            .copied()
        {
            return Ok(measurement);
        }
        let measurement = self.inner.registry.measure(spec, ctx)?;
        self.inner
            .measurement_cache
            .lock()
            .expect("native widget measurement cache lock poisoned")
            .insert(key, measurement);
        Ok(measurement)
    }

    pub(crate) fn part_manifests(
        &self,
        spec: &CompiledNativeWidgetSpec,
    ) -> Result<Vec<WidgetPartManifest>, AvengerChartError> {
        if let Some(attachment) = self
            .inner
            .attachments
            .lock()
            .expect("native widget evaluation runtime lock poisoned")
            .get(&spec.id)
            .map(|attachment| attachment.instance.clone())
        {
            attachment.validate_spec(spec)?;
            return Ok(attachment.resolved_part_manifests());
        }
        let resolved = self.inner.registry.resolve(spec)?;
        resolved
            .factory
            .part_manifests(spec, resolved.payload.as_ref())
    }

    fn attachment(
        &self,
        spec: &CompiledNativeWidgetSpec,
    ) -> Result<Arc<NativeWidgetSessionAttachment>, AvengerChartError> {
        let mut attachments = self
            .inner
            .attachments
            .lock()
            .expect("native widget evaluation runtime lock poisoned");
        if let Some(attachment) = attachments.get(&spec.id) {
            attachment.instance.validate_spec(spec)?;
            return Ok(attachment.clone());
        }

        // Resolve kind/version/payload before claiming a slot, so a malformed
        // artifact cannot leave a persistent empty instance key behind.
        self.inner.registry.resolve(spec)?;
        let key = NativeWidgetInstanceKey::new(self.inner.namespace.clone(), spec.id.clone());
        let slot = self.inner.store.slot(key);
        let instance = slot
            .get_or_try_init(|| NativeWidgetInstanceHandle::create(&self.inner.registry, spec))
            .map_err(|error| match error {
                NativeWidgetSlotInitError::TypeMismatch(error) => {
                    AvengerChartError::InvalidArgument(error.to_string())
                }
                NativeWidgetSlotInitError::Initialization(error) => error,
            })?;
        instance.validate_spec(spec)?;
        let epoch = slot.attach();
        self.inner.services.attach(slot.key(), epoch);
        let attachment = Arc::new(NativeWidgetSessionAttachment {
            slot,
            epoch,
            instance,
        });
        attachments.insert(spec.id.clone(), attachment.clone());
        Ok(attachment)
    }

    fn routed_attachment(
        &self,
        route: &NativeWidgetEventRoute,
    ) -> Option<Arc<NativeWidgetSessionAttachment>> {
        let attachment = self
            .inner
            .attachments
            .lock()
            .expect("native widget evaluation runtime lock poisoned")
            .get(route.key.widget_id())
            .cloned()?;
        (attachment.slot.key() == &route.key
            && attachment.epoch == route.epoch
            && attachment.slot.is_active(route.epoch))
        .then_some(attachment)
    }

    pub(crate) fn route_event(&self, event: &SceneGraphEvent) -> Option<NativeWidgetEventRoute> {
        self.inner.services.route_event(event)
    }

    pub(crate) fn focused_route(&self) -> Option<NativeWidgetEventRoute> {
        let (key, epoch) = self.inner.services.focused_attachment()?;
        self.routed_attachment(&NativeWidgetEventRoute { key, epoch })
            .map(|attachment| NativeWidgetEventRoute {
                key: attachment.slot.key().clone(),
                epoch: attachment.epoch,
            })
    }

    pub(crate) fn dispatch(
        &self,
        route: &NativeWidgetEventRoute,
        event: &NativeWidgetEvent,
        transform: NativeWidgetHostTransform,
        now: Instant,
        base_font_size: f32,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let Some(attachment) = self.routed_attachment(route) else {
            return Ok(NativeWidgetDispatchOutcome::default());
        };
        let environment = attachment.instance.environment().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Native widget '{}' received an event before its first evaluation",
                route.key.widget_id()
            ))
        })?;
        let mut ctx = NativeWidgetCtx::for_existing_attachment(
            attachment.slot.clone(),
            attachment.epoch,
            self.inner.services.clone(),
            self.inner.sink.clone(),
            transform,
            now,
        )
        .with_style_snapshot(environment.styles, environment.params, base_font_size);
        attachment.instance.dispatch(event, &mut ctx)
    }

    pub(crate) fn deactivate(
        &self,
        route: &NativeWidgetEventRoute,
        transform: NativeWidgetHostTransform,
        now: Instant,
        base_font_size: f32,
    ) -> Result<NativeWidgetDispatchOutcome, AvengerChartError> {
        let Some(attachment) = self.routed_attachment(route) else {
            return Ok(NativeWidgetDispatchOutcome::default());
        };
        let environment = attachment.instance.environment().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Native widget '{}' was deactivated before its first evaluation",
                route.key.widget_id()
            ))
        })?;
        let mut ctx = NativeWidgetCtx::for_existing_attachment(
            attachment.slot.clone(),
            attachment.epoch,
            self.inner.services.clone(),
            self.inner.sink.clone(),
            transform,
            now,
        )
        .with_style_snapshot(environment.styles, environment.params, base_font_size);
        attachment.instance.on_deactivate(&mut ctx)
    }

    pub(crate) fn rebuild_counts(&self, route: &NativeWidgetEventRoute) -> Option<(u64, u64)> {
        self.routed_attachment(route)
            .map(|attachment| attachment.instance.rebuild_counts())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_and_render(
        &self,
        spec: &CompiledNativeWidgetSpec,
        state_values: &IndexMap<String, ScalarValue>,
        environment: &NativeWidgetEnvironment,
        transform: NativeWidgetHostTransform,
        now: Instant,
        base_font_size: f32,
    ) -> Result<NativeWidgetEvaluationOutput, AvengerChartError> {
        let attachment = self.attachment(spec)?;
        let mut ctx = NativeWidgetCtx::for_existing_attachment(
            attachment.slot.clone(),
            attachment.epoch,
            self.inner.services.clone(),
            self.inner.sink.clone(),
            transform,
            now,
        )
        .with_style_snapshot(
            environment.styles.clone(),
            environment.params.clone(),
            base_font_size,
        );
        let revisions = self
            .inner
            .param_revisions
            .lock()
            .expect("native widget parameter revision lock poisoned");
        let render =
            attachment
                .instance
                .sync_and_render(state_values, &revisions, environment, &mut ctx)?;
        let output = NativeWidgetEvaluationOutput {
            key: attachment.slot.key().clone(),
            epoch: attachment.epoch,
            render,
        };
        self.inner
            .evaluation_outputs
            .lock()
            .expect("native widget evaluation output lock poisoned")
            .insert(spec.id.clone(), output.clone());
        Ok(output)
    }

    pub(crate) fn evaluation_outputs(&self) -> IndexMap<String, NativeWidgetEvaluationOutput> {
        self.inner
            .evaluation_outputs
            .lock()
            .expect("native widget evaluation output lock poisoned")
            .clone()
    }

    pub(crate) fn detach_all(&self) {
        self.inner.detach_all(false);
    }
}

#[derive(Clone)]
pub(crate) struct NativeWidgetEvaluationOutput {
    pub(crate) key: NativeWidgetInstanceKey,
    pub(crate) epoch: NativeWidgetAttachmentEpoch,
    pub(crate) render: NativeWidgetRuntimeRender,
}

impl NativeWidgetEvaluationRuntimeInner {
    fn detach_all(&self, unmount: bool) {
        let attachments = std::mem::take(
            &mut *self
                .attachments
                .lock()
                .expect("native widget evaluation runtime lock poisoned"),
        );
        for attachment in attachments.into_values() {
            // A replacement session may already own a newer epoch for the
            // same stored instance. Old teardown must be completely inert:
            // no lifecycle callback, focus/IME mutation, or wake cancellation.
            if !attachment.slot.is_active(attachment.epoch) {
                continue;
            }
            let mut ctx = NativeWidgetCtx::for_existing_attachment(
                attachment.slot.clone(),
                attachment.epoch,
                self.services.clone(),
                self.sink.clone(),
                NativeWidgetHostTransform::default(),
                *self
                    .now
                    .lock()
                    .expect("native widget runtime clock lock poisoned"),
            );
            if unmount {
                let _ = attachment.instance.on_unmount(&mut ctx);
            } else {
                let _ = attachment.instance.on_session_detach(&mut ctx);
            }
            if attachment.slot.detach(attachment.epoch) {
                self.services
                    .detach(attachment.slot.key(), attachment.epoch, self.sink.as_ref());
            }
        }
    }
}

impl Drop for NativeWidgetEvaluationRuntimeInner {
    fn drop(&mut self) {
        self.detach_all(self.remove_namespace_on_drop);
        if self.remove_namespace_on_drop {
            self.store.remove_namespace(&self.namespace);
        }
    }
}

#[derive(Clone)]
pub struct NativeWidgetRuntimeResources {
    pub registry: Arc<NativeWidgetRegistry>,
    pub instance_store: Arc<dyn NativeWidgetInstanceStore>,
    pub document_id: NativeWidgetDocumentId,
}

impl NativeWidgetRuntimeResources {
    pub fn new(
        registry: Arc<NativeWidgetRegistry>,
        instance_store: Arc<dyn NativeWidgetInstanceStore>,
        document_id: NativeWidgetDocumentId,
    ) -> Self {
        Self {
            registry,
            instance_store,
            document_id,
        }
    }

    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(NativeWidgetRegistry::new()),
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        )
    }

    pub fn namespace(&self, plot_id: NativeWidgetPlotId) -> NativeWidgetNamespace {
        NativeWidgetNamespace::new(self.document_id, plot_id)
    }

    pub fn host_services(&self, plot_id: NativeWidgetPlotId) -> NativeWidgetHostServices {
        self.instance_store.host_services(&self.namespace(plot_id))
    }

    pub fn set_host_services(
        &self,
        plot_id: NativeWidgetPlotId,
        services: NativeWidgetHostServices,
    ) {
        self.instance_store
            .set_host_services(self.namespace(plot_id), services);
    }

    /// Final-evict every live native instance for one plot member. Sessions
    /// must be detached first; unlike session teardown, this invokes
    /// `on_unmount` exactly once before the slots are removed.
    pub fn evict_plot(
        &self,
        plot_id: NativeWidgetPlotId,
        now: Instant,
    ) -> Result<Vec<RuntimeHostCommand>, AvengerChartError> {
        let namespace = self.namespace(plot_id);
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        for slot in self.instance_store.slots_for_namespace(&namespace) {
            let Some(instance) = slot.take_instance::<NativeWidgetInstanceHandle>()? else {
                continue;
            };
            let mut ctx = NativeWidgetCtx::attach_at(
                slot,
                services.clone(),
                sink.clone(),
                NativeWidgetHostTransform::default(),
                now,
            );
            if let Some((environment, base_font_size)) = instance.style_context() {
                ctx =
                    ctx.with_style_snapshot(environment.styles, environment.params, base_font_size);
            }
            instance.on_unmount(&mut ctx)?;
        }
        self.instance_store.remove_namespace(&namespace);
        let commands = std::mem::take(
            &mut *sink
                .lock()
                .expect("native widget eviction command sink poisoned"),
        );
        Ok(commands)
    }
}

pub trait NativeWidgetHostCommandSink: Send + Sync {
    fn push(&self, command: RuntimeHostCommand);
}

impl NativeWidgetHostCommandSink for Mutex<Vec<RuntimeHostCommand>> {
    fn push(&self, command: RuntimeHostCommand) {
        self.lock()
            .expect("native widget command sink lock poisoned")
            .push(command);
    }
}

struct NativeWidgetOutcomeSink<'a> {
    sink: &'a dyn NativeWidgetHostCommandSink,
    outcome: &'a Mutex<NativeWidgetDispatchOutcome>,
}

impl NativeWidgetHostCommandSink for NativeWidgetOutcomeSink<'_> {
    fn push(&self, command: RuntimeHostCommand) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .commands
            .push(command.clone());
        self.sink.push(command);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NativeWidgetHostTransform {
    offset: [f32; 2],
}

impl NativeWidgetHostTransform {
    pub fn from_offsets(offsets: impl IntoIterator<Item = [f32; 2]>) -> Option<Self> {
        let mut offset = [0.0, 0.0];
        for next in offsets {
            if !next[0].is_finite() || !next[1].is_finite() {
                return None;
            }
            offset[0] += next[0];
            offset[1] += next[1];
            if !offset[0].is_finite() || !offset[1].is_finite() {
                return None;
            }
        }
        Some(Self { offset })
    }

    pub fn map_rect(self, rect: LogicalRect) -> LogicalRect {
        LogicalRect::new(
            rect.x() + self.offset[0],
            rect.y() + self.offset[1],
            rect.width(),
            rect.height(),
        )
        .expect("finite host transform preserves a finite logical rectangle")
    }
}

#[derive(Clone)]
struct FocusedNativeWidget {
    key: NativeWidgetInstanceKey,
    epoch: NativeWidgetAttachmentEpoch,
    caret: Option<LogicalRect>,
    clipboard_payload: String,
}

#[derive(Default)]
struct NativeWidgetHostServiceState {
    focused: Option<FocusedNativeWidget>,
    attachments: HashMap<NativeWidgetInstanceKey, NativeWidgetAttachmentEpoch>,
}

#[derive(Clone, Default)]
pub struct NativeWidgetHostServices {
    state: Arc<Mutex<NativeWidgetHostServiceState>>,
}

/// The live native-widget attachment selected for one neutral event.
///
/// Keyboard, IME, and clipboard events route only to the focused attachment.
/// Runtime wakes route by their full instance namespace and attachment epoch,
/// whether or not that attachment currently owns focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeWidgetEventRoute {
    pub key: NativeWidgetInstanceKey,
    pub epoch: NativeWidgetAttachmentEpoch,
}

impl NativeWidgetHostServices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused_clipboard_payload(&self) -> Option<String> {
        self.state
            .lock()
            .expect("native widget host service lock poisoned")
            .focused
            .as_ref()
            .map(|focused| focused.clipboard_payload.clone())
    }

    pub fn focused_attachment(
        &self,
    ) -> Option<(NativeWidgetInstanceKey, NativeWidgetAttachmentEpoch)> {
        self.state
            .lock()
            .expect("native widget host service lock poisoned")
            .focused
            .as_ref()
            .map(|focused| (focused.key.clone(), focused.epoch))
    }

    /// Resolve the native attachment eligible to receive this event.
    pub fn route_event(&self, event: &SceneGraphEvent) -> Option<NativeWidgetEventRoute> {
        let state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        match event {
            SceneGraphEvent::KeyPress(_)
            | SceneGraphEvent::KeyRelease(_)
            | SceneGraphEvent::Ime(_)
            | SceneGraphEvent::Clipboard(_) => {
                let focused = state.focused.as_ref()?;
                (state.attachments.get(&focused.key) == Some(&focused.epoch)).then(|| {
                    NativeWidgetEventRoute {
                        key: focused.key.clone(),
                        epoch: focused.epoch,
                    }
                })
            }
            SceneGraphEvent::RuntimeWake(wake) => {
                state.attachments.iter().find_map(|(key, epoch)| {
                    (key.wake_namespace() == wake.key.namespace
                        && epoch.get() == wake.key.attachment_epoch)
                        .then(|| NativeWidgetEventRoute {
                            key: key.clone(),
                            epoch: *epoch,
                        })
                })
            }
            _ => None,
        }
    }

    fn attach(&self, key: &NativeWidgetInstanceKey, epoch: NativeWidgetAttachmentEpoch) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        state.attachments.insert(key.clone(), epoch);
        if let Some(focused) = state.focused.as_mut()
            && &focused.key == key
        {
            focused.epoch = epoch;
        }
    }

    fn detach(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if state.attachments.get(key) != Some(&epoch) {
            return;
        }
        state.attachments.remove(key);
        let owns_focus = state
            .focused
            .as_ref()
            .is_some_and(|focused| &focused.key == key && focused.epoch == epoch);
        if owns_focus {
            state.focused = None;
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
    }

    fn focus(
        &self,
        key: NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        caret: Option<LogicalRect>,
        clipboard_payload: String,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        let same_attachment = state
            .focused
            .as_ref()
            .is_some_and(|focused| focused.key == key && focused.epoch == epoch);
        if same_attachment {
            let focused = state.focused.as_mut().expect("checked above");
            focused.clipboard_payload = clipboard_payload;
            if focused.caret != caret {
                focused.caret = caret;
                sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
            }
            return;
        }

        if state.focused.is_some() {
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
        state.focused = Some(FocusedNativeWidget {
            key,
            epoch,
            caret,
            clipboard_payload,
        });
        sink.push(RuntimeHostCommand::SetImeAllowed { allowed: true });
        sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
    }

    fn cancel_composition_and_refocus(
        &self,
        key: NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        caret: Option<LogicalRect>,
        clipboard_payload: String,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if state.focused.is_some() {
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
        state.focused = Some(FocusedNativeWidget {
            key,
            epoch,
            caret,
            clipboard_payload,
        });
        sink.push(RuntimeHostCommand::SetImeAllowed { allowed: true });
        sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
    }

    fn blur(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        let owns_focus = state
            .focused
            .as_ref()
            .is_some_and(|focused| &focused.key == key && focused.epoch == epoch);
        if owns_focus {
            state.focused = None;
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
    }

    fn set_clipboard_payload(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        payload: String,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if let Some(focused) = state.focused.as_mut()
            && &focused.key == key
            && focused.epoch == epoch
        {
            focused.clipboard_payload = payload;
        }
    }
}

/// Typed access to one part in the evaluation's resolved widget style set.
pub struct NativeWidgetPartTheme<'a> {
    widget_id: &'a str,
    style: &'a ResolvedWidgetPartStyle,
    params: &'a IndexMap<String, ScalarValue>,
    base_font_size: f32,
}

impl<'a> NativeWidgetPartTheme<'a> {
    pub fn value(
        &self,
        property: WidgetStyleProperty,
    ) -> Option<&'a avenger_chart_core::ThemeValue> {
        self.style.values.get(&property)
    }

    pub fn length(&self, property: WidgetStyleProperty) -> Result<Option<f32>, AvengerChartError> {
        if property.value_type() != WidgetStyleValueType::Length {
            return Err(self.type_error(property, "length"));
        }
        self.value(property)
            .map(|value| {
                value
                    .eval_as_length(&avenger_chart_core::theme::eval::EvalContext::new(
                        self.params,
                        self.base_font_size,
                    ))
                    .map(|value| value as f32)
                    .map_err(|error| self.invalid(property, error.to_string()))
            })
            .transpose()
    }

    pub fn number(&self, property: WidgetStyleProperty) -> Result<Option<f32>, AvengerChartError> {
        if property.value_type() != WidgetStyleValueType::Number {
            return Err(self.type_error(property, "number"));
        }
        self.value(property)
            .map(|value| {
                value
                    .eval_as_number(&avenger_chart_core::theme::eval::EvalContext::new(
                        self.params,
                        self.base_font_size,
                    ))
                    .map(|value| value as f32)
                    .map_err(|error| self.invalid(property, error.to_string()))
            })
            .transpose()
    }

    pub fn color(
        &self,
        property: WidgetStyleProperty,
    ) -> Result<Option<[f32; 4]>, AvengerChartError> {
        if property.value_type() != WidgetStyleValueType::Color {
            return Err(self.type_error(property, "color"));
        }
        self.value(property)
            .map(|value| {
                value
                    .eval_as_color(&avenger_chart_core::theme::eval::EvalContext::new(
                        self.params,
                        self.base_font_size,
                    ))
                    .map(|color| color.to_array())
                    .map_err(|error| self.invalid(property, error.to_string()))
            })
            .transpose()
    }

    pub fn string(
        &self,
        property: WidgetStyleProperty,
    ) -> Result<Option<String>, AvengerChartError> {
        if !matches!(
            property.value_type(),
            WidgetStyleValueType::String
                | WidgetStyleValueType::Cursor
                | WidgetStyleValueType::FontWeight
        ) {
            return Err(self.type_error(property, "string"));
        }
        self.value(property)
            .map(|value| {
                value
                    .eval_as_string(&avenger_chart_core::theme::eval::EvalContext::new(
                        self.params,
                        self.base_font_size,
                    ))
                    .map_err(|error| self.invalid(property, error.to_string()))
            })
            .transpose()
    }

    fn type_error(&self, property: WidgetStyleProperty, expected: &str) -> AvengerChartError {
        self.invalid(
            property,
            format!(
                "requested as {expected}, but property has {:?} type",
                property.value_type()
            ),
        )
    }

    fn invalid(&self, property: WidgetStyleProperty, message: String) -> AvengerChartError {
        AvengerChartError::InvalidWidgetStyle {
            widget_id: self.widget_id.to_string(),
            property: property.name().to_string(),
            message,
        }
    }
}

pub struct NativeWidgetCtx {
    slot: Arc<NativeWidgetInstanceSlot>,
    epoch: NativeWidgetAttachmentEpoch,
    services: NativeWidgetHostServices,
    sink: Arc<dyn NativeWidgetHostCommandSink>,
    transform: NativeWidgetHostTransform,
    now: Instant,
    styles: Option<Arc<ResolvedWidgetStyleSet>>,
    style_params: Arc<IndexMap<String, ScalarValue>>,
    base_font_size: f32,
    outcome: Mutex<NativeWidgetDispatchOutcome>,
    owns_attachment: bool,
}

impl NativeWidgetCtx {
    pub fn attach(
        slot: Arc<NativeWidgetInstanceSlot>,
        services: NativeWidgetHostServices,
        sink: Arc<dyn NativeWidgetHostCommandSink>,
        transform: NativeWidgetHostTransform,
    ) -> Self {
        Self::attach_at(slot, services, sink, transform, Instant::now())
    }

    pub fn attach_at(
        slot: Arc<NativeWidgetInstanceSlot>,
        services: NativeWidgetHostServices,
        sink: Arc<dyn NativeWidgetHostCommandSink>,
        transform: NativeWidgetHostTransform,
        now: Instant,
    ) -> Self {
        let epoch = slot.attach();
        services.attach(slot.key(), epoch);
        Self {
            slot,
            epoch,
            services,
            sink,
            transform,
            now,
            styles: None,
            style_params: Arc::new(IndexMap::new()),
            base_font_size: 12.0,
            outcome: Mutex::new(NativeWidgetDispatchOutcome::default()),
            owns_attachment: true,
        }
    }

    fn for_existing_attachment(
        slot: Arc<NativeWidgetInstanceSlot>,
        epoch: NativeWidgetAttachmentEpoch,
        services: NativeWidgetHostServices,
        sink: Arc<dyn NativeWidgetHostCommandSink>,
        transform: NativeWidgetHostTransform,
        now: Instant,
    ) -> Self {
        services.attach(slot.key(), epoch);
        Self {
            slot,
            epoch,
            services,
            sink,
            transform,
            now,
            styles: None,
            style_params: Arc::new(IndexMap::new()),
            base_font_size: 12.0,
            outcome: Mutex::new(NativeWidgetDispatchOutcome::default()),
            owns_attachment: false,
        }
    }

    pub fn with_style_snapshot(
        mut self,
        styles: Arc<ResolvedWidgetStyleSet>,
        params: Arc<IndexMap<String, ScalarValue>>,
        base_font_size: f32,
    ) -> Self {
        self.styles = Some(styles);
        self.style_params = params;
        self.base_font_size = base_font_size;
        self
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    pub fn set_now(&mut self, now: Instant) {
        self.now = now;
    }

    pub fn epoch(&self) -> NativeWidgetAttachmentEpoch {
        self.epoch
    }

    pub fn is_active(&self) -> bool {
        self.slot.is_active(self.epoch)
    }

    pub fn text_cursor(&self) -> CursorStyle {
        CursorStyle::Text
    }

    pub fn part_theme(
        &self,
        part: &str,
        mark_type: &str,
    ) -> Result<NativeWidgetPartTheme<'_>, AvengerChartError> {
        avenger_chart_core::validate_structural_id("native widget part", part)?;
        avenger_chart_core::validate_structural_id("native widget scene mark kind", mark_type)?;
        let styles = self.styles.as_ref().ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Native widget '{}' has no resolved style snapshot",
                self.slot.key().widget_id()
            ))
        })?;
        let style =
            styles
                .parts
                .get(part)
                .ok_or_else(|| AvengerChartError::InvalidWidgetStyle {
                    widget_id: self.slot.key().widget_id().to_string(),
                    property: format!("part({part})"),
                    message: format!(
                        "part was not resolved for scene mark kind '{mark_type}' in this snapshot"
                    ),
                })?;
        Ok(NativeWidgetPartTheme {
            widget_id: self.slot.key().widget_id(),
            style,
            params: self.style_params.as_ref(),
            base_font_size: self.base_font_size,
        })
    }

    pub fn assign_param(&self, assignment: ScopedParamAssignment) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .param_assignments
            .push(assignment);
    }

    pub fn request_evaluation(&self, intent: NativeWidgetEvaluationIntent) {
        let mut outcome = self
            .outcome
            .lock()
            .expect("native widget outcome lock poisoned");
        outcome.evaluation_intent = match (outcome.evaluation_intent, intent) {
            (NativeWidgetEvaluationIntent::Exact, _) | (_, NativeWidgetEvaluationIntent::Exact) => {
                NativeWidgetEvaluationIntent::Exact
            }
            (NativeWidgetEvaluationIntent::Preview, _)
            | (_, NativeWidgetEvaluationIntent::Preview) => NativeWidgetEvaluationIntent::Preview,
            _ => NativeWidgetEvaluationIntent::None,
        };
    }

    pub fn mark_scene_dirty(&self) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .scene_dirty = true;
    }

    pub fn mark_index_dirty(&self) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .index_dirty = true;
    }

    pub fn set_cursor(&self, cursor: CursorStyle) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .cursor = Some(cursor);
    }

    pub fn consume(&self) {
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .consume = true;
    }

    pub fn take_outcome(&self) -> NativeWidgetDispatchOutcome {
        std::mem::take(
            &mut *self
                .outcome
                .lock()
                .expect("native widget outcome lock poisoned"),
        )
    }

    fn outcome_sink(&self) -> NativeWidgetOutcomeSink<'_> {
        NativeWidgetOutcomeSink {
            sink: self.sink.as_ref(),
            outcome: &self.outcome,
        }
    }

    pub fn focus(&self, caret: Option<LogicalRect>, clipboard_payload: impl Into<String>) {
        if !self.is_active() {
            return;
        }
        let clipboard_payload = clipboard_payload.into();
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .focus = Some(NativeWidgetFocusRequest::Focus {
            caret,
            clipboard_payload: clipboard_payload.clone(),
            cancel_composition: false,
        });
        let sink = self.outcome_sink();
        self.services.focus(
            self.slot.key().clone(),
            self.epoch,
            caret.map(|rect| self.transform.map_rect(rect)),
            clipboard_payload,
            &sink,
        );
    }

    /// Cancel an active platform composition and restore focus at a new caret.
    ///
    /// Ordinary caret refreshes use [`Self::focus`] and are cached. Pointer
    /// click-away/click-back paths use this explicit off/on sequence so the
    /// platform IME cannot retain a composition at the old caret.
    pub fn cancel_composition_and_refocus(
        &self,
        caret: Option<LogicalRect>,
        clipboard_payload: impl Into<String>,
    ) {
        if !self.is_active() {
            return;
        }
        let clipboard_payload = clipboard_payload.into();
        self.outcome
            .lock()
            .expect("native widget outcome lock poisoned")
            .focus = Some(NativeWidgetFocusRequest::Focus {
            caret,
            clipboard_payload: clipboard_payload.clone(),
            cancel_composition: true,
        });
        let sink = self.outcome_sink();
        self.services.cancel_composition_and_refocus(
            self.slot.key().clone(),
            self.epoch,
            caret.map(|rect| self.transform.map_rect(rect)),
            clipboard_payload,
            &sink,
        );
    }

    pub fn blur(&self) {
        if self.is_active() {
            self.outcome
                .lock()
                .expect("native widget outcome lock poisoned")
                .focus = Some(NativeWidgetFocusRequest::Blur);
            let sink = self.outcome_sink();
            self.services.blur(self.slot.key(), self.epoch, &sink);
        }
    }

    pub fn set_clipboard_payload(&self, payload: impl Into<String>) {
        if self.is_active() {
            self.services
                .set_clipboard_payload(self.slot.key(), self.epoch, payload.into());
        }
    }

    pub fn write_clipboard(&self, text: impl Into<String>) {
        if self.is_active() {
            self.outcome_sink()
                .push(RuntimeHostCommand::WriteClipboard { text: text.into() });
        }
    }

    pub fn request_wakeup(&self, purpose: &str, deadline: Instant, generation: u64) {
        if self.is_active() {
            self.outcome_sink().push(RuntimeHostCommand::RequestWakeup {
                key: self.runtime_wake_key(purpose),
                deadline,
                generation,
            });
        }
    }

    pub fn cancel_wakeup(&self, purpose: &str) {
        if self.is_active() {
            self.outcome_sink().push(RuntimeHostCommand::CancelWakeup {
                key: self.runtime_wake_key(purpose),
            });
        }
    }

    pub fn accepts_wakeup(&self, wake: &RuntimeWakeEvent, purpose: &str) -> bool {
        self.is_active() && wake.key == self.runtime_wake_key(purpose)
    }

    pub fn accepts_wakeup_generation(
        &self,
        wake: &RuntimeWakeEvent,
        purpose: &str,
        expected_generation: u64,
    ) -> bool {
        self.accepts_wakeup(wake, purpose) && wake.generation == expected_generation
    }

    fn runtime_wake_key(&self, purpose: &str) -> RuntimeWakeKey {
        RuntimeWakeKey::new(self.slot.key().wake_namespace(), self.epoch.get(), purpose)
    }
}

impl Drop for NativeWidgetCtx {
    fn drop(&mut self) {
        if self.owns_attachment && self.slot.detach(self.epoch) {
            self.services
                .detach(self.slot.key(), self.epoch, self.sink.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        concat::HConcat,
        plot::{Chart, EvaluationRequest, PlotSessionOptions},
        widget_cell::WidgetCell,
        zerod::ZeroDCoord,
    };
    use avenger_chart_core::{
        CanonicalJson, LegendPosition, NativeWidget, NativeWidgetMeasureSpec,
        NativeWidgetPlacementExt, NativeWidgetStateSpec,
    };
    use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
    use datafusion::prelude::SessionContext;

    struct ToyNativeWidgetFactory;

    struct CountingToyNativeWidgetFactory {
        measurement_calls: Arc<AtomicU64>,
    }

    struct LifecycleNativeWidgetFactory {
        detach_calls: Arc<AtomicU64>,
        unmount_calls: Arc<AtomicU64>,
        callback_times: Arc<Mutex<Vec<Instant>>>,
    }

    struct ToyNativeWidget;

    impl NativeWidget for ToyNativeWidget {
        fn id(&self) -> &str {
            "toy"
        }

        fn kind(&self) -> &'static str {
            "toy-native"
        }

        fn schema_version(&self) -> u32 {
            1
        }

        fn payload(&self) -> serde_json::Value {
            serde_json::json!({ "width": 42.0 })
        }

        fn measure(&self) -> NativeWidgetMeasureSpec {
            NativeWidgetMeasureSpec::Registry
        }

        fn state(&self) -> NativeWidgetStateSpec {
            NativeWidgetStateSpec::try_new(Vec::new()).unwrap()
        }
    }

    impl NativeWidgetFactory for ToyNativeWidgetFactory {
        fn kind(&self) -> &'static str {
            "toy-native"
        }

        fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
            1..=2
        }

        fn measure(
            &self,
            spec: &CompiledNativeWidgetSpec,
            payload: &serde_json::Value,
            _ctx: &NativeWidgetFactoryContext<'_>,
        ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
            let width = payload["width"].as_f64().ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Toy native widget '{}' requires numeric width",
                    spec.id
                ))
            })? as f32;
            Ok(NativeWidgetMeasurement::fixed(width, 24.0))
        }

        fn create(
            &self,
            _spec: &CompiledNativeWidgetSpec,
            _payload: &serde_json::Value,
        ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
            Ok(Box::new(ToyNativeWidgetInstance))
        }
    }

    impl NativeWidgetFactory for CountingToyNativeWidgetFactory {
        fn kind(&self) -> &'static str {
            "toy-native"
        }

        fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
            1..=2
        }

        fn measure(
            &self,
            _spec: &CompiledNativeWidgetSpec,
            _payload: &serde_json::Value,
            _ctx: &NativeWidgetFactoryContext<'_>,
        ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
            self.measurement_calls.fetch_add(1, Ordering::Relaxed);
            Ok(NativeWidgetMeasurement::fixed(42.0, 24.0))
        }

        fn create(
            &self,
            _spec: &CompiledNativeWidgetSpec,
            _payload: &serde_json::Value,
        ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
            Ok(Box::new(ToyNativeWidgetInstance))
        }
    }

    impl NativeWidgetFactory for LifecycleNativeWidgetFactory {
        fn kind(&self) -> &'static str {
            "lifecycle-native"
        }

        fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
            1..=1
        }

        fn create(
            &self,
            _spec: &CompiledNativeWidgetSpec,
            _payload: &serde_json::Value,
        ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
            Ok(Box::new(LifecycleNativeWidgetInstance {
                detach_calls: self.detach_calls.clone(),
                unmount_calls: self.unmount_calls.clone(),
                callback_times: self.callback_times.clone(),
            }))
        }
    }

    struct LifecycleNativeWidgetInstance {
        detach_calls: Arc<AtomicU64>,
        unmount_calls: Arc<AtomicU64>,
        callback_times: Arc<Mutex<Vec<Instant>>>,
    }

    impl NativeWidgetInstance for LifecycleNativeWidgetInstance {
        fn scene(
            &mut self,
            _environment: &NativeWidgetEnvironment,
            _ctx: &mut NativeWidgetCtx,
        ) -> Result<NativeWidgetScene, AvengerChartError> {
            Ok(NativeWidgetScene::default())
        }

        fn on_session_detach(
            &mut self,
            ctx: &mut NativeWidgetCtx,
        ) -> Result<(), AvengerChartError> {
            self.detach_calls.fetch_add(1, Ordering::Relaxed);
            self.callback_times.lock().unwrap().push(ctx.now());
            Ok(())
        }

        fn on_unmount(&mut self, ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
            self.unmount_calls.fetch_add(1, Ordering::Relaxed);
            self.callback_times.lock().unwrap().push(ctx.now());
            Ok(())
        }
    }

    struct ToyNativeWidgetInstance;

    impl NativeWidgetInstance for ToyNativeWidgetInstance {
        fn on_event(
            &mut self,
            _event: &NativeWidgetEvent,
            ctx: &mut NativeWidgetCtx,
        ) -> Result<(), AvengerChartError> {
            ctx.mark_index_dirty();
            ctx.request_evaluation(NativeWidgetEvaluationIntent::Exact);
            ctx.consume();
            Ok(())
        }

        fn scene(
            &mut self,
            environment: &NativeWidgetEnvironment,
            _ctx: &mut NativeWidgetCtx,
        ) -> Result<NativeWidgetScene, AvengerChartError> {
            NativeWidgetScene::try_from_iter([(
                "body".to_string(),
                SceneMark::Rect(SceneRectMark {
                    x: 0.0.into(),
                    y: 0.0.into(),
                    width: Some(environment.frame_size[0].into()),
                    height: Some(environment.frame_size[1].into()),
                    ..Default::default()
                }),
            )])
        }
    }

    fn toy_spec(schema_version: u32) -> CompiledNativeWidgetSpec {
        CompiledNativeWidgetSpec {
            id: "toy".to_string(),
            kind: "toy-native".to_string(),
            schema_version,
            payload: CanonicalJson::from_value(serde_json::json!({ "width": 42.0 })).unwrap(),
            measure: NativeWidgetMeasureSpec::Registry,
            state: NativeWidgetStateSpec::try_new(Vec::new()).unwrap(),
        }
    }

    fn lifecycle_spec() -> CompiledNativeWidgetSpec {
        let mut spec = toy_spec(1);
        spec.id = "lifecycle".to_string();
        spec.kind = "lifecycle-native".to_string();
        spec.measure = NativeWidgetMeasureSpec::Declarative(
            avenger_chart_core::WidgetMeasureSpec::fixed(10.0, 10.0),
        );
        spec
    }

    fn commands(sink: &Arc<Mutex<Vec<RuntimeHostCommand>>>) -> Vec<RuntimeHostCommand> {
        std::mem::take(&mut *sink.lock().unwrap())
    }

    fn has_named_scene_mark(marks: &[SceneMark], name: &str) -> bool {
        marks.iter().any(|mark| {
            let matches = match mark {
                SceneMark::Arc(mark) => mark.name == name,
                SceneMark::Area(mark) => mark.name == name,
                SceneMark::Group(mark) => mark.name == name,
                SceneMark::Image(mark) => mark.name == name,
                SceneMark::Line(mark) => mark.name == name,
                SceneMark::Path(mark) => mark.name == name,
                SceneMark::Rect(mark) => mark.name == name,
                SceneMark::Rule(mark) => mark.name == name,
                SceneMark::Symbol(mark) => mark.name == name,
                SceneMark::Text(mark) => mark.name == name,
                SceneMark::Trail(mark) => mark.name == name,
                SceneMark::WarpedImage(mark) => mark.name == name,
            };
            matches
                || matches!(mark, SceneMark::Group(group) if has_named_scene_mark(&group.marks, name))
        })
    }

    #[test]
    fn registry_lookup_is_purely_evaluation_time_after_bincode() {
        let spec = toy_spec(1);
        let bytes = bincode::serialize(&spec).unwrap();
        let decoded: CompiledNativeWidgetSpec = bincode::deserialize(&bytes).unwrap();

        let empty = NativeWidgetRegistry::new();
        assert!(matches!(
            empty.resolve(&decoded),
            Err(AvengerChartError::UnknownNativeWidgetKind { widget_id, kind })
                if widget_id == "toy" && kind == "toy-native"
        ));

        let registry = NativeWidgetRegistry::new()
            .with_factory(ToyNativeWidgetFactory)
            .unwrap();
        let resolved = registry.resolve(&decoded).unwrap();
        assert_eq!(resolved.payload["width"], serde_json::json!(42.0));
        registry.create_instance(&decoded).unwrap();
    }

    #[test]
    fn registry_reports_unsupported_version_and_validates_measurement() {
        let registry = NativeWidgetRegistry::new()
            .with_factory(ToyNativeWidgetFactory)
            .unwrap();
        let unsupported = toy_spec(3);
        assert!(matches!(
            registry.resolve(&unsupported),
            Err(AvengerChartError::UnsupportedNativeWidgetSchemaVersion {
                widget_id,
                kind,
                schema_version: 3,
            }) if widget_id == "toy" && kind == "toy-native"
        ));

        let styles = Arc::new(ResolvedWidgetStyleSet::default());
        let params = IndexMap::new();
        let measurement = registry
            .measure(
                &toy_spec(1),
                &NativeWidgetFactoryContext {
                    params: &params,
                    styles: &styles,
                    base_font_size: 12.0,
                },
            )
            .unwrap();
        assert_eq!(measurement, NativeWidgetMeasurement::fixed(42.0, 24.0));
    }

    #[tokio::test]
    async fn installed_factory_evaluates_direct_and_bincode_artifacts_persistently() {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Chart::<ZeroDCoord>::new()
            .native_widget(ToyNativeWidget.position(LegendPosition::Bottom))
            .compile(ctx.as_ref())
            .await
            .unwrap();
        let bytes = bincode::serialize(&compiled).unwrap();
        let decoded: super::super::CompiledPlot = bincode::deserialize(&bytes).unwrap();

        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(ToyNativeWidgetFactory)
                .unwrap(),
        );
        let resources = NativeWidgetRuntimeResources::new(
            registry,
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        );
        let now = Instant::now();

        let mut direct = Arc::new(compiled).instantiate(ctx.clone());
        direct.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::from_member_path("direct"),
        ));
        let first = direct
            .evaluate(EvaluationRequest::new().at(now))
            .await
            .unwrap();
        assert!(has_named_scene_mark(&first.scene_graph.marks, "toy"));
        assert!(has_named_scene_mark(&first.scene_graph.marks, "body"));
        let first_attachment = &first.native_widgets.by_widget_id["toy"];
        assert!(first_attachment.scene_rebuilt);
        assert!(first_attachment.index_rebuilt);

        let second = direct
            .evaluate(
                EvaluationRequest::new().at(now + avenger_common::time::Duration::from_millis(10)),
            )
            .await
            .unwrap();
        let second_attachment = &second.native_widgets.by_widget_id["toy"];
        assert!(!second_attachment.scene_rebuilt);
        assert!(!second_attachment.index_rebuilt);
        // `body` is runtime-discovered, so the first evaluation renders once
        // to discover it and once more with its resolved part style.
        assert_eq!(second_attachment.scene_rebuilds, 2);

        let event = NativeWidgetEvent {
            event: SceneGraphEvent::WindowCloseRequested,
            hit_part: Some("body".to_string()),
            current: Some([3.0, 4.0]),
            start: Some([3.0, 4.0]),
            previous: None,
            wheel_delta: None,
            frame_size: [42.0, 24.0],
        };
        let route = NativeWidgetEventRoute {
            key: second_attachment.key.clone(),
            epoch: second_attachment.epoch,
        };
        let outcome = direct
            .dispatch_native_widget_event(
                &route,
                &event,
                NativeWidgetHostTransform::default(),
                now,
                12.0,
            )
            .unwrap();
        assert!(outcome.consume);
        assert_eq!(
            outcome.evaluation_intent,
            NativeWidgetEvaluationIntent::Exact
        );
        assert!(outcome.index_dirty);

        let after_event = direct
            .evaluate(EvaluationRequest::new().at(now))
            .await
            .unwrap();
        let after_event_attachment = &after_event.native_widgets.by_widget_id["toy"];
        assert!(!after_event_attachment.scene_rebuilt);
        assert!(after_event_attachment.index_rebuilt);

        let mut restored = Arc::new(decoded).instantiate(ctx);
        restored.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::from_member_path("restored"),
        ));
        let restored = restored
            .evaluate(EvaluationRequest::new().at(now))
            .await
            .unwrap();
        assert!(has_named_scene_mark(&restored.scene_graph.marks, "body"));
        assert_eq!(restored.native_widgets.by_widget_id.len(), 1);
    }

    #[tokio::test]
    async fn native_widget_cell_uses_generic_descriptor_and_reports_live_attachment() {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Chart::<HConcat>::new()
            .mark(WidgetCell::<HConcat>::native_widget(ToyNativeWidget).name("native-cell"))
            .compile(ctx.as_ref())
            .await
            .unwrap();
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(ToyNativeWidgetFactory)
                .unwrap(),
        );
        let resources = NativeWidgetRuntimeResources::new(
            registry,
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        );
        let mut session = Arc::new(compiled).instantiate(ctx);
        session.set_options(PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::from_member_path("widget-cell"),
        ));
        let evaluated = session.evaluate(EvaluationRequest::new()).await.unwrap();
        assert!(has_named_scene_mark(
            &evaluated.scene_graph.marks,
            "native-cell"
        ));
        assert!(has_named_scene_mark(&evaluated.scene_graph.marks, "body"));
        assert!(evaluated.native_widgets.by_widget_id.contains_key("toy"));
        assert!(evaluated.widget_frames.by_widget_id.contains_key("toy"));
    }

    #[test]
    fn paint_geometry_and_position_have_distinct_native_cache_invalidation() {
        let measurement_calls = Arc::new(AtomicU64::new(0));
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(CountingToyNativeWidgetFactory {
                    measurement_calls: measurement_calls.clone(),
                })
                .unwrap(),
        );
        let runtime = NativeWidgetEvaluationRuntime::new(
            registry,
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetNamespace::ephemeral(),
            false,
        );
        let params = Arc::new(IndexMap::new());
        let paint_a = Arc::new(ResolvedWidgetStyleSet {
            digest: 10,
            geometry_digest: 20,
            ..Default::default()
        });
        let paint_b = Arc::new(ResolvedWidgetStyleSet {
            digest: 11,
            geometry_digest: 20,
            ..Default::default()
        });
        let geometry_b = Arc::new(ResolvedWidgetStyleSet {
            digest: 12,
            geometry_digest: 21,
            ..Default::default()
        });
        for styles in [&paint_a, &paint_b, &geometry_b] {
            runtime
                .measure(
                    &toy_spec(1),
                    &NativeWidgetFactoryContext {
                        params: params.as_ref(),
                        styles,
                        base_font_size: 12.0,
                    },
                )
                .unwrap();
        }
        assert_eq!(measurement_calls.load(Ordering::Relaxed), 2);

        let environment = |styles| {
            NativeWidgetEnvironment::new(
                [42.0, 24.0],
                params.clone(),
                styles,
                WidgetPresentationState::default(),
                0,
                0,
                0,
            )
            .unwrap()
        };
        let state = IndexMap::new();
        let now = Instant::now();
        runtime.set_now(now);
        let first = runtime
            .sync_and_render(
                &toy_spec(1),
                &state,
                &environment(paint_a),
                NativeWidgetHostTransform::default(),
                now,
                12.0,
            )
            .unwrap();
        assert!(first.render.scene_rebuilt && first.render.index_rebuilt);

        let paint = runtime
            .sync_and_render(
                &toy_spec(1),
                &state,
                &environment(paint_b.clone()),
                NativeWidgetHostTransform::default(),
                now,
                12.0,
            )
            .unwrap();
        assert!(paint.render.scene_rebuilt);
        assert!(!paint.render.index_rebuilt);

        let moved = runtime
            .sync_and_render(
                &toy_spec(1),
                &state,
                &environment(paint_b),
                NativeWidgetHostTransform::from_offsets([[100.0, 50.0]]).unwrap(),
                now,
                12.0,
            )
            .unwrap();
        assert!(!moved.render.scene_rebuilt);
        assert!(!moved.render.index_rebuilt);

        let geometry = runtime
            .sync_and_render(
                &toy_spec(1),
                &state,
                &environment(geometry_b),
                NativeWidgetHostTransform::default(),
                now,
                12.0,
            )
            .unwrap();
        assert!(geometry.render.scene_rebuilt && geometry.render.index_rebuilt);
    }

    #[test]
    fn session_detach_and_final_eviction_use_host_clock_and_unmount_once() {
        let detach_calls = Arc::new(AtomicU64::new(0));
        let unmount_calls = Arc::new(AtomicU64::new(0));
        let callback_times = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(LifecycleNativeWidgetFactory {
                    detach_calls: detach_calls.clone(),
                    unmount_calls: unmount_calls.clone(),
                    callback_times: callback_times.clone(),
                })
                .unwrap(),
        );
        let store = Arc::new(InMemoryNativeWidgetInstanceStore::new());
        let resources = NativeWidgetRuntimeResources::new(
            registry.clone(),
            store.clone(),
            NativeWidgetDocumentId::new(),
        );
        let plot_id = NativeWidgetPlotId::from_member_path("lifecycle-test");
        let runtime = NativeWidgetEvaluationRuntime::new(
            registry,
            store,
            resources.namespace(plot_id.clone()),
            false,
        );
        let params = Arc::new(IndexMap::new());
        let styles = Arc::new(ResolvedWidgetStyleSet::default());
        let environment = NativeWidgetEnvironment::new(
            [10.0, 10.0],
            params,
            styles,
            WidgetPresentationState::default(),
            0,
            0,
            0,
        )
        .unwrap();
        let detach_time = Instant::now();
        runtime.set_now(detach_time);
        runtime
            .sync_and_render(
                &lifecycle_spec(),
                &IndexMap::new(),
                &environment,
                NativeWidgetHostTransform::default(),
                detach_time,
                12.0,
            )
            .unwrap();
        drop(runtime);
        assert_eq!(detach_calls.load(Ordering::Relaxed), 1);
        assert_eq!(unmount_calls.load(Ordering::Relaxed), 0);

        let unmount_time = detach_time + avenger_common::time::Duration::from_secs(3);
        resources.evict_plot(plot_id.clone(), unmount_time).unwrap();
        assert_eq!(detach_calls.load(Ordering::Relaxed), 1);
        assert_eq!(unmount_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            *callback_times.lock().unwrap(),
            vec![detach_time, unmount_time]
        );

        resources.evict_plot(plot_id, unmount_time).unwrap();
        assert_eq!(unmount_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn replacement_epoch_makes_old_runtime_teardown_inert() {
        let detach_calls = Arc::new(AtomicU64::new(0));
        let unmount_calls = Arc::new(AtomicU64::new(0));
        let callback_times = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(
            NativeWidgetRegistry::new()
                .with_factory(LifecycleNativeWidgetFactory {
                    detach_calls: detach_calls.clone(),
                    unmount_calls,
                    callback_times,
                })
                .unwrap(),
        );
        let store = Arc::new(InMemoryNativeWidgetInstanceStore::new());
        let namespace = NativeWidgetNamespace::ephemeral();
        let old = NativeWidgetEvaluationRuntime::new(
            registry.clone(),
            store.clone(),
            namespace.clone(),
            false,
        );
        let replacement = NativeWidgetEvaluationRuntime::new(registry, store, namespace, false);
        let environment = NativeWidgetEnvironment::new(
            [10.0, 10.0],
            Arc::new(IndexMap::new()),
            Arc::new(ResolvedWidgetStyleSet::default()),
            WidgetPresentationState::default(),
            0,
            0,
            0,
        )
        .unwrap();
        let old_output = old
            .sync_and_render(
                &lifecycle_spec(),
                &IndexMap::new(),
                &environment,
                NativeWidgetHostTransform::default(),
                Instant::now(),
                12.0,
            )
            .unwrap();
        let replacement_output = replacement
            .sync_and_render(
                &lifecycle_spec(),
                &IndexMap::new(),
                &environment,
                NativeWidgetHostTransform::default(),
                Instant::now(),
                12.0,
            )
            .unwrap();
        assert!(replacement_output.epoch.get() > old_output.epoch.get());

        drop(old);
        assert_eq!(detach_calls.load(Ordering::Relaxed), 0);
        let route = NativeWidgetEventRoute {
            key: replacement_output.key,
            epoch: replacement_output.epoch,
        };
        assert_eq!(replacement.rebuild_counts(&route), Some((1, 1)));

        drop(replacement);
        assert_eq!(detach_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn callback_context_reports_host_neutral_outcome_and_teed_commands() {
        let slot = InMemoryNativeWidgetInstanceStore::new().slot(NativeWidgetInstanceKey::new(
            NativeWidgetNamespace::ephemeral(),
            "editor",
        ));
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = NativeWidgetCtx::attach(
            slot,
            services,
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        ctx.assign_param(ScopedParamAssignment {
            name: "query".to_string(),
            owner_path: Vec::new(),
            value: ScalarValue::Utf8(Some("updated".to_string())),
            replace_scoped_values: false,
        });
        ctx.request_evaluation(NativeWidgetEvaluationIntent::Preview);
        ctx.request_evaluation(NativeWidgetEvaluationIntent::Exact);
        ctx.mark_scene_dirty();
        ctx.mark_index_dirty();
        ctx.set_cursor(CursorStyle::Text);
        ctx.focus(None, "selected");
        ctx.request_wakeup("commit", ctx.now(), 7);
        ctx.consume();

        let outcome = ctx.take_outcome();
        assert_eq!(outcome.param_assignments.len(), 1);
        assert_eq!(
            outcome.evaluation_intent,
            NativeWidgetEvaluationIntent::Exact
        );
        assert!(outcome.scene_dirty);
        assert!(outcome.index_dirty);
        assert_eq!(outcome.cursor, Some(CursorStyle::Text));
        assert!(outcome.consume);
        assert!(matches!(
            outcome.focus,
            Some(NativeWidgetFocusRequest::Focus { clipboard_payload, .. })
                if clipboard_payload == "selected"
        ));
        assert_eq!(outcome.commands, commands(&sink));
        assert!(matches!(
            outcome.commands.last(),
            Some(RuntimeHostCommand::RequestWakeup { generation: 7, .. })
        ));
        assert!(ctx.take_outcome().commands.is_empty());
    }

    #[test]
    fn dynamic_parts_keep_stable_paths_and_reject_mark_kind_changes() {
        let mut part_order = Vec::new();
        let mut part_kinds = HashMap::new();
        let manifests = IndexMap::new();

        let mut first = NativeWidgetScene::try_from_iter([(
            "choice".to_string(),
            SceneMark::Rect(SceneRectMark::default()),
        )])
        .unwrap();
        validate_native_widget_scene(
            "dynamic",
            &manifests,
            &mut part_order,
            &mut part_kinds,
            &mut first,
        )
        .unwrap();
        assert_eq!(part_order, ["choice"]);

        let mut second = NativeWidgetScene::try_from_iter([(
            "label".to_string(),
            SceneMark::Rect(SceneRectMark::default()),
        )])
        .unwrap();
        validate_native_widget_scene(
            "dynamic",
            &manifests,
            &mut part_order,
            &mut part_kinds,
            &mut second,
        )
        .unwrap();
        assert_eq!(part_order, ["choice", "label"]);
        assert!(matches!(
            second.parts.get("choice"),
            Some(SceneMark::Group(group)) if !group.interactive
        ));
        assert!(matches!(
            second.parts.get("label"),
            Some(SceneMark::Rect(_))
        ));

        let mut collision = NativeWidgetScene::try_from_iter([(
            "choice".to_string(),
            SceneMark::Group(Default::default()),
        )])
        .unwrap();
        let error = validate_native_widget_scene(
            "dynamic",
            &manifests,
            &mut part_order,
            &mut part_kinds,
            &mut collision,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AvengerChartError::InvalidArgument(message)
                if message.contains("changed part 'choice'")
                    && message.contains("'rect' to 'group'")
        ));
    }

    #[test]
    fn namespace_isolation_and_rebuilds_reuse_instance_identity() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let document = NativeWidgetDocumentId::new();
        let first_namespace = NativeWidgetNamespace::new(
            document,
            NativeWidgetPlotId::from_member_path("panel/first"),
        );
        let second_namespace = NativeWidgetNamespace::new(
            document,
            NativeWidgetPlotId::from_member_path("panel/second"),
        );

        let first = store.slot(NativeWidgetInstanceKey::new(
            first_namespace.clone(),
            "search",
        ));
        let rebuilt = store.slot(NativeWidgetInstanceKey::new(first_namespace, "search"));
        let isolated = store.slot(NativeWidgetInstanceKey::new(second_namespace, "search"));
        assert!(Arc::ptr_eq(&first, &rebuilt));
        assert!(!Arc::ptr_eq(&first, &isolated));
        assert_eq!(store.slot_count(), 2);
    }

    #[test]
    fn attach_new_then_detach_old_preserves_new_lease_and_instance() {
        let namespace = NativeWidgetNamespace::ephemeral();
        let slot = InMemoryNativeWidgetInstanceStore::new()
            .slot(NativeWidgetInstanceKey::new(namespace, "editor"));
        let editor = slot
            .get_or_init(|| Mutex::new(String::from("draft")))
            .unwrap();
        let old = slot.attach();
        let replacement = slot.attach();

        assert!(!slot.detach(old));
        assert!(slot.is_active(replacement));
        assert_eq!(*editor.lock().unwrap(), "draft");
        assert!(slot.detach(replacement));
    }

    #[test]
    fn headless_namespace_cleanup_drops_only_its_slots() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let keep = NativeWidgetNamespace::ephemeral();
        let temporary = NativeWidgetNamespace::ephemeral();
        store.slot(NativeWidgetInstanceKey::new(keep, "kept"));
        store.slot(NativeWidgetInstanceKey::new(temporary.clone(), "temporary"));
        store.remove_namespace(&temporary);
        assert_eq!(store.slot_count(), 1);
    }

    #[test]
    fn focus_switch_orders_ime_commands_and_translates_nested_caret() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let namespace = NativeWidgetNamespace::ephemeral();
        let first = store.slot(NativeWidgetInstanceKey::new(namespace.clone(), "first"));
        let second = store.slot(NativeWidgetInstanceKey::new(namespace, "second"));
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let transform =
            NativeWidgetHostTransform::from_offsets([[10.0, 20.0], [3.0, 4.0], [-1.0, 2.0]])
                .unwrap();
        let first_ctx = NativeWidgetCtx::attach(first, services.clone(), sink.clone(), transform);
        first_ctx.focus(
            Some(LogicalRect::new(5.0, 6.0, 1.0, 12.0).unwrap()),
            "first selection",
        );
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea {
                    rect: LogicalRect::new(17.0, 32.0, 1.0, 12.0),
                },
            ]
        );
        first_ctx.focus(
            Some(LogicalRect::new(5.0, 6.0, 1.0, 12.0).unwrap()),
            "first selection",
        );
        assert!(commands(&sink).is_empty(), "unchanged IME rect is cached");
        first_ctx.cancel_composition_and_refocus(
            Some(LogicalRect::new(7.0, 8.0, 1.0, 12.0).unwrap()),
            "moved selection",
        );
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: false },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea {
                    rect: LogicalRect::new(19.0, 34.0, 1.0, 12.0),
                },
            ]
        );

        let second_ctx = NativeWidgetCtx::attach(
            second,
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        second_ctx.focus(None, "second selection");
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: false },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
            ]
        );
        assert_eq!(
            services.focused_clipboard_payload().as_deref(),
            Some("second selection")
        );
        let routed = services
            .route_event(&SceneGraphEvent::Ime(
                avenger_eventstream::window::ImeEvent::Preedit {
                    text: "draft".into(),
                    cursor: Some((0, 5)),
                },
            ))
            .expect("focused IME route");
        assert_eq!(routed.key.widget_id(), "second");
        assert_eq!(routed.epoch, second_ctx.epoch());
    }

    #[test]
    fn replacement_lease_survives_old_drop_and_unfocused_wakes_route() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let slot = store.slot(NativeWidgetInstanceKey::new(
            NativeWidgetNamespace::ephemeral(),
            "editor",
        ));
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let old = NativeWidgetCtx::attach(
            slot.clone(),
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        old.focus(None, "draft");
        commands(&sink);

        let replacement = NativeWidgetCtx::attach(
            slot,
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        let replacement_epoch = replacement.epoch();
        drop(old);
        assert!(replacement.is_active());
        assert_eq!(
            services.focused_attachment().map(|(_, epoch)| epoch),
            Some(replacement_epoch)
        );
        assert!(commands(&sink).is_empty());

        replacement.blur();
        commands(&sink);
        let deadline = Instant::now();
        replacement.request_wakeup("commit", deadline, 9);
        let queued = commands(&sink);
        let [
            RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            },
        ] = queued.as_slice()
        else {
            panic!("expected one wake command")
        };
        let wake = RuntimeWakeEvent {
            key: key.clone(),
            generation: *generation,
        };
        assert!(replacement.accepts_wakeup(&wake, "commit"));
        assert!(replacement.accepts_wakeup_generation(&wake, "commit", 9));
        assert!(!replacement.accepts_wakeup_generation(&wake, "commit", 8));
        assert_eq!(
            services
                .route_event(&SceneGraphEvent::RuntimeWake(wake.clone()))
                .map(|route| route.epoch),
            Some(replacement_epoch)
        );

        let stale = RuntimeWakeEvent {
            key: RuntimeWakeKey::new(
                key.namespace.clone(),
                replacement_epoch.get().saturating_sub(1),
                key.purpose.clone(),
            ),
            generation: 9,
        };
        assert!(
            services
                .route_event(&SceneGraphEvent::RuntimeWake(stale))
                .is_none()
        );
    }

    #[test]
    fn plot_session_option_equality_uses_resource_identity_and_namespace_value() {
        let resources = NativeWidgetRuntimeResources::in_memory();
        let options = PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::chart_root(),
        );
        assert_eq!(options, options.clone());

        let mut changed_registry = options.clone();
        changed_registry.native_widget_registry = Arc::new(NativeWidgetRegistry::new());
        assert_ne!(options, changed_registry);

        let mut changed_store = options.clone();
        changed_store.native_widget_instance_store =
            Arc::new(InMemoryNativeWidgetInstanceStore::new());
        assert_ne!(options, changed_store);

        let mut changed_namespace = options.clone();
        changed_namespace.native_widget_namespace =
            resources.namespace(NativeWidgetPlotId::from_member_path("another-member"));
        assert_ne!(options, changed_namespace);

        let debug = format!("{options:?}");
        assert!(debug.contains("opaque@"));
        assert!(!debug.contains("slots"));

        let temporary_a = PlotSessionOptions::default();
        let temporary_b = PlotSessionOptions::default();
        assert_ne!(
            temporary_a.native_widget_namespace, temporary_b.native_widget_namespace,
            "headless/default sessions require temporary document ownership"
        );
    }

    #[tokio::test]
    async fn plot_session_equal_options_are_noop_and_each_identity_change_revises_attachment() {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Chart::<ZeroDCoord>::new()
            .compile(ctx.as_ref())
            .await
            .unwrap();
        let mut session = Arc::new(compiled).instantiate(ctx);
        let resources = NativeWidgetRuntimeResources::in_memory();
        let base = PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::chart_root(),
        );

        session.set_options(base.clone());
        let revision = session.native_widget_attachment_revision();
        assert_eq!(session.options(), &base);
        session.set_options(base.clone());
        assert_eq!(session.native_widget_attachment_revision(), revision);

        let mut registry = base.clone();
        registry.native_widget_registry = Arc::new(NativeWidgetRegistry::new());
        session.set_options(registry);
        assert_eq!(session.native_widget_attachment_revision(), revision + 1);

        let mut store = base.clone();
        store.native_widget_instance_store = Arc::new(InMemoryNativeWidgetInstanceStore::new());
        session.set_options(store);
        assert_eq!(session.native_widget_attachment_revision(), revision + 2);

        let mut namespace = base;
        namespace.native_widget_namespace = resources.namespace(
            NativeWidgetPlotId::from_member_path("panel/nested-widget-cell"),
        );
        session.set_options(namespace);
        assert_eq!(session.native_widget_attachment_revision(), revision + 3);
    }
}
