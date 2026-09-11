//! Nested categorical band scale evaluation.
//!
//! This module treats Arrow struct values as categorical paths and lays those
//! paths out as nested bands. The code is intentionally independent of chart
//! axes, facets, and DataFusion so lower-level consumers can evaluate and read
//! nested categorical geometry directly. Shared nested levels may synthesize
//! virtual guide bands for missing paths, but every allocated categorical span
//! is either padding/gap or a real/virtual axis band.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{Array, ArrayRef, Float32Array, StructArray, UInt32Array},
    compute::{cast, kernels::take},
    datatypes::{DataType, Field},
};
use indexmap::IndexMap;
use lazy_static::lazy_static;

use crate::error::AvengerScaleError;

use super::{
    ConfiguredScale, DomainKind, InferDomainFromDataMethod, OptionConstraint, OptionDefinition,
    RangeKind, ScaleConfig, ScaleContext, ScaleImpl,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedBandNestScope {
    Free,
    Shared,
}

impl NestedBandNestScope {
    fn parse(token: &str) -> Result<Self, AvengerScaleError> {
        match token.trim().to_ascii_lowercase().as_str() {
            "" | "free" => Ok(Self::Free),
            "shared" => Ok(Self::Shared),
            other => Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "nest scope must be 'free' or 'shared', got '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NestedBandLevelOptions {
    pub nest_scope: NestedBandNestScope,
    pub padding_inner: f32,
    pub padding_outer: f32,
    pub padding_inner_px: Option<f32>,
    pub padding_outer_px: Option<f32>,
}

impl Default for NestedBandLevelOptions {
    fn default() -> Self {
        Self {
            nest_scope: NestedBandNestScope::Free,
            padding_inner: 0.0,
            padding_outer: 0.0,
            padding_inner_px: None,
            padding_outer_px: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedBandPathComponent {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedBandPath {
    pub components: Vec<NestedBandPathComponent>,
}

impl NestedBandPath {
    fn key(&self) -> Vec<String> {
        self.components
            .iter()
            .map(|component| component.key.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedBandAxisBand {
    pub level: usize,
    pub field_name: String,
    pub label: String,
    pub path: Vec<NestedBandPathComponent>,
    pub start: f32,
    pub end: f32,
    pub center: f32,
    pub bandwidth: f32,
}

/// Solved nested-band geometry for a configured scale.
#[derive(Debug, Clone)]
pub struct NestedBandLayout {
    field_names: Vec<String>,
    leaf_level: usize,
    leaf_bandwidth: f32,
    axis_bands: Vec<Vec<NestedBandAxisBand>>,
    band_by_prefix: HashMap<(usize, Vec<String>), NestedBandAxisBand>,
}

impl NestedBandLayout {
    pub fn from_config(config: &ScaleConfig) -> Result<Self, AvengerScaleError> {
        let paths = extract_struct_paths(&config.domain, "domain", ComponentExtraction::Labeled)?;
        if paths.paths.is_empty() {
            return Err(AvengerScaleError::EmptyDomain);
        }

        let level_options = level_options(config, paths.field_names.len())?;
        let mut builder = LayoutBuilder::new(paths, level_options);
        builder.solve(config)
    }

    pub fn field_names(&self) -> &[String] {
        &self.field_names
    }

    pub fn leaf_level(&self) -> usize {
        self.leaf_level
    }

    pub fn leaf_bandwidth(&self) -> f32 {
        self.leaf_bandwidth
    }

    pub fn axis_bands(&self, level: usize) -> Result<&[NestedBandAxisBand], AvengerScaleError> {
        self.axis_bands
            .get(level)
            .map(Vec::as_slice)
            .ok_or_else(|| invalid_level(level, self.field_names.len()))
    }

    pub fn position_for_path(&self, path: &NestedBandPath, level: usize, band: f32) -> Option<f32> {
        let prefix_len = level.checked_add(1)?;
        let key = path.key();
        if key.len() < prefix_len {
            return None;
        }
        let prefix = key[..prefix_len].to_vec();
        let axis_band = self.band_by_prefix.get(&(level, prefix))?;
        Some(axis_band.start + (axis_band.end - axis_band.start) * band)
    }
}

/// Nested band scale that maps struct-valued categorical paths to numeric
/// positions.
#[derive(Debug, Clone)]
pub struct NestedBandScale;

impl NestedBandScale {
    pub fn configured(domain: ArrayRef, range: (f32, f32)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("band".to_string(), 0.0.into()),
                    ("level".to_string(), (-1).into()),
                    ("padding_inner".to_string(), 0.0.into()),
                    ("padding_outer".to_string(), 0.0.into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for NestedBandScale {
    fn scale_type(&self) -> &'static str {
        "nested_band"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Unique
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::NestedCategorical
    }

    fn range_kind(&self) -> RangeKind {
        RangeKind::Continuous
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional(
                    "band",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional("level", OptionConstraint::Integer),
                OptionDefinition::optional("nest_scopes", OptionConstraint::String),
                OptionDefinition::optional("padding_inner", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_outer", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_inner_px", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_outer_px", OptionConstraint::NonNegativeFloat),
                OptionDefinition::optional("padding_inner_levels", OptionConstraint::String),
                OptionDefinition::optional("padding_outer_levels", OptionConstraint::String),
                OptionDefinition::optional("padding_inner_px_levels", OptionConstraint::String),
                OptionDefinition::optional("padding_outer_px_levels", OptionConstraint::String),
            ];
        }

        &DEFINITIONS
    }

    fn validate_options(&self, config: &ScaleConfig) -> Result<(), AvengerScaleError> {
        OptionDefinition::validate_all(self.option_definitions(), &config.options)?;
        let band = config.option_f32("band", 0.0);
        if !(0.0..=1.0).contains(&band) || !band.is_finite() {
            return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "band is {band} but must be between 0 and 1"
            )));
        }
        level_options(config, 1).map(|_| ())
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        self.validate_options(config)?;
        let layout = NestedBandLayout::from_config(config)?;
        let values = extract_struct_paths(values, "values", ComponentExtraction::KeyOnly)?;
        if values.field_names.len() != layout.field_names.len() {
            return Err(AvengerScaleError::InvalidDataTypeError(
                values.data_type,
                "nested_band".to_string(),
            ));
        }

        let boundary_level = boundary_level(config, layout.leaf_level())?;
        let band = config.option_f32("band", 0.0);
        let positions = values
            .paths
            .iter()
            .map(|path| layout.position_for_path(path, boundary_level, band))
            .collect::<Vec<_>>();
        Ok(Arc::new(Float32Array::from(positions)) as ArrayRef)
    }

    fn invert_range_interval(
        &self,
        config: &ScaleConfig,
        range: (f32, f32),
    ) -> Result<ArrayRef, AvengerScaleError> {
        self.validate_options(config)?;
        let layout = NestedBandLayout::from_config(config)?;
        let paths = extract_struct_paths(&config.domain, "domain", ComponentExtraction::Labeled)?;
        let leaf_bands = layout.axis_bands(layout.leaf_level())?;

        let (mut lo, mut hi) = range;
        if lo.is_nan() || hi.is_nan() {
            return take_domain_key_indices(&config.domain, Vec::new());
        }
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }

        let mut domain_index_by_key = HashMap::new();
        for (index, path) in paths.paths.iter().enumerate() {
            domain_index_by_key
                .entry(path.key())
                .or_insert(index as u32);
        }

        let is_point = (lo - hi).abs() < f32::EPSILON;
        let indices = leaf_bands
            .iter()
            .filter(|band| {
                let start = band.start.min(band.end);
                let end = band.start.max(band.end);
                if is_point {
                    lo >= start && lo <= end
                } else {
                    hi >= start && lo <= end
                }
            })
            .filter_map(|band| {
                let key = band
                    .path
                    .iter()
                    .map(|component| component.key.clone())
                    .collect::<Vec<_>>();
                domain_index_by_key.get(&key).copied()
            })
            .collect::<Vec<_>>();

        take_domain_key_indices(&config.domain, indices)
    }
}

pub fn nested_band_layout(config: &ScaleConfig) -> Result<NestedBandLayout, AvengerScaleError> {
    NestedBandLayout::from_config(config)
}

pub fn nested_axis_bands(
    config: &ScaleConfig,
    level: usize,
) -> Result<Vec<NestedBandAxisBand>, AvengerScaleError> {
    Ok(NestedBandLayout::from_config(config)?
        .axis_bands(level)?
        .to_vec())
}

fn boundary_level(config: &ScaleConfig, leaf_level: usize) -> Result<usize, AvengerScaleError> {
    let level = config.option_i32("level", -1);
    if level < 0 {
        return Ok(leaf_level);
    }
    let level = level as usize;
    if level > leaf_level {
        return Err(invalid_level(level, leaf_level + 1));
    }
    Ok(level)
}

#[derive(Debug, Clone)]
struct ExtractedPaths {
    data_type: DataType,
    field_names: Vec<String>,
    paths: Vec<NestedBandPath>,
}

#[derive(Debug, Clone, Copy)]
enum ComponentExtraction {
    KeyOnly,
    Labeled,
}

fn extract_struct_paths(
    array: &ArrayRef,
    role: &str,
    extraction: ComponentExtraction,
) -> Result<ExtractedPaths, AvengerScaleError> {
    let struct_array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| {
            AvengerScaleError::InvalidDataTypeError(
                array.data_type().clone(),
                "nested_band".to_string(),
            )
        })?;

    let fields = struct_array.fields();
    if fields.is_empty() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "nested_band {role} struct must contain at least one field"
        )));
    }

    let field_names = fields
        .iter()
        .map(|field| field.name().to_string())
        .collect::<Vec<_>>();

    let mut paths = Vec::with_capacity(struct_array.len());
    for row in 0..struct_array.len() {
        let mut components = Vec::with_capacity(fields.len());
        for (field, column) in fields.iter().zip(struct_array.columns()) {
            let component = if struct_array.is_null(row) {
                null_component_for_field(field, extraction)?
            } else {
                component_at(column.as_ref(), row, extraction)?
            };
            components.push(component);
        }
        paths.push(NestedBandPath { components });
    }

    Ok(ExtractedPaths {
        data_type: array.data_type().clone(),
        field_names,
        paths,
    })
}

fn component_at(
    array: &dyn Array,
    index: usize,
    extraction: ComponentExtraction,
) -> Result<NestedBandPathComponent, AvengerScaleError> {
    if array.is_null(index) {
        return null_component_for_data_type(array.data_type(), extraction);
    }

    if matches!(array.data_type(), DataType::Struct(_)) {
        return labeled_component_at(array, index, extraction);
    }

    let (key, label) = scalar_key_and_label(array, index)?;
    Ok(NestedBandPathComponent { key, label })
}

fn scalar_key_and_label(
    array: &dyn Array,
    index: usize,
) -> Result<(String, String), AvengerScaleError> {
    let label = scalar_label(array, index)?.unwrap_or_else(|| "null".to_string());
    Ok((format!("{:?}:{label}", array.data_type()), label))
}

fn scalar_label(array: &dyn Array, index: usize) -> Result<Option<String>, AvengerScaleError> {
    if array.is_null(index) {
        return Ok(None);
    }
    let one = Arc::new(array.slice(index, 1)) as ArrayRef;
    let casted = cast(&one, &DataType::Utf8).map_err(|_| {
        AvengerScaleError::InvalidDataTypeError(
            array.data_type().clone(),
            "nested_band".to_string(),
        )
    })?;
    let labels = casted
        .as_any()
        .downcast_ref::<arrow::array::StringArray>()
        .ok_or_else(|| {
            AvengerScaleError::InternalError("Failed to cast nested band value to Utf8".to_string())
        })?;
    Ok(Some(labels.value(0).to_string()))
}

fn labeled_component_at(
    array: &dyn Array,
    index: usize,
    extraction: ComponentExtraction,
) -> Result<NestedBandPathComponent, AvengerScaleError> {
    if matches!(extraction, ComponentExtraction::KeyOnly) {
        return Err(AvengerScaleError::ScaleOperationNotSupported(
            "nested_band values must use key-only struct fields; labeled component structs are only supported in domains"
                .to_string(),
        ));
    }

    let struct_array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| {
            AvengerScaleError::InternalError(
                "Failed to downcast nested band component to StructArray".to_string(),
            )
        })?;
    validate_labeled_component_type(array.data_type())?;
    let key_column = struct_array.column_by_name("key").ok_or_else(|| {
        AvengerScaleError::InternalError("Validated labeled component without key".to_string())
    })?;
    let label_column = struct_array.column_by_name("label").ok_or_else(|| {
        AvengerScaleError::InternalError("Validated labeled component without label".to_string())
    })?;

    let (key, key_label) = scalar_key_and_label(key_column.as_ref(), index)?;
    let label = scalar_label(label_column.as_ref(), index)?.unwrap_or(key_label);
    Ok(NestedBandPathComponent { key, label })
}

fn null_component_for_field(
    field: &Field,
    extraction: ComponentExtraction,
) -> Result<NestedBandPathComponent, AvengerScaleError> {
    null_component_for_data_type(field.data_type(), extraction)
}

fn null_component_for_data_type(
    data_type: &DataType,
    extraction: ComponentExtraction,
) -> Result<NestedBandPathComponent, AvengerScaleError> {
    if matches!(data_type, DataType::Struct(_)) {
        if matches!(extraction, ComponentExtraction::KeyOnly) {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "nested_band values must use key-only struct fields; labeled component structs are only supported in domains"
                    .to_string(),
            ));
        }
        let key_type = labeled_component_key_type(data_type)?;
        return Ok(NestedBandPathComponent {
            key: format!("{key_type:?}:null"),
            label: "null".to_string(),
        });
    }

    Ok(NestedBandPathComponent {
        key: format!("{data_type:?}:null"),
        label: "null".to_string(),
    })
}

