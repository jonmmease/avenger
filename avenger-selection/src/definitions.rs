use crate::{
    identity::ProducerAddress, Error, PixelGrid, ProducerId, ProjectionId, Result, SelectionId,
    ViewId,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    logical_expr::{Expr, Volatility},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// How active producer contributions combine for a named selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Global,
    Union,
    Intersect,
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

/// Immutable semantics captured by each active producer contribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerDefinition {
    address: ProducerAddress,
    projections: Vec<Projection>,
    grids: BTreeMap<ProjectionId, PixelGrid>,
}
impl ProducerDefinition {
    /// Define a producer with unique, nonempty projected dimensions and exact precision.
    pub fn new(
        selection: SelectionId,
        id: ProducerId,
        view: ViewId,
        projections: impl IntoIterator<Item = Projection>,
    ) -> Result<Self> {
        let mut projections: Vec<_> = projections.into_iter().collect();
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
            address: ProducerAddress {
                selection,
                producer: id,
                origin: view,
            },
            projections,
            grids: BTreeMap::new(),
        })
    }
    /// Return the shared selection receiving this producer's updates.
    pub fn selection(&self) -> &SelectionId {
        &self.address.selection
    }
    /// Return the declaration identity within its selection and view instance.
    pub fn id(&self) -> &ProducerId {
        &self.address.producer
    }
    /// Return the opaque view instance used for self-filter exclusion.
    pub fn view(&self) -> &ViewId {
        &self.address.origin
    }
    pub(crate) fn address(&self) -> &ProducerAddress {
        &self.address
    }
    /// Return projections in canonical local-ID order.
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }
    /// Capture a replacement set of pixel grids without modifying this definition.
    /// Each grid refers to a declared projection and can use its own pixel size.
    /// Projections without grids keep exact comparisons. An empty set removes all grids.
    /// Reapply retained raw values with `set` to update membership after a resize.
    pub fn with_pixel_grids(
        &self,
        grids: impl IntoIterator<Item = (ProjectionId, PixelGrid)>,
    ) -> Result<Self> {
        let mut result = self.clone();
        result.grids.clear();
        for (id, grid) in grids {
            if !self.projections.iter().any(|p| p.id() == &id) {
                return Err(Error::InvalidDefinition(format!(
                    "unknown pixel projection {id}"
                )));
            }
            if result.grids.insert(id.clone(), grid).is_some() {
                return Err(Error::InvalidDefinition(format!(
                    "duplicate pixel grid for {id}"
                )));
            }
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
