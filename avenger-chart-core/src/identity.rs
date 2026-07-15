//! Opaque identities used by compiled chart artifacts.
//!
//! Author-facing names are retained separately for diagnostics and host
//! bindings. Runtime lookup uses these identities and must not reconstruct a
//! target by parsing either [`Display`](std::fmt::Display) output or a public
//! export path.

use std::{collections::BTreeMap, fmt, hash::Hash, ops::Index};

use indexmap::IndexMap;

use serde::{Deserialize, Serialize};

macro_rules! opaque_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            fn from_allocated(value: String) -> Self {
                Self(value)
            }

            /// Return the opaque serialized value used by compiled artifacts.
            ///
            /// This is not an authoring path and must not be parsed to recover
            /// source-level meaning.
            pub fn as_opaque_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(
    StateId,
    "Shared opaque identity substrate for compiled state."
);
opaque_id!(MarkId, "Opaque identity for one compiled mark.");

impl MarkId {
    pub(crate) fn unresolved_authoring() -> Self {
        Self::from_allocated("mark:unresolved".to_string())
    }

    /// Whether this identity is the authoring-stage sentinel that must be
    /// replaced at a compiled-artifact boundary.
    #[doc(hidden)]
    pub fn is_unresolved(&self) -> bool {
        self.0 == "mark:unresolved"
    }
}

impl Default for MarkId {
    fn default() -> Self {
        Self::unresolved_authoring()
    }
}
opaque_id!(
    ViewId,
    "Opaque identity for one compiled inline view scope."
);

impl ViewId {
    pub(crate) fn unresolved_authoring() -> Self {
        Self::from_allocated("view:unresolved".to_string())
    }

    /// Whether this identity is the authoring-stage sentinel that must be
    /// replaced at a compiled-artifact boundary.
    #[doc(hidden)]
    pub fn is_unresolved(&self) -> bool {
        self.0 == "view:unresolved"
    }
}

impl Default for ViewId {
    fn default() -> Self {
        Self::unresolved_authoring()
    }
}
opaque_id!(
    ToolInstanceId,
    "Opaque identity for one expanded tool instance."
);
opaque_id!(
    WidgetInstanceId,
    "Opaque identity for one composed or native widget instance."
);
opaque_id!(
    StateMigrationKey,
    "Stable host-facing state migration metadata, distinct from runtime identity."
);

macro_rules! state_ref {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(StateId);

        impl $name {
            fn from_allocated(id: StateId) -> Self {
                Self(id)
            }

            pub(crate) fn unresolved_authoring() -> Self {
                Self(StateId::from_allocated(format!(
                    "state:unresolved:{}",
                    stringify!($name)
                )))
            }

            /// Whether this identity is the authoring-stage sentinel that must
            /// be replaced at the root compilation boundary.
            #[doc(hidden)]
            pub fn is_unresolved(&self) -> bool {
                self.0.as_opaque_str().starts_with("state:unresolved:")
            }

            pub fn state_id(&self) -> &StateId {
                &self.0
            }

            pub fn as_opaque_str(&self) -> &str {
                self.0.as_opaque_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

state_ref!(ParamRef, "Typed runtime identity for a compiled parameter.");
state_ref!(StoreRef, "Typed runtime identity for a compiled store.");
state_ref!(
    SelectionRef,
    "Typed runtime identity for a compiled selection."
);

/// A resolved runtime target paired with its diagnostic source name.
///
/// Equality includes both fields so compiled-artifact comparisons retain
/// diagnostic metadata, while runtime registries use only `id` as their key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedStateTarget<I> {
    pub id: I,
    pub source_name: String,
}

/// A compiled state registry with opaque runtime identity as its only spec key.
///
/// `source_name_index` is deliberately a separate author/host adapter. It maps
/// diagnostic source names to opaque identities, but never owns or duplicates
/// state specs. Runtime code should retain the typed identity returned by
/// [`Self::resolve_source_name`] and use [`Self::get_by_id`] thereafter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "I: Serialize + Eq + Hash, S: Serialize",
    deserialize = "I: Deserialize<'de> + Eq + Hash, S: Deserialize<'de>"
))]
pub struct CompiledStateRegistry<I: Eq + Hash, S> {
    specs_by_id: IndexMap<I, S>,
    source_name_index: IndexMap<String, I>,
}