fn validate_labeled_component_type(data_type: &DataType) -> Result<(), AvengerScaleError> {
    let DataType::Struct(fields) = data_type else {
        return Ok(());
    };
    let valid = fields.len() == 2
        && fields.iter().any(|field| field.name() == "key")
        && fields.iter().any(|field| field.name() == "label");
    if valid {
        return Ok(());
    }
    Err(AvengerScaleError::InvalidScalePropertyValue(
        "nested_band domain component structs must contain exactly 'key' and 'label' fields"
            .to_string(),
    ))
}

fn labeled_component_key_type(data_type: &DataType) -> Result<&DataType, AvengerScaleError> {
    validate_labeled_component_type(data_type)?;
    let DataType::Struct(fields) = data_type else {
        return Err(AvengerScaleError::InternalError(
            "Expected labeled component struct data type".to_string(),
        ));
    };
    fields
        .iter()
        .find(|field| field.name() == "key")
        .map(|field| field.data_type())
        .ok_or_else(|| {
            AvengerScaleError::InternalError(
                "Validated labeled component without key field".to_string(),
            )
        })
}

fn level_options(
    config: &ScaleConfig,
    level_count: usize,
) -> Result<Vec<NestedBandLevelOptions>, AvengerScaleError> {
    let scope_levels = parse_scope_levels(config)?;
    let padding_inner_levels = parse_f32_levels(config, "padding_inner_levels")?;
    let padding_outer_levels = parse_f32_levels(config, "padding_outer_levels")?;
    let padding_inner_px_levels = parse_f32_levels(config, "padding_inner_px_levels")?;
    let padding_outer_px_levels = parse_f32_levels(config, "padding_outer_px_levels")?;

    let padding_inner = config.option_f32("padding_inner", 0.0);
    let padding_outer = config.option_f32("padding_outer", 0.0);
    let padding_inner_px = config
        .options
        .get("padding_inner_px")
        .map(|scalar| scalar.as_f32())
        .transpose()?;
    let padding_outer_px = config
        .options
        .get("padding_outer_px")
        .map(|scalar| scalar.as_f32())
        .transpose()?;

    let mut options = Vec::with_capacity(level_count);
    for level in 0..level_count {
        let mut option = NestedBandLevelOptions {
            nest_scope: scope_levels
                .get(level)
                .copied()
                .unwrap_or(NestedBandNestScope::Free),
            padding_inner: padding_inner_levels
                .get(level)
                .and_then(|value| *value)
                .unwrap_or(padding_inner),
            padding_outer: padding_outer_levels
                .get(level)
                .and_then(|value| *value)
                .unwrap_or(padding_outer),
            padding_inner_px: padding_inner_px_levels
                .get(level)
                .and_then(|value| *value)
                .or(padding_inner_px),
            padding_outer_px: padding_outer_px_levels
                .get(level)
                .and_then(|value| *value)
                .or(padding_outer_px),
        };

        if level == 0 {
            option.nest_scope = NestedBandNestScope::Free;
        }
        validate_level_option(level, &option)?;
        options.push(option);
    }

    Ok(options)
}

