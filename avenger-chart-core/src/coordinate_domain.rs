use std::{
    collections::{HashMap, VecDeque},
    fmt::Debug,
    hash::Hash,
};

use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    AvengerChartError, CoordinationScope, DomainBounds, DomainExtent, Param, union_domain_extents,
};

const DOMAIN_SPAN_EPSILON: f64 = 1e-9;

#[derive(Clone, Debug)]
pub struct CoordinateDomainDescriptor {
    pub id: String,
    pub bindings: Vec<CoordinateDomainBinding>,
    pub params: Vec<CoordinateDomainParamSpec>,
    pub metrics: Vec<CoordinateMetricDescriptor>,
    pub sharing_policy: CoordinateDomainSharingPolicy,
    pub depends_on_plot_area: bool,
}

impl CoordinateDomainDescriptor {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            bindings: Vec::new(),
            params: Vec::new(),
            metrics: Vec::new(),
            sharing_policy: CoordinateDomainSharingPolicy::LocalOnly,
            depends_on_plot_area: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateDomainBinding {
    pub coord_channel: String,
    pub scale_name: Option<String>,
    pub role: CoordinateDomainRole,
    pub ownership: CoordinateDomainOwnership,
    pub materialize: CoordinateDomainMaterialization,
    pub required_scale_type: Option<CoordinateDomainScaleType>,
}

impl CoordinateDomainBinding {
    pub fn observed(coord_channel: impl Into<String>, role: CoordinateDomainRole) -> Self {
        Self {
            coord_channel: coord_channel.into(),
            scale_name: None,
            role,
            ownership: CoordinateDomainOwnership::ObserveAndMayOverride,
            materialize: CoordinateDomainMaterialization::ExistingOnly,
            required_scale_type: None,
        }
    }

    pub fn owned(
        coord_channel: impl Into<String>,
        role: CoordinateDomainRole,
        materialize: CoordinateDomainMaterialization,
    ) -> Self {
        Self {
            coord_channel: coord_channel.into(),
            scale_name: None,
            role,
            ownership: CoordinateDomainOwnership::OwnsFinalDomain,
            materialize,
            required_scale_type: None,
        }
    }

    pub fn with_scale_name(mut self, scale_name: impl Into<String>) -> Self {
        self.scale_name = Some(scale_name.into());
        self
    }