impl<I: Eq + Hash, S> Default for CompiledStateRegistry<I, S> {
    fn default() -> Self {
        Self {
            specs_by_id: IndexMap::new(),
            source_name_index: IndexMap::new(),
        }
    }
}

impl<I, S> CompiledStateRegistry<I, S>
where
    I: Clone + Eq + Hash,
{
    /// Build a registry from declaration-ordered specs.
    ///
    /// The callbacks keep this container independent of individual state spec
    /// types while enforcing the invariant that both runtime IDs and source
    /// names are unique.
    pub fn try_from_specs(
        specs: impl IntoIterator<Item = S>,
        mut id: impl FnMut(&S) -> &I,
        mut source_name: impl FnMut(&S) -> &str,
    ) -> Result<Self, CompiledStateRegistryError> {
        let mut registry = Self::default();
        for spec in specs {
            let runtime_id = id(&spec).clone();
            let source_name = source_name(&spec).to_string();
            if registry.specs_by_id.contains_key(&runtime_id) {
                return Err(CompiledStateRegistryError::DuplicateRuntimeId);
            }
            if registry.source_name_index.contains_key(&source_name) {
                return Err(CompiledStateRegistryError::DuplicateSourceName { source_name });
            }
            registry
                .source_name_index
                .insert(source_name, runtime_id.clone());
            registry.specs_by_id.insert(runtime_id, spec);
        }
        Ok(registry)
    }

    /// Canonical runtime registry, keyed only by opaque identity.
    pub fn by_id(&self) -> &IndexMap<I, S> {
        &self.specs_by_id
    }

    pub fn get_by_id(&self, id: &I) -> Option<&S> {
        self.specs_by_id.get(id)
    }

    /// Resolve an author/host-facing source name at a boundary.
    pub fn resolve_source_name(&self, source_name: &str) -> Option<&I> {
        self.source_name_index.get(source_name)
    }

    /// Convenience boundary adapter that resolves a source name and returns its
    /// spec. Runtime code should prefer `resolve_source_name` plus `get_by_id`.
    pub fn get(&self, source_name: &str) -> Option<&S> {
        self.resolve_source_name(source_name)
            .and_then(|id| self.get_by_id(id))
    }

    pub fn contains_key(&self, source_name: &str) -> bool {
        self.source_name_index.contains_key(source_name)
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &String> {
        self.source_name_index.keys()
    }

    pub fn values(&self) -> impl ExactSizeIterator<Item = &S> {
        self.source_name_index
            .values()
            .map(|id| &self.specs_by_id[id])
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&String, &S)> {
        self.source_name_index
            .iter()
            .map(|(name, id)| (name, &self.specs_by_id[id]))
    }

    pub fn len(&self) -> usize {
        self.specs_by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs_by_id.is_empty()
    }
}

impl<I, S> Index<&str> for CompiledStateRegistry<I, S>
where
    I: Clone + Eq + Hash,
{
    type Output = S;

    fn index(&self, source_name: &str) -> &Self::Output {
        self.get(source_name)
            .unwrap_or_else(|| panic!("unknown compiled state source name '{source_name}'"))
    }
}

impl<'a, I, S> IntoIterator for &'a CompiledStateRegistry<I, S>
where
    I: Clone + Eq + Hash,
{
    type Item = (&'a String, &'a S);
    type IntoIter = Box<dyn ExactSizeIterator<Item = Self::Item> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompiledStateRegistryError {
    #[error("duplicate opaque runtime state identity")]
    DuplicateRuntimeId,
    #[error("duplicate compiled state source name '{source_name}'")]
    DuplicateSourceName { source_name: String },
}

impl<I> ResolvedStateTarget<I> {
    pub fn new(id: I, source_name: impl Into<String>) -> Self {
        Self {
            id,
            source_name: source_name.into(),
        }
    }
}

/// The state namespace of an authoring symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateSymbolKind {
    Param,
    Store,
    Selection,
}

/// A typed state identity resolved from an author-facing symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateSymbol {
    Param(ParamRef),
    Store(StoreRef),
    Selection(SelectionRef),
}