fn validate_level_option(
    level: usize,
    option: &NestedBandLevelOptions,
) -> Result<(), AvengerScaleError> {
    if option.padding_inner < 0.0 || !option.padding_inner.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_inner for level {level} must be non-negative and finite"
        )));
    }
    if option.padding_outer < 0.0 || !option.padding_outer.is_finite() {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "padding_outer for level {level} must be non-negative and finite"
        )));
    }
    if let Some(value) = option.padding_inner_px {
        if value < 0.0 || !value.is_finite() {
            return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "padding_inner_px for level {level} must be non-negative and finite"
            )));
        }
    }
    if let Some(value) = option.padding_outer_px {
        if value < 0.0 || !value.is_finite() {
            return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "padding_outer_px for level {level} must be non-negative and finite"
            )));
        }
    }
    Ok(())
}

fn parse_scope_levels(config: &ScaleConfig) -> Result<Vec<NestedBandNestScope>, AvengerScaleError> {
    let Some(value) = config.options.get("nest_scopes") else {
        return Ok(Vec::new());
    };
    value
        .as_string()?
        .split(',')
        .map(NestedBandNestScope::parse)
        .collect()
}

fn parse_f32_levels(
    config: &ScaleConfig,
    key: &str,
) -> Result<Vec<Option<f32>>, AvengerScaleError> {
    let Some(value) = config.options.get(key) else {
        return Ok(Vec::new());
    };
    value
        .as_string()?
        .split(',')
        .map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return Ok(None);
            }
            let value = token.parse::<f32>().map_err(|_| {
                AvengerScaleError::InvalidScalePropertyValue(format!(
                    "{key} must be a comma-separated list of numeric values"
                ))
            })?;
            if value < 0.0 || !value.is_finite() {
                return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                    "{key} values must be non-negative and finite"
                )));
            }
            Ok(Some(value))
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Span {
    units: f32,
    fixed: f32,
}

impl Span {
    const ZERO: Self = Self {
        units: 0.0,
        fixed: 0.0,
    };

    const LEAF: Self = Self {
        units: 1.0,
        fixed: 0.0,
    };

    fn px(self, leaf_bandwidth: f32) -> f32 {
        self.units * leaf_bandwidth + self.fixed
    }
}

#[derive(Debug, Clone)]
struct Node {
    level: Option<usize>,
    component: Option<NestedBandPathComponent>,
    children: Vec<usize>,
}

#[derive(Debug)]
struct LayoutBuilder {
    field_names: Vec<String>,
    level_options: Vec<NestedBandLevelOptions>,
    nodes: Vec<Node>,
    global_order: Vec<IndexMap<String, NestedBandPathComponent>>,
    leaf_node_by_path: HashMap<Vec<String>, usize>,
    nodes_by_level_key: HashMap<(usize, String), Vec<usize>>,
    measure_cache: HashMap<(usize, Vec<usize>), Span>,
}

