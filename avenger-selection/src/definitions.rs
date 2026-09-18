use crate::{Error, PixelGrid, ProducerAddress, ProjectionId, Result, SelectionId};
use datafusion::{
    arrow::datatypes::DataType,
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    logical_expr::{Expr, Volatility},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

/// How active producer contributions combine for a named selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Global,
    Union,
    Intersect,
}

/// A shared name and its producer-combination policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionDefinition {
    id: SelectionId,
    resolution: Resolution,
}
impl SelectionDefinition {
    /// Define one shared name, independently of its producers' projections.
    pub fn new(id: SelectionId, resolution: Resolution) -> Self {
        Self { id, resolution }
    }
    /// Return the shared name.
    pub fn id(&self) -> &SelectionId {
        &self.id
    }
    /// Return the combination policy.
    pub fn resolution(&self) -> Resolution {
        self.resolution
    }
}

/// Interaction kind, independent of equality, set, or range comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Point,
    Interval,
}

/// Comparison precision captured by a producer definition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IntervalPrecision {
    Exact,
    Pixels { size: f64 },
}

/// A checked deterministic row expression with a producer-local identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    id: ProjectionId,
    expr: Expr,
    literal_types: Vec<DataType>,
}
impl Projection {
    /// Validate a row-local expression without requiring a consuming schema.
    pub fn new(id: ProjectionId, expr: Expr) -> Result<Self> {
        let expr = row_expr(expr)?;
        // ScalarValue equality ignores timestamp zones. Literal types remain
        // part of projection identity for configuration checks and global toggles.
        let mut literal_types = Vec::new();
        expr.apply(|node| {
            if let Expr::Literal(value, _) = node {
                literal_types.push(value.data_type());
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        Ok(Self {
            id,
            expr,
            literal_types,
        })
    }
    /// Return the producer-local identity.
    pub fn id(&self) -> &ProjectionId {
        &self.id
    }
    /// Return the projected expression without output aliases.
    pub fn expr(&self) -> &Expr {
        &self.expr
    }
    pub(crate) fn same_meaning(&self, other: &Self) -> bool {
        self.expr == other.expr && self.literal_types == other.literal_types
    }
}

/// An opaque lineage and exact type for identity-preserving row IDs.
/// Clone this descriptor when filtering or renaming IDs preserves identity.
/// Create a fresh descriptor when IDs are regenerated or come from another relation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowIdentity {
    lineage: u64,
    data_type: DataType,
}
impl RowIdentity {
    /// Allocate a fresh lineage token with an explicitly typed ID domain.
    pub fn new(data_type: DataType) -> Result<Self> {
        crate::values::validate_type(&data_type)?;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let lineage = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("row identity space exhausted");
        Ok(Self { lineage, data_type })
    }
    /// Return the type of every ID in this lineage.
    pub fn data_type(&self) -> &DataType {
        &self.data_type
    }
}

/// Immutable semantics captured by each active producer contribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerDefinition {
    address: ProducerAddress,
    kind: SelectionKind,
    projections: Vec<Projection>,
    identity: Option<RowIdentity>,
    grids: BTreeMap<ProjectionId, PixelGrid>,
}
impl ProducerDefinition {
    /// Define a producer with unique, nonempty projected dimensions and exact precision.
    pub fn new(
        address: ProducerAddress,
        kind: SelectionKind,
        mut projections: Vec<Projection>,
    ) -> Result<Self> {
        if projections.is_empty() {
            return Err(Error::InvalidDefinition(
                "tuple producers need at least one projection".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for projection in &projections {
            if !ids.insert(projection.id()) {
                return Err(Error::InvalidDefinition(format!(
                    "duplicate projection {}",
                    projection.id()
                )));
            }
        }
        projections.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self {
            address,
            kind,
            projections,
            identity: None,
            grids: BTreeMap::new(),
        })
    }
    /// Define point selection by row IDs instead of projected tuples.
    pub fn row_ids(address: ProducerAddress, identity: RowIdentity) -> Self {
        Self {
            address,
            kind: SelectionKind::Point,
            projections: vec![],
            identity: Some(identity),
            grids: BTreeMap::new(),
        }
    }
    /// Return the selection, declaration, and view instance.
    pub fn address(&self) -> &ProducerAddress {
        &self.address
    }
    /// Return the interaction kind.
    pub fn kind(&self) -> SelectionKind {
        self.kind
    }
    /// Return projections in canonical local-ID order.
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }
    /// Return the required row lineage for an identity producer.
    pub fn identity(&self) -> Option<&RowIdentity> {
        self.identity.as_ref()
    }
    /// Return the membership precision captured by this definition.
    pub fn precision(&self) -> IntervalPrecision {
        match self.grids.values().next() {
            Some(grid) => IntervalPrecision::Pixels { size: grid.size() },
            None => IntervalPrecision::Exact,
        }
    }
    /// Capture a replacement set of pixel grids without modifying this definition.
    /// All grids must refer to declared projections and use the same pixel size.
    /// Only interval producers accept grids. Reapply retained raw values with `set`
    /// to change an existing contribution's precision after a resize.
    pub fn with_pixel_grids(
        &self,
        grids: impl IntoIterator<Item = (ProjectionId, PixelGrid)>,
    ) -> Result<Self> {
        if self.kind != SelectionKind::Interval {
            return Err(Error::InvalidDefinition(
                "pixel grids require an interval producer".into(),
            ));
        }
        let mut result = self.clone();
        result.grids.clear();
        for (id, grid) in grids {
            if !self.projections.iter().any(|p| p.id() == &id) {
                return Err(Error::InvalidDefinition(format!(
                    "unknown pixel projection {id}"
                )));
            }
            if result
                .grids
                .values()
                .any(|other| other.size() != grid.size())
            {
                return Err(Error::InvalidDefinition(
                    "pixel grids must have the same cell size".into(),
                ));
            }
            if result.grids.insert(id.clone(), grid).is_some() {
                return Err(Error::InvalidDefinition(format!(
                    "duplicate pixel grid for {id}"
                )));
            }
        }
        if result.grids.is_empty() {
            return Err(Error::InvalidDefinition(
                "pixel precision requires at least one grid".into(),
            ));
        }
        Ok(result)
    }
    /// Return the captured grid for a projected dimension, if any.
    pub fn pixel_grid(&self, id: &ProjectionId) -> Option<&PixelGrid> {
        self.grids.get(id)
    }
    /// Inspect captured grids in canonical projection order.
    pub fn pixel_grids(&self) -> impl Iterator<Item = (&ProjectionId, &PixelGrid)> {
        self.grids.iter()
    }
}

pub(crate) fn row_expr(expr: Expr) -> Result<Expr> {
    #[allow(deprecated)]
    expr.apply(|node| {
        match node {
            Expr::ScalarFunction(f) if f.func.signature().volatility != Volatility::Immutable => {
                return datafusion::common::plan_err!(
                    "selection projection function {} must be immutable",
                    f.func.name()
                )
            }
            Expr::Alias(_)
            | Expr::Column(_)
            | Expr::Literal(..)
            | Expr::BinaryExpr(_)
            | Expr::Like(_)
            | Expr::SimilarTo(_)
            | Expr::Not(_)
            | Expr::IsNotNull(_)
            | Expr::IsNull(_)
            | Expr::IsTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotTrue(_)
            | Expr::IsNotFalse(_)
            | Expr::IsNotUnknown(_)
            | Expr::Negative(_)
            | Expr::Between(_)
            | Expr::Case(_)
            | Expr::Cast(_)
            | Expr::TryCast(_)
            | Expr::ScalarFunction(_)
            | Expr::InList(_) => {}
            _ => {
                return datafusion::common::plan_err!(
                    "selection projection must be row-local: {node}"
                )
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(expr
        .transform_up(|node| {
            Ok(match node {
                Expr::Alias(alias) => Transformed::yes(*alias.expr),
                node => Transformed::no(node),
            })
        })?
        .data)
}

pub(crate) type Producer = Arc<ProducerDefinition>;