impl StateSymbol {
    pub fn kind(&self) -> StateSymbolKind {
        match self {
            Self::Param(_) => StateSymbolKind::Param,
            Self::Store(_) => StateSymbolKind::Store,
            Self::Selection(_) => StateSymbolKind::Selection,
        }
    }
}

/// One lexical scope's author-facing state symbol table.
///
/// Params, stores, and selections deliberately share this table so a name
/// conflict is diagnosed before expressions are lowered. Distinct lexical
/// scopes own distinct tables.
#[derive(Clone, Debug, Default)]
pub struct StateSymbolTable {
    symbols: BTreeMap<String, StateSymbol>,
}

impl StateSymbolTable {
    pub fn insert(
        &mut self,
        source_name: impl Into<String>,
        symbol: StateSymbol,
    ) -> Result<(), StateSymbolConflict> {
        let source_name = source_name.into();
        if let Some(existing) = self.symbols.get(&source_name) {
            return Err(StateSymbolConflict {
                source_name,
                existing: existing.kind(),
                attempted: symbol.kind(),
            });
        }
        self.symbols.insert(source_name, symbol);
        Ok(())
    }

    pub fn get(&self, source_name: &str) -> Option<&StateSymbol> {
        self.symbols.get(source_name)
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

/// A same-scope state-name collision.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "state name '{source_name}' is already declared as {existing:?}; cannot redeclare it as {attempted:?}"
)]
pub struct StateSymbolConflict {
    pub source_name: String,
    pub existing: StateSymbolKind,
    pub attempted: StateSymbolKind,
}

/// Deterministic allocator for opaque compiled identities.
///
/// Allocation is deterministic for a stable seed and traversal order. Source
/// names and public aliases are intentionally not allocator inputs. A future
/// language compiler may supply a stable component-instance seed while using
/// the same allocation contract.
#[derive(Clone, Debug)]
pub struct CompiledIdentityAllocator {
    seed: String,
    state_ordinal: u64,
    mark_ordinal: u64,
    view_ordinal: u64,
    tool_ordinal: u64,
    widget_ordinal: u64,
}

impl Default for CompiledIdentityAllocator {
    fn default() -> Self {
        Self::new("root")
    }
}

impl CompiledIdentityAllocator {
    pub fn new(seed: impl Into<String>) -> Self {
        Self {
            seed: encode_seed(&seed.into()),
            state_ordinal: 0,
            mark_ordinal: 0,
            view_ordinal: 0,
            tool_ordinal: 0,
            widget_ordinal: 0,
        }
    }

    pub fn allocate_param(&mut self) -> ParamRef {
        ParamRef::from_allocated(self.allocate_state("param"))
    }

    pub fn allocate_store(&mut self) -> StoreRef {
        StoreRef::from_allocated(self.allocate_state("store"))
    }

    pub fn allocate_selection(&mut self) -> SelectionRef {
        SelectionRef::from_allocated(self.allocate_state("selection"))
    }

    pub fn allocate_mark(&mut self) -> MarkId {
        let ordinal = take_ordinal(&mut self.mark_ordinal);
        MarkId::from_allocated(format!("mark:{}:{ordinal:016x}", self.seed))
    }

    pub fn allocate_view(&mut self) -> ViewId {
        let ordinal = take_ordinal(&mut self.view_ordinal);
        ViewId::from_allocated(format!("view:{}:{ordinal:016x}", self.seed))
    }

    pub fn allocate_tool_instance(&mut self) -> ToolInstanceId {
        let ordinal = take_ordinal(&mut self.tool_ordinal);
        ToolInstanceId::from_allocated(format!("tool:{}:{ordinal:016x}", self.seed))
    }