impl LayoutBuilder {
    fn new(paths: ExtractedPaths, level_options: Vec<NestedBandLevelOptions>) -> Self {
        let mut builder = Self {
            field_names: paths.field_names.clone(),
            level_options,
            nodes: vec![Node {
                level: None,
                component: None,
                children: Vec::new(),
            }],
            global_order: vec![IndexMap::new(); paths.field_names.len()],
            leaf_node_by_path: HashMap::new(),
            nodes_by_level_key: HashMap::new(),
            measure_cache: HashMap::new(),
        };

        for path in paths.paths {
            builder.insert_path(path);
        }

        builder
    }

    fn solve(&mut self, config: &ScaleConfig) -> Result<NestedBandLayout, AvengerScaleError> {
        let root_span = self.measure_children(&[0], 0)?;
        let (range_start, range_end) = config.numeric_interval_range()?;
        if !range_start.is_finite() || !range_end.is_finite() {
            return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "nested_band range ({range_start}, {range_end}) must be finite"
            )));
        }
        let range_min = range_start.min(range_end);
        let range_max = range_start.max(range_end);
        let range_extent = range_max - range_min;
        let leaf_bandwidth = if root_span.units > 0.0 {
            ((range_extent - root_span.fixed) / root_span.units).max(0.0)
        } else {
            0.0
        };

        let mut solved = SolvedLayoutBuilder {
            field_names: self.field_names.clone(),
            leaf_level: self.field_names.len() - 1,
            leaf_bandwidth,
            axis_bands: vec![Vec::new(); self.field_names.len()],
            band_by_prefix: HashMap::new(),
            range_min,
            range_max,
            reverse: range_end < range_start,
        };
        self.layout_children(
            Some(0),
            &[0],
            Vec::new(),
            0,
            range_min,
            leaf_bandwidth,
            &mut solved,
        )?;
        Ok(solved.finish())
    }

    fn insert_path(&mut self, path: NestedBandPath) {
        let path_key = path.key();
        if self.leaf_node_by_path.contains_key(&path_key) {
            return;
        }

        let mut parent = 0;
        for (level, component) in path.components.iter().enumerate() {
            self.global_order[level]
                .entry(component.key.clone())
                .or_insert_with(|| component.clone());

            let child = self.nodes[parent].children.iter().copied().find(|child| {
                self.nodes[*child]
                    .component
                    .as_ref()
                    .is_some_and(|existing| existing.key == component.key)
            });

            parent = if let Some(child) = child {
                child
            } else {
                let child = self.nodes.len();
                self.nodes.push(Node {
                    level: Some(level),
                    component: Some(component.clone()),
                    children: Vec::new(),
                });
                self.nodes[parent].children.push(child);
                self.nodes_by_level_key
                    .entry((level, component.key.clone()))
                    .or_default()
                    .push(child);
                child
            };
        }

        self.leaf_node_by_path.insert(path_key, parent);
    }

    fn measure_children(
        &mut self,
        source_nodes: &[usize],
        child_level: usize,
    ) -> Result<Span, AvengerScaleError> {
        if child_level >= self.field_names.len() {
            return Ok(Span::ZERO);
        }

        let cache_key = (child_level, source_nodes.to_vec());
        if let Some(span) = self.measure_cache.get(&cache_key) {
            return Ok(*span);
        }

        let keys = self.template_child_keys(source_nodes, child_level);
        if keys.is_empty() {
            return Ok(Span::ZERO);
        }

        let mut units = 0.0;
        let mut fixed = 0.0;
        for key in &keys {
            let child_sources = self.template_child_source_nodes(source_nodes, child_level, key);
            let span = self.measure_band(&child_sources, child_level)?;
            units += span.units;
            fixed += span.fixed;
        }

        let option = &self.level_options[child_level];
        let gap_count = keys.len().saturating_sub(1) as f32;
        let (inner_units, inner_fixed) = gap_span(option.padding_inner, option.padding_inner_px);
        let (outer_units, outer_fixed) = gap_span(option.padding_outer, option.padding_outer_px);

        let span = Span {
            units: units + gap_count * inner_units + 2.0 * outer_units,
            fixed: fixed + gap_count * inner_fixed + 2.0 * outer_fixed,
        };
        self.measure_cache.insert(cache_key, span);
        Ok(span)
    }

    fn measure_band(
        &mut self,
        source_nodes: &[usize],
        level: usize,
    ) -> Result<Span, AvengerScaleError> {
        if level == self.field_names.len() - 1 {
            Ok(Span::LEAF)
        } else {
            self.measure_children(source_nodes, level + 1)
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
    )]
    fn layout_children(
        &mut self,
        concrete_node: Option<usize>,
        source_nodes: &[usize],
        path_prefix: Vec<NestedBandPathComponent>,
        child_level: usize,
        start: f32,
        leaf_bandwidth: f32,
        solved: &mut SolvedLayoutBuilder,
    ) -> Result<(), AvengerScaleError> {
        if child_level >= self.field_names.len() {
            return Ok(());
        }

        let keys = self.template_child_keys(source_nodes, child_level);
        if keys.is_empty() {
            return Ok(());
        }

        let option = &self.level_options[child_level];
        let inner_gap = gap_px(
            option.padding_inner,
            option.padding_inner_px,
            leaf_bandwidth,
        );
        let outer_gap = gap_px(
            option.padding_outer,
            option.padding_outer_px,
            leaf_bandwidth,
        );
        let mut cursor = start + outer_gap;

        for key in keys {
            let concrete_child = concrete_node.and_then(|node| self.child_with_key(node, &key));
            let child_sources = self.template_child_source_nodes(source_nodes, child_level, &key);
            let span = self.measure_band(&child_sources, child_level)?;
            let width = span.px(leaf_bandwidth);
            let end = cursor + width;
            if let Some(component) =
                self.template_component(concrete_child, &child_sources, child_level, &key)
            {
                let mut path = path_prefix.clone();
                path.push(component.clone());
                solved.record_axis_band_for_path(
                    child_level,
                    &component,
                    path.clone(),
                    cursor,
                    end,
                    concrete_child.is_some(),
                );
                self.layout_children(
                    concrete_child,
                    &child_sources,
                    path,
                    child_level + 1,
                    cursor,
                    leaf_bandwidth,
                    solved,
                )?;
            }
            cursor = end + inner_gap;
        }
        Ok(())
    }

    fn template_child_keys(&self, source_nodes: &[usize], child_level: usize) -> Vec<String> {
        if child_level == 0
            || self.level_options[child_level].nest_scope == NestedBandNestScope::Free
        {
            let mut keys = IndexMap::new();
            for node in source_nodes {
                for child in &self.nodes[*node].children {
                    let child = &self.nodes[*child];
                    if child.level == Some(child_level) {
                        let component = child.component.as_ref().expect("child component");
                        keys.entry(component.key.clone()).or_insert(());
                    }
                }
            }
            keys.keys().cloned().collect()
        } else {
            self.global_order[child_level].keys().cloned().collect()
        }
    }

    fn template_child_source_nodes(
        &self,
        source_nodes: &[usize],
        child_level: usize,
        key: &str,
    ) -> Vec<usize> {
        if child_level > 0
            && self.level_options[child_level].nest_scope == NestedBandNestScope::Shared
        {
            return self
                .nodes_by_level_key
                .get(&(child_level, key.to_string()))
                .cloned()
                .unwrap_or_default();
        }

        source_nodes
            .iter()
            .filter_map(|node| self.child_with_key(*node, key))
            .collect()
    }

    fn template_component(
        &self,
        concrete_node: Option<usize>,
        source_nodes: &[usize],
        level: usize,
        key: &str,
    ) -> Option<NestedBandPathComponent> {
        concrete_node
            .and_then(|node| self.nodes[node].component.clone())
            .or_else(|| {
                source_nodes
                    .first()
                    .and_then(|node| self.nodes[*node].component.clone())
            })
            .or_else(|| self.global_order[level].get(key).cloned())
    }

    fn child_with_key(&self, node: usize, key: &str) -> Option<usize> {
        self.nodes[node].children.iter().copied().find(|child| {
            self.nodes[*child]
                .component
                .as_ref()
                .is_some_and(|component| component.key == key)
        })
    }
}