    pub fn requiring_scale_type(mut self, scale_type: CoordinateDomainScaleType) -> Self {
        self.required_scale_type = Some(scale_type);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainRole {
    X,
    Y,
    Named(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainOwnership {
    ObserveAndMayOverride,
    OwnsFinalDomain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainMaterialization {
    ExistingOnly,
    CreateIfAbsent {
        scale_type: CoordinateDomainScaleType,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainScaleType {
    LinearNumeric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainSharingPolicy {
    LocalOnly,
    FacetRepeatGroups,
    AnyCompatibleGroup,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoordinateMetricDescriptor {
    pub id: String,
    pub x_channel: String,
    pub y_channel: String,
}

impl CoordinateMetricDescriptor {
    pub fn new(
        id: impl Into<String>,
        x_channel: impl Into<String>,
        y_channel: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            x_channel: x_channel.into(),
            y_channel: y_channel.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CoordinateDomainParamSpec {
    pub param: Param,
    pub sharing: CoordinationScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoordinateDomainCellKey(String);

impl CoordinateDomainCellKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoordinateDomainSharedNodeKey(String);

impl CoordinateDomainSharedNodeKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CoordinateDomainGroupRequest<'a> {
    pub descriptor_id: &'a str,
    pub cells: &'a [CoordinateDomainCellRequest<'a>],
}

#[derive(Clone, Copy, Debug)]
pub struct CoordinateDomainCellRequest<'a> {
    pub cell_key: &'a CoordinateDomainCellKey,
    pub plot_area_width: f32,
    pub plot_area_height: f32,
    pub params: &'a IndexMap<String, ScalarValue>,
    pub scale_states: &'a [CoordinateDomainScaleState],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateDomainScaleState {
    pub scale_name: String,
    pub coord_channel: String,
    pub role: CoordinateDomainRole,
    pub base_domain: Option<DomainExtent>,
    pub range: Option<(f64, f64)>,
    pub node: CoordinateDomainNode,
    pub has_explicit_domain: bool,
    pub raw_domain_param: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CoordinateDomainNode {
    Local {
        cell_key: CoordinateDomainCellKey,
        scale_name: String,
    },
    Shared {
        key: CoordinateDomainSharedNodeKey,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinateDomainGroupResolution {
    pub cells: Vec<CoordinateDomainCellResolution>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateDomainCellResolution {
    pub cell_key: CoordinateDomainCellKey,
    pub domain_overrides: HashMap<String, DomainExtent>,
    pub metadata: Vec<CoordinateDomainMetadata>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateDomainMetadata {
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoordinateDomainResolvedState {
    pub cells: Vec<CoordinateDomainCellResolution>,
}

pub trait CoordinateDomainProvider: Send + Sync {
    fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor>;

    fn resolve_domain_cell(
        &self,
        request: CoordinateDomainCellRequest<'_>,
    ) -> Result<CoordinateDomainCellResolution, AvengerChartError> {
        Ok(CoordinateDomainCellResolution {
            cell_key: request.cell_key.clone(),
            domain_overrides: HashMap::new(),
            metadata: Vec::new(),
        })
    }

    fn resolve_domain_group(
        &self,
        request: CoordinateDomainGroupRequest<'_>,
    ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
        let cells = request
            .cells
            .iter()
            .copied()
            .map(|cell| self.resolve_domain_cell(cell))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CoordinateDomainGroupResolution { cells })
    }
}

#[derive(Clone, Debug)]
pub struct NumericDomainSpanEquation<N> {
    pub x_node: N,
    pub y_node: N,
    pub x_extent: DomainExtent,
    pub y_extent: DomainExtent,
    pub x_range_span: f64,
    pub y_range_span: f64,
    pub ratio: f64,
}

pub fn solve_numeric_domain_span_graph<N>(
    inputs: &[NumericDomainSpanEquation<N>],
) -> Result<HashMap<N, DomainExtent>, AvengerChartError>
where
    N: Clone + Debug + Eq + Hash,
{
    let mut extents = HashMap::<N, DomainExtent>::new();
    let mut edges = HashMap::<N, Vec<(N, f64)>>::new();

    for input in inputs {
        validate_positive_finite(input.ratio, "domain span ratio")?;
        let x_range_span = validate_positive_finite(input.x_range_span, "x range span")?;
        let y_range_span = validate_positive_finite(input.y_range_span, "y range span")?;
        merge_node_extent(&mut extents, input.x_node.clone(), input.x_extent.clone());
        merge_node_extent(&mut extents, input.y_node.clone(), input.y_extent.clone());

        let offset = (y_range_span / (input.ratio * x_range_span)).ln();
        if input.x_node == input.y_node {
            if offset.abs() > DOMAIN_SPAN_EPSILON {
                return Err(AvengerChartError::InvalidArgument(
                    "domain span equation uses the same domain node for x and y but the \
                     plot-area equation requires different spans"
                        .to_string(),
                ));
            }
            continue;
        }

        edges
            .entry(input.x_node.clone())
            .or_default()
            .push((input.y_node.clone(), offset));
        edges
            .entry(input.y_node.clone())
            .or_default()
            .push((input.x_node.clone(), -offset));
    }

    let mut offsets = HashMap::<N, f64>::new();
    let mut component_by_node = HashMap::<N, N>::new();
    for node in extents.keys() {
        if offsets.contains_key(node) {
            continue;
        }
        let component = node.clone();
        offsets.insert(node.clone(), 0.0);
        component_by_node.insert(node.clone(), component.clone());
        let mut queue = VecDeque::from([node.clone()]);
        while let Some(current) = queue.pop_front() {
            let current_offset = offsets[&current];
            for (next, edge_offset) in edges.get(&current).into_iter().flatten() {
                let candidate = current_offset + edge_offset;
                match offsets.get(next) {
                    Some(existing) => {
                        if (existing - candidate).abs() > DOMAIN_SPAN_EPSILON {
                            return Err(AvengerChartError::InvalidArgument(
                                "domain span equations are inconsistent".to_string(),
                            ));
                        }
                    }
                    None => {
                        offsets.insert(next.clone(), candidate);
                        component_by_node.insert(next.clone(), component.clone());
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    let mut component_required_shift = HashMap::<N, f64>::new();
    for node in extents.keys() {
        let component = component_by_node[node].clone();
        let base_span = numeric_extent_span(&extents[node], node)?;
        let required = base_span.ln() - offsets[node];
        component_required_shift
            .entry(component)
            .and_modify(|existing| *existing = existing.max(required))
            .or_insert(required);
    }

    extents
        .into_iter()
        .map(|(node, extent)| {
            let component = component_by_node[&node].clone();
            let shift = component_required_shift[&component];
            let target_span = (offsets[&node] + shift).exp();
            Ok((node, expand_numeric_extent(&extent, target_span)))
        })
        .collect()
}

fn validate_positive_finite(value: f64, label: &str) -> Result<f64, AvengerChartError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be positive and finite, got {value}"
        )))
    }
}

fn merge_node_extent<N>(extents: &mut HashMap<N, DomainExtent>, node: N, extent: DomainExtent)
where
    N: Eq + Hash,
{
    extents
        .entry(node)
        .and_modify(|existing| *existing = union_domain_extents(existing, &extent))
        .or_insert(extent);
}

fn numeric_extent_span<N>(extent: &DomainExtent, node: &N) -> Result<f64, AvengerChartError>
where
    N: Debug,
{
    let DomainBounds::Numeric { min, max } = &extent.bounds else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "domain span graph requires numeric extents, got {node:?}"
        )));
    };
    validate_positive_finite((max - min).abs(), "domain span")
}

fn expand_numeric_extent(extent: &DomainExtent, target_span: f64) -> DomainExtent {
    let DomainBounds::Numeric { min, max } = &extent.bounds else {
        return extent.clone();
    };
    let center = (*min + *max) / 2.0;
    let half = target_span / 2.0;
    DomainExtent {
        bounds: DomainBounds::Numeric {
            min: center - half,
            max: center + half,
        },
        radius: extent.radius.clone(),
        ordered_discrete: extent.ordered_discrete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric_span(extent: &DomainExtent) -> f64 {
        let DomainBounds::Numeric { min, max } = &extent.bounds else {
            panic!("expected numeric extent");
        };
        *max - *min
    }

    fn assert_span(extent: &DomainExtent, expected: f64) {
        let actual = numeric_span(extent);
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected span {expected}, got {actual}"
        );
    }

    #[test]
    fn numeric_domain_span_graph_expands_shared_x_from_local_y_equations() {
        let solved = solve_numeric_domain_span_graph(&[
            NumericDomainSpanEquation {
                x_node: "shared_x",
                y_node: "left_y",
                x_extent: DomainExtent::numeric(0.0, 10.0),
                y_extent: DomainExtent::numeric(0.0, 10.0),
                x_range_span: 200.0,
                y_range_span: 100.0,
                ratio: 1.0,
            },
            NumericDomainSpanEquation {
                x_node: "shared_x",
                y_node: "right_y",
                x_extent: DomainExtent::numeric(0.0, 8.0),
                y_extent: DomainExtent::numeric(0.0, 5.0),
                x_range_span: 200.0,
                y_range_span: 100.0,
                ratio: 1.0,
            },
        ])
        .expect("solve");

        assert_span(solved.get("shared_x").expect("shared x"), 20.0);
        assert_span(solved.get("left_y").expect("left y"), 10.0);
        assert_span(solved.get("right_y").expect("right y"), 10.0);
    }

    #[test]
    fn numeric_domain_span_graph_rejects_inconsistent_shared_equations() {
        let err = solve_numeric_domain_span_graph(&[
            NumericDomainSpanEquation {
                x_node: "shared_x",
                y_node: "shared_y",
                x_extent: DomainExtent::numeric(0.0, 10.0),
                y_extent: DomainExtent::numeric(0.0, 10.0),
                x_range_span: 200.0,
                y_range_span: 100.0,
                ratio: 1.0,
            },
            NumericDomainSpanEquation {
                x_node: "shared_x",
                y_node: "shared_y",
                x_extent: DomainExtent::numeric(0.0, 10.0),
                y_extent: DomainExtent::numeric(0.0, 10.0),
                x_range_span: 400.0,
                y_range_span: 100.0,
                ratio: 1.0,
            },
        ])
        .expect_err("inconsistent repeat equations should fail");

        assert!(err.to_string().contains("inconsistent"));
    }
}