    pub fn allocate_widget_instance(&mut self) -> WidgetInstanceId {
        let ordinal = take_ordinal(&mut self.widget_ordinal);
        WidgetInstanceId::from_allocated(format!("widget:{}:{ordinal:016x}", self.seed))
    }

    /// Allocate generated state owned by a tool instance.
    pub fn allocate_tool_param(&self, owner: &ToolInstanceId, ordinal: u64) -> ParamRef {
        Self::derive_tool_param(owner, ordinal)
    }

    pub fn derive_tool_param(owner: &ToolInstanceId, ordinal: u64) -> ParamRef {
        ParamRef::from_allocated(StateId::from_allocated(format!(
            "state:{}:tool-param:{ordinal:016x}",
            owner.as_opaque_str()
        )))
    }

    pub fn derive_tool_store(owner: &ToolInstanceId, ordinal: u64) -> StoreRef {
        StoreRef::from_allocated(StateId::from_allocated(format!(
            "state:{}:tool-store:{ordinal:016x}",
            owner.as_opaque_str()
        )))
    }

    pub fn derive_tool_selection(owner: &ToolInstanceId, ordinal: u64) -> SelectionRef {
        SelectionRef::from_allocated(StateId::from_allocated(format!(
            "state:{}:tool-selection:{ordinal:016x}",
            owner.as_opaque_str()
        )))
    }

    pub fn derive_tool_mark(owner: &ToolInstanceId, ordinal: u64) -> MarkId {
        MarkId::from_allocated(format!(
            "mark:{}:tool-mark:{ordinal:016x}",
            owner.as_opaque_str()
        ))
    }

    pub fn derive_widget_param(owner: &WidgetInstanceId, ordinal: u64) -> ParamRef {
        ParamRef::from_allocated(StateId::from_allocated(format!(
            "state:{}:widget-param:{ordinal:016x}",
            owner.as_opaque_str()
        )))
    }

    /// Derive the behavior identity owned by a widget instance.
    pub fn widget_behavior_instance(&self, owner: &WidgetInstanceId) -> ToolInstanceId {
        ToolInstanceId::from_allocated(format!("tool:{}:behavior", owner.as_opaque_str()))
    }

    /// Construct migration metadata. This constructor is intentionally
    /// separate from every runtime-ID allocator.
    pub fn migration_key(&self, stable_source_identity: impl AsRef<str>) -> StateMigrationKey {
        StateMigrationKey::from_allocated(format!(
            "migration:{}:{}",
            self.seed,
            encode_seed(stable_source_identity.as_ref())
        ))
    }

    fn allocate_state(&mut self, kind: &str) -> StateId {
        let ordinal = take_ordinal(&mut self.state_ordinal);
        StateId::from_allocated(format!("state:{}:{kind}:{ordinal:016x}", self.seed))
    }
}

fn take_ordinal(next: &mut u64) -> u64 {
    let current = *next;
    *next = next
        .checked_add(1)
        .expect("compiled identity ordinal exhausted");
    current
}