#[derive(Debug)]
struct SolvedLayoutBuilder {
    field_names: Vec<String>,
    leaf_level: usize,
    leaf_bandwidth: f32,
    axis_bands: Vec<Vec<NestedBandAxisBand>>,
    band_by_prefix: HashMap<(usize, Vec<String>), NestedBandAxisBand>,
    range_min: f32,
    range_max: f32,
    reverse: bool,
}

impl SolvedLayoutBuilder {
    fn finish(self) -> NestedBandLayout {
        NestedBandLayout {
            field_names: self.field_names,
            leaf_level: self.leaf_level,
            leaf_bandwidth: self.leaf_bandwidth,
            axis_bands: self.axis_bands,
            band_by_prefix: self.band_by_prefix,
        }
    }

    fn record_axis_band_for_path(
        &mut self,
        level: usize,
        component: &NestedBandPathComponent,
        path: Vec<NestedBandPathComponent>,
        logical_start: f32,
        logical_end: f32,
        index_for_scaling: bool,
    ) {
        let start = self.coord(logical_start);
        let end = self.coord(logical_end);
        let axis_band = NestedBandAxisBand {
            level,
            field_name: self.field_names[level].clone(),
            label: component.label.clone(),
            path: path.clone(),
            start,
            end,
            center: (start + end) / 2.0,
            bandwidth: (end - start).abs(),
        };
        if index_for_scaling {
            let key = path.iter().map(|component| component.key.clone()).collect();
            self.band_by_prefix.insert((level, key), axis_band.clone());
        }
        self.axis_bands[level].push(axis_band);
    }

    fn coord(&self, logical: f32) -> f32 {
        if self.reverse {
            self.range_max - (logical - self.range_min)
        } else {
            logical
        }
    }
}

fn gap_span(normalized: f32, px: Option<f32>) -> (f32, f32) {
    match px {
        Some(px) => (0.0, px),
        None => (normalized, 0.0),
    }
}

fn gap_px(normalized: f32, px: Option<f32>, leaf_bandwidth: f32) -> f32 {
    px.unwrap_or(normalized * leaf_bandwidth)
}

fn invalid_level(level: usize, level_count: usize) -> AvengerScaleError {
    AvengerScaleError::InvalidScalePropertyValue(format!(
        "nested_band level {level} is invalid for {level_count} levels"
    ))
}

fn take_domain_indices(
    domain: &ArrayRef,
    indices: Vec<u32>,
) -> Result<ArrayRef, AvengerScaleError> {
    let indices = Arc::new(UInt32Array::from(indices)) as ArrayRef;
    Ok(take::take(domain, &indices, None)?)
}

fn take_domain_key_indices(
    domain: &ArrayRef,
    indices: Vec<u32>,
) -> Result<ArrayRef, AvengerScaleError> {
    let key_domain = key_only_domain(domain)?;
    take_domain_indices(&key_domain, indices)
}

fn key_only_domain(domain: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
    let struct_array = domain
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| {
            AvengerScaleError::InvalidDataTypeError(
                domain.data_type().clone(),
                "nested_band".to_string(),
            )
        })?;
    let fields = struct_array.fields();
    let columns = fields
        .iter()
        .zip(struct_array.columns())
        .map(|(field, column)| {
            if matches!(column.data_type(), DataType::Struct(_)) {
                validate_labeled_component_type(column.data_type())?;
                let component = column
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast nested band component to StructArray".to_string(),
                        )
                    })?;
                let key_column = component.column_by_name("key").ok_or_else(|| {
                    AvengerScaleError::InternalError(
                        "Validated labeled component without key".to_string(),
                    )
                })?;
                let key_field = match column.data_type() {
                    DataType::Struct(component_fields) => component_fields
                        .iter()
                        .find(|component_field| component_field.name() == "key")
                        .ok_or_else(|| {
                            AvengerScaleError::InternalError(
                                "Validated labeled component without key field".to_string(),
                            )
                        })?,
                    _ => unreachable!("checked struct component"),
                };
                Ok((
                    Arc::new(Field::new(
                        field.name(),
                        key_field.data_type().clone(),
                        field.is_nullable() || key_field.is_nullable(),
                    )),
                    key_column.clone(),
                ))
            } else {
                Ok((
                    Arc::new(Field::new(
                        field.name(),
                        column.data_type().clone(),
                        field.is_nullable(),
                    )),
                    column.clone(),
                ))
            }
        })
        .collect::<Result<Vec<_>, AvengerScaleError>>()?;
    Ok(Arc::new(StructArray::from(columns)) as ArrayRef)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Array, ArrayRef, Date32Array, Int32Array, StringArray, StructArray},
        datatypes::{DataType, Field},
    };

    use super::*;

    fn utf8_struct(fields: &[(&str, Vec<Option<&str>>)]) -> ArrayRef {
        let columns = fields
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(*name, DataType::Utf8, true)),
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    fn i32_struct(fields: &[(&str, Vec<Option<i32>>)]) -> ArrayRef {
        let columns = fields
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(*name, DataType::Int32, true)),
                    Arc::new(Int32Array::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    #[allow(
        clippy::type_complexity,
        reason = "The tuple describes the input or output of this test fixture."
    )]
    fn labeled_i32_struct(fields: &[(&str, Vec<Option<i32>>, Vec<Option<&str>>)]) -> ArrayRef {
        let columns = fields
            .iter()
            .map(|(name, keys, labels)| {
                let key_field = Arc::new(Field::new("key", DataType::Int32, true));
                let label_field = Arc::new(Field::new("label", DataType::Utf8, true));
                let component = Arc::new(StructArray::from(vec![
                    (
                        key_field,
                        Arc::new(Int32Array::from(keys.clone())) as ArrayRef,
                    ),
                    (
                        label_field,
                        Arc::new(StringArray::from(labels.clone())) as ArrayRef,
                    ),
                ])) as ArrayRef;
                (
                    Arc::new(Field::new(*name, component.data_type().clone(), true)),
                    component,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    fn invalid_labeled_i32_struct_missing_label() -> ArrayRef {
        let key_field = Arc::new(Field::new("key", DataType::Int32, true));
        let component = Arc::new(StructArray::from(vec![(
            key_field,
            Arc::new(Int32Array::from(vec![Some(1)])) as ArrayRef,
        )])) as ArrayRef;
        Arc::new(StructArray::from(vec![(
            Arc::new(Field::new("month", component.data_type().clone(), true)),
            component,
        )])) as ArrayRef
    }

    fn positions(scale: &ConfiguredScale, values: &ArrayRef) -> Vec<Option<f32>> {
        let scaled = scale.scale(values).expect("scale");
        let scaled = scaled.as_any().downcast_ref::<Float32Array>().unwrap();
        (0..scaled.len())
            .map(|index| {
                if scaled.is_null(index) {
                    None
                } else {
                    Some(scaled.value(index))
                }
            })
            .collect()
    }

    fn struct_utf8_values(array: &ArrayRef, field_name: &str) -> Vec<Option<String>> {
        let struct_array = array.as_any().downcast_ref::<StructArray>().unwrap();
        let column = struct_array.column_by_name(field_name).unwrap();
        let strings = column.as_any().downcast_ref::<StringArray>().unwrap();
        (0..array.len())
            .map(|index| (!strings.is_null(index)).then(|| strings.value(index).to_string()))
            .collect()
    }

    fn struct_i32_values(array: &ArrayRef, field_name: &str) -> Vec<Option<i32>> {
        let struct_array = array.as_any().downcast_ref::<StructArray>().unwrap();
        let column = struct_array.column_by_name(field_name).unwrap();
        let values = column.as_any().downcast_ref::<Int32Array>().unwrap();
        (0..array.len())
            .map(|index| (!values.is_null(index)).then(|| values.value(index)))
            .collect()
    }

    fn axis_path_labels(bands: &[NestedBandAxisBand]) -> Vec<Vec<&str>> {
        bands
            .iter()
            .map(|band| {
                band.path
                    .iter()
                    .map(|component| component.label.as_str())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn nested_band_free_layout_has_variable_parent_spans_and_constant_leaf_bandwidth() {
        let domain = utf8_struct(&[
            ("cyl", vec![Some("4"), Some("4"), Some("6")]),
            ("mfr", vec![Some("ford"), Some("toyota"), Some("ford")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 300.0));
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 100.0);
        let parent_bands = layout.axis_bands(0).expect("parent bands");
        assert_eq!(parent_bands[0].label, "4");
        assert_eq!(parent_bands[0].start, 0.0);
        assert_eq!(parent_bands[0].end, 200.0);
        assert_eq!(parent_bands[1].label, "6");
        assert_eq!(parent_bands[1].start, 200.0);
        assert_eq!(parent_bands[1].end, 300.0);

        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(100.0), Some(200.0)]
        );
    }

    #[test]
    fn nested_band_shared_layout_reserves_missing_child_slots() {
        let domain = utf8_struct(&[
            ("cyl", vec![Some("4"), Some("4"), Some("6")]),
            ("mfr", vec![Some("ford"), Some("toyota"), Some("ford")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 300.0))
            .with_option("nest_scopes", "free,shared");
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 75.0);
        let parent_bands = layout.axis_bands(0).expect("parent bands");
        assert_eq!(parent_bands[0].start, 0.0);
        assert_eq!(parent_bands[0].end, 150.0);
        assert_eq!(parent_bands[1].start, 150.0);
        assert_eq!(parent_bands[1].end, 300.0);
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(75.0), Some(150.0)]
        );

        let leaf_bands = layout.axis_bands(1).expect("leaf bands");
        assert_eq!(
            leaf_bands
                .iter()
                .map(|band| band.label.as_str())
                .collect::<Vec<_>>(),
            vec!["ford", "toyota", "ford", "toyota"]
        );
        assert_eq!(leaf_bands[3].start, 225.0);
        assert_eq!(leaf_bands[3].end, 300.0);

        let values = utf8_struct(&[("cyl", vec![Some("6")]), ("mfr", vec![Some("toyota")])]);
        assert_eq!(positions(&scale, &values), vec![None]);
    }

    #[test]
    fn nested_band_shared_parent_records_virtual_free_leaf_descendants() {
        let domain = utf8_struct(&[
            (
                "year",
                vec![Some("2024"), Some("2024"), Some("2025"), Some("2025")],
            ),
            (
                "quarter",
                vec![Some("Q1"), Some("Q2"), Some("Q1"), Some("Q2")],
            ),
            (
                "month",
                vec![Some("Jan"), Some("Apr"), Some("Jan"), Some("May")],
            ),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 600.0))
            .with_option("nest_scopes", "free,shared,free");
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 100.0);
        let leaf_bands = layout.axis_bands(2).expect("leaf bands");
        assert_eq!(
            axis_path_labels(leaf_bands),
            vec![
                vec!["2024", "Q1", "Jan"],
                vec!["2024", "Q2", "Apr"],
                vec!["2024", "Q2", "May"],
                vec!["2025", "Q1", "Jan"],
                vec!["2025", "Q2", "Apr"],
                vec!["2025", "Q2", "May"],
            ]
        );

        let year_2024 = &layout.axis_bands(0).expect("year bands")[0];
        let year_2024_leaves = leaf_bands
            .iter()
            .filter(|band| band.path[0].label == "2024")
            .collect::<Vec<_>>();
        assert_eq!(year_2024_leaves.first().unwrap().start, year_2024.start);
        assert_eq!(year_2024_leaves.last().unwrap().end, year_2024.end);

        let virtual_values = utf8_struct(&[
            ("year", vec![Some("2024"), Some("2025"), Some("2025")]),
            ("quarter", vec![Some("Q2"), Some("Q2"), Some("Q2")]),
            ("month", vec![Some("May"), Some("Apr"), Some("May")]),
        ]);
        assert_eq!(
            positions(&scale, &virtual_values),
            vec![None, None, Some(500.0)]
        );

        let inverted = scale
            .invert_range_interval((250.0, 250.0))
            .expect("invert virtual leaf");
        assert_eq!(inverted.len(), 0);
    }

    #[test]
    fn nested_band_shared_missing_non_leaf_records_virtual_descendants_recursively() {
        let domain = utf8_struct(&[
            ("region", vec![Some("A"), Some("A"), Some("B")]),
            ("category", vec![Some("cars"), Some("cars"), Some("trucks")]),
            ("maker", vec![Some("ford"), Some("toyota"), Some("volvo")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 600.0))
            .with_option("nest_scopes", "free,shared,free");
        let layout = nested_band_layout(&scale.config).expect("layout");

        let leaf_bands = layout.axis_bands(2).expect("leaf bands");
        assert_eq!(
            axis_path_labels(leaf_bands),
            vec![
                vec!["A", "cars", "ford"],
                vec!["A", "cars", "toyota"],
                vec!["A", "trucks", "volvo"],
                vec!["B", "cars", "ford"],
                vec!["B", "cars", "toyota"],
                vec!["B", "trucks", "volvo"],
            ]
        );
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(100.0), Some(500.0)]
        );

        let virtual_values = utf8_struct(&[
            ("region", vec![Some("A"), Some("B")]),
            ("category", vec![Some("trucks"), Some("cars")]),
            ("maker", vec![Some("volvo"), Some("ford")]),
        ]);
        assert_eq!(positions(&scale, &virtual_values), vec![None, None]);

        let inverted = scale
            .invert_range_interval((450.0, 450.0))
            .expect("invert recursively virtual leaf");
        assert_eq!(inverted.len(), 0);
    }

    #[test]
    fn nested_band_boundary_band_controls_leaf_position() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let start =
            NestedBandScale::configured(domain.clone(), (0.0, 200.0)).with_option("band", 0.0);
        let center =
            NestedBandScale::configured(domain.clone(), (0.0, 200.0)).with_option("band", 0.5);
        let end =
            NestedBandScale::configured(domain.clone(), (0.0, 200.0)).with_option("band", 1.0);

        assert_eq!(positions(&start, &domain), vec![Some(0.0), Some(100.0)]);
        assert_eq!(positions(&center, &domain), vec![Some(50.0), Some(150.0)]);
        assert_eq!(positions(&end, &domain), vec![Some(100.0), Some(200.0)]);
    }

    #[test]
    fn nested_band_three_level_free_shared_mixed_layout() {
        let domain = utf8_struct(&[
            (
                "region",
                vec![Some("north"), Some("north"), Some("south"), Some("south")],
            ),
            (
                "category",
                vec![Some("cars"), Some("cars"), Some("cars"), Some("trucks")],
            ),
            (
                "maker",
                vec![Some("ford"), Some("toyota"), Some("ford"), Some("volvo")],
            ),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 600.0))
            .with_option("nest_scopes", "free,shared,free");
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 100.0);
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(100.0), Some(300.0), Some(500.0)]
        );

        let region_bands = layout.axis_bands(0).expect("region bands");
        assert_eq!(region_bands[0].label, "north");
        assert_eq!(region_bands[0].start, 0.0);
        assert_eq!(region_bands[0].end, 300.0);
        assert_eq!(region_bands[1].label, "south");
        assert_eq!(region_bands[1].start, 300.0);
        assert_eq!(region_bands[1].end, 600.0);
    }

    #[test]
    fn nested_band_level_band_spans_parent_group() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let start = NestedBandScale::configured(domain.clone(), (0.0, 300.0))
            .with_option("level", 0)
            .with_option("band", 0.0);
        let end = NestedBandScale::configured(domain.clone(), (0.0, 300.0))
            .with_option("level", 0)
            .with_option("band", 1.0);

        assert_eq!(
            positions(&start, &domain),
            vec![Some(0.0), Some(0.0), Some(200.0)]
        );
        assert_eq!(
            positions(&end, &domain),
            vec![Some(200.0), Some(200.0), Some(300.0)]
        );
    }

    #[test]
    fn nested_band_reversed_range_reverses_positions() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (200.0, 0.0));

        assert_eq!(positions(&scale, &domain), vec![Some(200.0), Some(100.0)]);
    }

    #[test]
    fn nested_band_zero_leaf_padding_produces_touching_leaf_bands() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));
        let leaf_bands = nested_axis_bands(&scale.config, 1).expect("leaf bands");

        assert_eq!(leaf_bands[0].end, leaf_bands[1].start);
    }

    #[test]
    fn nested_band_parent_pixel_padding_preserves_global_leaf_bandwidth() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 130.0))
            .with_option("padding_inner_px_levels", ",10");

        assert_eq!(
            nested_band_layout(&scale.config).unwrap().leaf_bandwidth(),
            60.0
        );
        assert_eq!(positions(&scale, &domain), vec![Some(0.0), Some(70.0)]);
    }

    #[test]
    fn nested_band_struct_field_order_defines_levels_and_leaf() {
        let domain = utf8_struct(&[
            ("outer", vec![Some("A"), Some("A"), Some("B")]),
            ("leaf", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 300.0));
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.field_names(), &["outer", "leaf"]);
        assert_eq!(layout.leaf_level(), 1);
        assert_eq!(layout.axis_bands(0).unwrap()[0].label, "A");
        assert_eq!(layout.axis_bands(1).unwrap()[0].label, "x");
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(100.0), Some(200.0)]
        );

        let swapped = utf8_struct(&[
            ("leaf", vec![Some("x"), Some("y"), Some("x")]),
            ("outer", vec![Some("A"), Some("A"), Some("B")]),
        ]);
        let swapped_scale = NestedBandScale::configured(swapped.clone(), (0.0, 300.0));
        let swapped_layout = nested_band_layout(&swapped_scale.config).expect("layout");

        assert_eq!(swapped_layout.field_names(), &["leaf", "outer"]);
        assert_eq!(swapped_layout.leaf_level(), 1);
        assert_eq!(swapped_layout.axis_bands(0).unwrap()[0].label, "x");
        assert_eq!(swapped_layout.axis_bands(1).unwrap()[0].label, "A");
        assert_eq!(
            positions(&swapped_scale, &swapped),
            vec![Some(0.0), Some(200.0), Some(100.0)]
        );
    }

    #[test]
    fn nested_band_null_values_scale_when_they_are_in_domain() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), None]),
        ]);
        let values = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![None, Some("x"), None]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));

        assert_eq!(
            positions(&scale, &values),
            vec![Some(100.0), Some(0.0), None]
        );
    }

    #[test]
    fn nested_band_numeric_and_date_fields_are_categorical() {
        let fields = [
            Arc::new(Field::new("group", DataType::Int32, true)),
            Arc::new(Field::new("date", DataType::Date32, true)),
        ];
        let domain = Arc::new(StructArray::from(vec![
            (
                fields[0].clone(),
                Arc::new(Int32Array::from(vec![1, 1])) as ArrayRef,
            ),
            (
                fields[1].clone(),
                Arc::new(Date32Array::from(vec![10, 11])) as ArrayRef,
            ),
        ])) as ArrayRef;
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 200.0));
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.field_names(), &["group", "date"]);
        assert_eq!(layout.leaf_level(), 1);
        assert_eq!(layout.leaf_bandwidth(), 100.0);
        assert_eq!(positions(&scale, &domain), vec![Some(0.0), Some(100.0)]);
    }

    #[test]
    fn nested_band_labeled_domain_axis_uses_label_field() {
        let domain = labeled_i32_struct(&[(
            "month",
            vec![Some(1), Some(2)],
            vec![Some("Jan"), Some("Feb")],
        )]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));
        let layout = nested_band_layout(&scale.config).expect("layout");
        let bands = layout.axis_bands(0).expect("month bands");

        assert_eq!(
            bands
                .iter()
                .map(|band| band.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Jan", "Feb"]
        );
        assert_eq!(
            bands
                .iter()
                .map(|band| band.path[0].key.as_str())
                .collect::<Vec<_>>(),
            vec!["Int32:1", "Int32:2"]
        );
    }

    #[test]
    fn nested_band_bare_key_values_scale_against_labeled_domain() {
        let domain = labeled_i32_struct(&[(
            "month",
            vec![Some(1), Some(2)],
            vec![Some("Jan"), Some("Feb")],
        )]);
        let values = i32_struct(&[("month", vec![Some(2), Some(1), Some(3)])]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));

        assert_eq!(
            positions(&scale, &values),
            vec![Some(100.0), Some(0.0), None]
        );
    }

    #[test]
    fn nested_band_labeled_domain_inversion_returns_key_only_paths() {
        let domain = labeled_i32_struct(&[(
            "month",
            vec![Some(1), Some(2)],
            vec![Some("Jan"), Some("Feb")],
        )]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));

        let inverted = scale
            .invert_range_interval((150.0, 150.0))
            .expect("invert point");

        assert_eq!(inverted.len(), 1);
        assert!(matches!(
            inverted.data_type(),
            DataType::Struct(fields) if fields[0].data_type() == &DataType::Int32
        ));
        assert_eq!(struct_i32_values(&inverted, "month"), vec![Some(2)]);
    }

    #[test]
    fn nested_band_label_null_falls_back_to_key_string() {
        let domain = labeled_i32_struct(&[("month", vec![Some(1)], vec![None])]);
        let scale = NestedBandScale::configured(domain, (0.0, 100.0));
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.axis_bands(0).unwrap()[0].label, "1");
    }

    #[test]
    fn nested_band_component_struct_missing_key_or_label_errors() {
        let domain = invalid_labeled_i32_struct_missing_label();
        let scale = NestedBandScale::configured(domain, (0.0, 100.0));
        let err = nested_band_layout(&scale.config).expect_err("invalid component");

        assert!(
            err.to_string().contains("key") && err.to_string().contains("label"),
            "{err}"
        );
    }

    #[test]
    fn nested_band_scalar_domain_behavior_unchanged() {
        let domain = utf8_struct(&[("month", vec![Some("Jan"), Some("Feb")])]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 200.0));
        let layout = nested_band_layout(&scale.config).expect("layout");

        assert_eq!(layout.axis_bands(0).unwrap()[0].label, "Jan");
        assert_eq!(positions(&scale, &domain), vec![Some(0.0), Some(100.0)]);
    }

    #[test]
    fn nested_band_invert_point_returns_leaf_struct_path() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 300.0));

        let inverted = scale
            .invert_range_interval((150.0, 150.0))
            .expect("invert point");

        assert_eq!(inverted.len(), 1);
        assert_eq!(
            struct_utf8_values(&inverted, "group"),
            vec![Some("A".into())]
        );
        assert_eq!(
            struct_utf8_values(&inverted, "series"),
            vec![Some("y".into())]
        );
    }

    #[test]
    fn nested_band_invert_point_in_padding_returns_empty_domain() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 220.0))
            .with_option("padding_inner_px_levels", ",20");

        let inverted = scale
            .invert_range_interval((105.0, 105.0))
            .expect("invert padding point");

        assert_eq!(inverted.len(), 0);
        assert!(matches!(inverted.data_type(), DataType::Struct(_)));
    }

    #[test]
    fn nested_band_invert_point_in_parent_gap_returns_empty_domain() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 220.0))
            .with_option("padding_inner_px_levels", "20,");

        let inverted = scale
            .invert_range_interval((105.0, 105.0))
            .expect("invert parent padding point");

        assert_eq!(inverted.len(), 0);
        assert!(matches!(inverted.data_type(), DataType::Struct(_)));
    }

    #[test]
    fn nested_band_invert_shared_ghost_slot_returns_empty_domain() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 400.0))
            .with_option("nest_scopes", "free,shared");

        let inverted = scale
            .invert_range_interval((350.0, 350.0))
            .expect("invert shared ghost slot");

        assert_eq!(inverted.len(), 0);
        assert!(matches!(inverted.data_type(), DataType::Struct(_)));
    }

    #[test]
    fn nested_band_invert_interval_returns_intersecting_leaf_paths() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 300.0));

        let inverted = scale
            .invert_range_interval((50.0, 250.0))
            .expect("invert interval");

        assert_eq!(inverted.len(), 3);
        assert_eq!(
            struct_utf8_values(&inverted, "group"),
            vec![Some("A".into()), Some("A".into()), Some("B".into())]
        );
        assert_eq!(
            struct_utf8_values(&inverted, "series"),
            vec![Some("x".into()), Some("y".into()), Some("x".into())]
        );
    }

    #[test]
    fn nested_band_rejects_invalid_level() {
        let domain = utf8_struct(&[("group", vec![Some("A")]), ("series", vec![Some("x")])]);
        let scale =
            NestedBandScale::configured(domain.clone(), (0.0, 100.0)).with_option("level", 2);

        let err = scale.scale(&domain).expect_err("invalid level");
        assert!(err.to_string().contains("level 2 is invalid"));
    }
}