fn encode_seed(seed: &str) -> String {
    // Length-prefixing prevents delimiter ambiguity while keeping identities
    // inspectable in explicit debug output. The result remains opaque to all
    // source-name and public-path resolution.
    format!("{:x}-{}", seed.len(), seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestParamSpec {
        id: ParamRef,
        name: String,
    }

    #[test]
    fn deterministic_allocators_reproduce_structure() {
        let mut left = CompiledIdentityAllocator::new("chart");
        let mut right = CompiledIdentityAllocator::new("chart");

        assert_eq!(left.allocate_param(), right.allocate_param());
        assert_eq!(left.allocate_store(), right.allocate_store());
        assert_eq!(left.allocate_selection(), right.allocate_selection());
        assert_eq!(left.allocate_mark(), right.allocate_mark());
        assert_eq!(left.allocate_view(), right.allocate_view());
        assert_eq!(
            left.allocate_tool_instance(),
            right.allocate_tool_instance()
        );
        assert_eq!(
            left.allocate_widget_instance(),
            right.allocate_widget_instance()
        );
    }

    #[test]
    fn same_source_name_in_distinct_scopes_has_distinct_identity() {
        let mut allocator = CompiledIdentityAllocator::default();
        let mut left_scope = StateSymbolTable::default();
        let mut right_scope = StateSymbolTable::default();
        let left = allocator.allocate_param();
        let right = allocator.allocate_param();

        left_scope
            .insert("value", StateSymbol::Param(left.clone()))
            .unwrap();
        right_scope
            .insert("value", StateSymbol::Param(right.clone()))
            .unwrap();

        assert_ne!(left, right);
    }

    #[test]
    fn params_and_stores_conflict_in_one_scope() {
        let mut allocator = CompiledIdentityAllocator::default();
        let mut scope = StateSymbolTable::default();
        scope
            .insert("value", StateSymbol::Param(allocator.allocate_param()))
            .unwrap();

        let error = scope
            .insert("value", StateSymbol::Store(allocator.allocate_store()))
            .unwrap_err();
        assert_eq!(error.existing, StateSymbolKind::Param);
        assert_eq!(error.attempted, StateSymbolKind::Store);
    }

    #[test]
    fn tool_instances_own_distinct_generated_state() {
        let mut allocator = CompiledIdentityAllocator::default();
        let left = allocator.allocate_tool_instance();
        let right = allocator.allocate_tool_instance();

        assert_ne!(
            allocator.allocate_tool_param(&left, 0),
            allocator.allocate_tool_param(&right, 0)
        );
    }

    #[test]
    fn widget_instances_own_distinct_behavior_and_generated_state() {
        let mut allocator = CompiledIdentityAllocator::default();
        let left_widget = allocator.allocate_widget_instance();
        let right_widget = allocator.allocate_widget_instance();
        let left_behavior = allocator.widget_behavior_instance(&left_widget);
        let right_behavior = allocator.widget_behavior_instance(&right_widget);

        assert_ne!(left_widget, right_widget);
        assert_ne!(left_behavior, right_behavior);
        assert_ne!(
            allocator.allocate_tool_param(&left_behavior, 0),
            allocator.allocate_tool_param(&right_behavior, 0)
        );
    }

    #[test]
    fn migration_keys_are_not_runtime_ids() {
        let mut allocator = CompiledIdentityAllocator::default();
        let state = allocator.allocate_param();
        let migration = allocator.migration_key("component/param:value");

        assert_ne!(state.as_opaque_str(), migration.as_opaque_str());
    }

    #[test]
    fn identities_round_trip_without_source_names() {
        let mut allocator = CompiledIdentityAllocator::new("chart");
        let identity = allocator.allocate_param();
        let encoded = bincode::serialize(&identity).unwrap();
        let decoded: ParamRef = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded, identity);
        assert!(!identity.as_opaque_str().contains("author-facing-name"));
    }

    #[test]
    fn compiled_registry_serializes_specs_once_and_resolves_names_at_boundaries() {
        let mut allocator = CompiledIdentityAllocator::new("registry");
        let first = TestParamSpec {
            id: allocator.allocate_param(),
            name: "first".to_string(),
        };
        let second = TestParamSpec {
            id: allocator.allocate_param(),
            name: "second".to_string(),
        };
        let registry = CompiledStateRegistry::try_from_specs(
            [first.clone(), second.clone()],
            |spec| &spec.id,
            |spec| spec.name.as_str(),
        )
        .unwrap();

        assert_eq!(registry.get("first"), Some(&first));
        assert_eq!(registry.get_by_id(&second.id), Some(&second));
        assert_eq!(registry.resolve_source_name("second"), Some(&second.id));

        let encoded = bincode::serialize(&registry).unwrap();
        let decoded: CompiledStateRegistry<ParamRef, TestParamSpec> =
            bincode::deserialize(&encoded).unwrap();
        assert_eq!(decoded, registry);
    }
}
