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
    array::{
        new_null_array, Array, ArrayRef, AsArray, Float32Array, Float64Array, StructArray,
        UInt32Array,
    },
    buffer::NullBuffer,
    compute::{cast, kernels::take},
    datatypes::{DataType, Field},
    row::{RowConverter, SortField},
};
use indexmap::IndexSet;
use lazy_static::lazy_static;

use crate::{error::AvengerScaleError, scalar::Scalar};

use super::domain_summary::{fit, DomainOptions, DomainSummary, Extents};
use super::no_domain;
use super::{
    band::{band_extent, layout_start, layout_step},
    ConfiguredScale, DomainKind, OptionConstraint, OptionDefinition, ScaleConfig, ScaleContext,
    ScaleImpl,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NestedBandNestScope {
    Free,
    Shared,
}

impl NestedBandNestScope {
    fn parse(token: &str) -> Result<Self, String> {
        match token.trim().to_ascii_lowercase().as_str() {
            "free" => Ok(Self::Free),
            "shared" => Ok(Self::Shared),
            other => Err(format!(
                "nest scope must be 'free' or 'shared', got '{other}'"
            )),
        }
    }
}

/// One level's nesting and padding, in leaf steps.
#[derive(Debug, Clone)]
struct NestedBandLevelOptions {
    nest_scope: NestedBandNestScope,
    padding_inner: f64,
    padding_outer: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedBandPathComponent {
    /// Arrow's row encoding of the component's key, which identifies it within its level.
    pub key: Vec<u8>,
    /// The component's explicit label, or else its key as text.
    pub label: String,
    /// The key when the label is derived from it, so guides can format it instead.
    pub value: Option<ArrayRef>,
}

#[derive(Debug, Clone, PartialEq)]
struct NestedBandPath {
    components: Vec<NestedBandPathComponent>,
}

impl NestedBandPath {
    fn key(&self) -> Vec<Vec<u8>> {
        self.components
            .iter()
            .map(|component| component.key.clone())
            .collect()
    }
}

/// One band of a level: its label, its path from the outermost level, and its extent. `start` is
/// the edge nearer the range start, so it is the higher pixel coordinate on a reversed range.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedBandAxisBand {
    pub label: String,
    pub path: Vec<NestedBandPathComponent>,
    pub start: f64,
    pub end: f64,
}

/// Solved nested-band geometry for a configured scale.
#[derive(Debug, Clone)]
pub struct NestedBandLayout {
    field_names: Vec<String>,
    leaf_level: usize,
    leaf_bandwidth: f64,
    axis_bands: Vec<Vec<NestedBandAxisBand>>,
    /// The index in `axis_bands[level]` of each band that a domain path reaches, by level and key.
    band_index: HashMap<(usize, Vec<Vec<u8>>), usize>,
    /// The domain row of each leaf band, or `None` for a shared level's virtual leaves.
    leaf_rows: Vec<Option<u32>>,
}

impl NestedBandLayout {
    pub fn from_config(config: &ScaleConfig) -> Result<Self, AvengerScaleError> {
        NestedBandScale.validate_options(config)?;
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

    pub fn leaf_bandwidth(&self) -> f64 {
        self.leaf_bandwidth
    }

    pub fn axis_bands(&self, level: usize) -> Result<&[NestedBandAxisBand], AvengerScaleError> {
        self.axis_bands
            .get(level)
            .map(Vec::as_slice)
            .ok_or_else(|| invalid_level(level, self.field_names.len()))
    }

    /// Records a band of `level`. A band that a domain path reaches positions values, and a
    /// leaf keeps its domain row for inversion.
    fn record(&mut self, level: usize, band: NestedBandAxisBand, concrete: bool, row: Option<u32>) {
        if concrete {
            let key = band
                .path
                .iter()
                .map(|component| component.key.clone())
                .collect();
            self.band_index
                .insert((level, key), self.axis_bands[level].len());
        }
        if level == self.leaf_level {
            self.leaf_rows.push(row);
        }
        self.axis_bands[level].push(band);
    }

    /// Moves the bands, laid out from `start` at the range's lower end, into range coordinates. A
    /// reversed range mirrors the bands across the span they fill, so the first band sits at the
    /// range start, as band scales reverse their bands.
    fn place(&mut self, reverse: bool, start: f64, offset: f64) {
        let end = self.axis_bands[0]
            .iter()
            .map(|band| band.end)
            .fold(start, f64::max);
        for band in self.axis_bands.iter_mut().flatten() {
            if reverse {
                (band.start, band.end) = (start + end - band.start, start + end - band.end);
            }
            band.start += offset;
            band.end += offset;
        }
    }

    /// The position `band` of the way across the path's band at `level`, from its lower edge, as
    /// band scales position values.
    fn position_for_path(&self, path: &NestedBandPath, level: usize, band: f64) -> Option<f64> {
        let mut key = path.key();
        key.truncate(level + 1);
        let axis_band = &self.axis_bands[level][*self.band_index.get(&(level, key))?];
        let lower = axis_band.start.min(axis_band.end);
        Some(lower + (axis_band.start - axis_band.end).abs() * band)
    }
}

/// Nested band scale that maps struct-valued categorical paths to numeric positions.
///
/// Each struct field is a level, outermost first, and the last field is the leaf. The options
/// that the band scale also has mean the same thing, and every gap is measured in leaf steps, so
/// a one-level nested scale positions values exactly as a band scale does.
///
/// # Config Options
///
/// - **align** (f64, default: 0.5): Alignment of the band layout within the range [0, 1].
///   0 aligns to the start, 0.5 centers, and 1 aligns to the end.
///
/// - **band** (f64, default: 0.0): Position within the band where the value is placed [0, 1].
///   0 places at the band's lower edge, 0.5 at its center, and 1 at its upper edge.
///
/// - **level** (i32, default: -1): The level whose bands position values. -1 is the leaf, and 0
///   places each value in its outermost band.
///
/// - **nest_scopes** (string or list of strings, default: "free"): How each level nests in its
///   parent. "free" gives a parent space for the children it has, and "shared" gives every
///   parent a slot for every child, so the slots line up across parents. One string sets every
///   level, and a list sets one level per entry, outermost first. The outermost level is always
///   free.
///
/// - **padding_inner** (f64 or list of f64, default: 0.0): The space between adjacent bands of a
///   level. At the leaf level it is a fraction [0, 1] of the step, as in band scales, so the leaf
///   bandwidth is the step times (1 - padding_inner). At an outer level it is the gap between
///   that level's groups, as a multiple of the leaf step. One number sets every level, and a list
///   sets one level per entry, outermost first.
///
/// - **padding_outer** (f64 or list of f64, default: 0.0): The space before a level's first band
///   and after its last, as a multiple of the leaf step: inside each parent for an inner level,
///   and at the range ends for the outermost level. One number or a list, as for padding_inner.
///
/// - **round** (boolean, default: false): When true, the step and the layout's start are rounded
///   to whole pixels and the leaf bandwidth to the nearest pixel.
///
/// - **range_offset** (f64, default: 0.0): An offset added to every position and band.
///
/// - **clip_padding_lower** (f64, default: 0.0): Pixels left empty at the range start, where the
///   first band sits, so that marks at the outer bands are not clipped. The bands and their
///   paddings fill the rest of the range.
///
/// - **clip_padding_upper** (f64, default: 0.0): Pixels left empty at the range end.
#[derive(Debug, Clone)]
pub struct NestedBandScale;

impl NestedBandScale {
    /// A nested band scale onto `range` with no domain yet. Scaling needs a domain, which
    /// [`ConfiguredScale::fit_domain`] fits to data.
    pub fn unfitted(range: (f64, f64)) -> ConfiguredScale {
        Self::configured(no_domain(), range)
    }

    pub fn configured(domain: ArrayRef, range: (f64, f64)) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain,
                range: Arc::new(Float64Array::from(vec![range.0, range.1])),
                options: vec![
                    ("align".to_string(), 0.5.into()),
                    ("band".to_string(), 0.0.into()),
                    ("level".to_string(), (-1).into()),
                    ("padding_inner".to_string(), 0.0.into()),
                    ("padding_outer".to_string(), 0.0.into()),
                    ("round".to_string(), false.into()),
                    ("range_offset".to_string(), 0.0.into()),
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

    fn summarize(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary, AvengerScaleError> {
        fit::category_summary(values, extents, true)
    }

    fn fit_domain(
        &self,
        config: &ScaleConfig,
        options: &DomainOptions,
        summary: &DomainSummary,
    ) -> Result<ScaleConfig, AvengerScaleError> {
        fit::fit_discrete(config, options, summary, "nested_band", domain_positions)
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::NestedCategorical
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional(
                    "align",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional(
                    "band",
                    OptionConstraint::FloatRange { min: 0.0, max: 1.0 }
                ),
                OptionDefinition::optional("level", OptionConstraint::Integer),
                OptionDefinition::optional(
                    "nest_scopes",
                    OptionConstraint::Custom {
                        description: "'free' or 'shared', or a list of them".to_string(),
                        validator: validate_nest_scopes,
                    }
                ),
                OptionDefinition::optional("padding_inner", level_padding_constraint()),
                OptionDefinition::optional("padding_outer", level_padding_constraint()),
                OptionDefinition::optional("round", OptionConstraint::Boolean),
                OptionDefinition::optional("range_offset", OptionConstraint::Float),
                OptionDefinition::optional(
                    "clip_padding_lower",
                    OptionConstraint::NonNegativeFloat
                ),
                OptionDefinition::optional(
                    "clip_padding_upper",
                    OptionConstraint::NonNegativeFloat
                ),
            ];
        }

        &DEFINITIONS
    }

    fn validate_options(&self, config: &ScaleConfig) -> Result<(), AvengerScaleError> {
        OptionDefinition::validate_all(self.option_definitions(), &config.options)
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let layout = NestedBandLayout::from_config(config)?;
        let values = extract_struct_paths(values, "values", ComponentExtraction::KeyOnly)?;
        if values.field_names.len() != layout.field_names.len() {
            return Err(AvengerScaleError::InvalidDataTypeError(
                values.data_type,
                "nested_band".to_string(),
            ));
        }

        let boundary_level = boundary_level(config, layout.leaf_level())?;
        let band = config.option_f64("band", 0.0);
        let positions = values
            .paths
            .iter()
            .map(|path| {
                layout
                    .position_for_path(path, boundary_level, band)
                    .map(|position| position as f32)
            })
            .collect::<Float32Array>();
        Ok(Arc::new(positions) as ArrayRef)
    }

    fn invert_range_interval(
        &self,
        config: &ScaleConfig,
        range: (f64, f64),
    ) -> Result<ArrayRef, AvengerScaleError> {
        let layout = NestedBandLayout::from_config(config)?;
        let leaf_bands = layout.axis_bands(layout.leaf_level())?;

        let (mut lo, mut hi) = range;
        if lo.is_nan() || hi.is_nan() {
            return take_domain_key_indices(&config.domain, Vec::new());
        }
        if hi < lo {
            std::mem::swap(&mut lo, &mut hi);
        }

        let indices = leaf_bands
            .iter()
            .zip(&layout.leaf_rows)
            .filter(|(band, _)| hi >= band.start.min(band.end) && lo <= band.start.max(band.end))
            .filter_map(|(_, row)| *row)
            .collect::<Vec<_>>();

        take_domain_key_indices(&config.domain, indices)
    }
}

/// The position of each domain path, for fitting the domain's clip padding.
fn domain_positions(config: &ScaleConfig) -> Result<Vec<f64>, AvengerScaleError> {
    let layout = NestedBandLayout::from_config(config)?;
    let paths = extract_struct_paths(&config.domain, "domain", ComponentExtraction::KeyOnly)?;
    let boundary_level = boundary_level(config, layout.leaf_level())?;
    let band = config.option_f64("band", 0.0);
    paths
        .paths
        .iter()
        .map(|path| {
            layout
                .position_for_path(path, boundary_level, band)
                .ok_or_else(|| {
                    AvengerScaleError::InternalError("a domain path has no position".to_string())
                })
        })
        .collect()
}

fn level_padding_constraint() -> OptionConstraint {
    OptionConstraint::Custom {
        description: "non-negative number or list of non-negative numbers".to_string(),
        validator: validate_level_padding,
    }
}

fn validate_level_padding(value: &Scalar) -> Result<(), String> {
    let values = match value.as_f64() {
        Ok(value) => vec![value],
        Err(_) => value
            .as_f64_list()
            .map_err(|_| "must be a number or a list of numbers".to_string())?,
    };
    if values
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
    {
        Ok(())
    } else {
        Err("must be non-negative and finite".to_string())
    }
}

fn validate_nest_scopes(value: &Scalar) -> Result<(), String> {
    let tokens = match value.as_string() {
        Ok(token) => vec![token],
        Err(_) => value
            .as_string_list()
            .map_err(|_| "must be a string or a list of strings".to_string())?,
    };
    tokens
        .iter()
        .try_for_each(|token| NestedBandNestScope::parse(token).map(|_| ()))
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

    let levels = struct_array
        .columns()
        .iter()
        .map(|column| level_components(column, struct_array.nulls(), extraction))
        .collect::<Result<Vec<_>, _>>()?;
    let paths = (0..struct_array.len())
        .map(|row| NestedBandPath {
            components: levels.iter().map(|level| level[row].clone()).collect(),
        })
        .collect();

    Ok(ExtractedPaths {
        data_type: array.data_type().clone(),
        field_names,
        paths,
    })
}

/// One struct field's components in row order. Rows where the struct or a labeled component is
/// null are null components, as are null keys without a label; both are labeled "null".
fn level_components(
    column: &ArrayRef,
    row_nulls: Option<&NullBuffer>,
    extraction: ComponentExtraction,
) -> Result<Vec<NestedBandPathComponent>, AvengerScaleError> {
    if column.is_empty() {
        return Ok(Vec::new());
    }
    let (keys, labels) = if let Some(component) = column.as_struct_opt() {
        if matches!(extraction, ComponentExtraction::KeyOnly) {
            return Err(AvengerScaleError::ScaleOperationNotSupported(
                "nested_band values must use key-only struct fields; labeled component structs are only supported in domains"
                    .to_string(),
            ));
        }
        validate_labeled_component_type(column.data_type())?;
        let keys = component.column_by_name("key").ok_or_else(|| {
            AvengerScaleError::InternalError("Validated labeled component without key".to_string())
        })?;
        let labels = component.column_by_name("label").ok_or_else(|| {
            AvengerScaleError::InternalError(
                "Validated labeled component without label".to_string(),
            )
        })?;
        (keys.clone(), Some(text(labels)?))
    } else {
        (column.clone(), None)
    };

    // Arrow's row format gives keys of every type a comparable binary form
    let converter = RowConverter::new(vec![SortField::new(keys.data_type().clone())])?;
    let key_rows = converter.convert_columns(std::slice::from_ref(&keys))?;
    let null_key = converter
        .convert_columns(&[new_null_array(keys.data_type(), 1)])?
        .row(0)
        .as_ref()
        .to_vec();

    // Values only need keys
    let key_text = match extraction {
        ComponentExtraction::KeyOnly => None,
        ComponentExtraction::Labeled => Some(text(&keys)?),
    };

    Ok((0..column.len())
        .map(|row| {
            let null_component = NestedBandPathComponent {
                key: null_key.clone(),
                label: "null".to_string(),
                value: None,
            };
            if row_nulls.is_some_and(|nulls| nulls.is_null(row)) || column.is_null(row) {
                return null_component;
            }
            let key = key_rows.row(row).as_ref().to_vec();
            let Some(key_text) = &key_text else {
                return NestedBandPathComponent {
                    key,
                    label: String::new(),
                    value: None,
                };
            };
            match labels.as_ref().filter(|labels| labels.is_valid(row)) {
                Some(labels) => NestedBandPathComponent {
                    key,
                    label: labels.value(row).to_string(),
                    value: None,
                },
                None if keys.is_null(row) => null_component,
                None => NestedBandPathComponent {
                    key,
                    label: key_text.value(row).to_string(),
                    value: Some(keys.slice(row, 1)),
                },
            }
        })
        .collect())
}

/// Values as text, for components that show their key.
fn text(values: &ArrayRef) -> Result<arrow::array::StringArray, AvengerScaleError> {
    let text = cast(values, &DataType::Utf8).map_err(|_| {
        AvengerScaleError::InvalidDataTypeError(
            values.data_type().clone(),
            "nested_band".to_string(),
        )
    })?;
    Ok(text.as_string::<i32>().clone())
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

fn level_options(
    config: &ScaleConfig,
    level_count: usize,
) -> Result<Vec<NestedBandLevelOptions>, AvengerScaleError> {
    let nest_scopes = level_values(config, "nest_scopes", level_count, |value| {
        match value.as_string() {
            Ok(token) => Ok(vec![token]),
            Err(_) => value.as_string_list(),
        }
    })?;
    let numbers = |name| {
        level_values(config, name, level_count, |value| match value.as_f64() {
            Ok(value) => Ok(vec![value]),
            Err(_) => value.as_f64_list(),
        })
    };
    let padding_inner = numbers("padding_inner")?;
    let padding_outer = numbers("padding_outer")?;

    (0..level_count)
        .map(|level| {
            let nest_scope = match &nest_scopes {
                Some(tokens) if level > 0 => NestedBandNestScope::parse(&tokens[level])
                    .map_err(AvengerScaleError::InvalidScalePropertyValue)?,
                _ => NestedBandNestScope::Free,
            };
            let option = NestedBandLevelOptions {
                nest_scope,
                padding_inner: padding_inner.as_ref().map_or(0.0, |values| values[level]),
                padding_outer: padding_outer.as_ref().map_or(0.0, |values| values[level]),
            };
            if level == level_count - 1 && option.padding_inner > 1.0 {
                return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                    "padding_inner is {} at the leaf level but must be between 0 and 1",
                    option.padding_inner
                )));
            }
            Ok(option)
        })
        .collect()
}

/// An option's value for every level: one value for all of them, or a list with one entry per
/// level, outermost first. `None` when the option isn't set.
fn level_values<T: Clone>(
    config: &ScaleConfig,
    name: &str,
    level_count: usize,
    read: impl Fn(&Scalar) -> Result<Vec<T>, AvengerScaleError>,
) -> Result<Option<Vec<T>>, AvengerScaleError> {
    let Some(value) = config.options.get(name) else {
        return Ok(None);
    };
    let values = read(value)?;
    if !matches!(value.array().data_type(), DataType::List(_)) {
        return Ok(Some(vec![values[0].clone(); level_count]));
    }
    if values.len() != level_count {
        return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
            "{name} lists {} values for a domain with {level_count} levels",
            values.len()
        )));
    }
    Ok(Some(values))
}

/// Builds the band tree from the domain's paths and lays it out in leaf steps.
#[derive(Debug, Clone)]
struct Node {
    component: Option<NestedBandPathComponent>,
    children: Vec<usize>,
    /// The domain row of a leaf's first path.
    row: Option<u32>,
}

#[derive(Debug)]
struct LayoutBuilder {
    field_names: Vec<String>,
    level_options: Vec<NestedBandLevelOptions>,
    nodes: Vec<Node>,
    /// Each level's keys in the order the domain first gives them, which shared levels follow.
    global_order: Vec<IndexSet<Vec<u8>>>,
    nodes_by_level_key: HashMap<(usize, Vec<u8>), Vec<usize>>,
    measure_cache: HashMap<(usize, Vec<usize>), f64>,
    /// The leaf step in pixels, once `solve` sets it.
    step: f64,
}

impl LayoutBuilder {
    fn new(paths: ExtractedPaths, level_options: Vec<NestedBandLevelOptions>) -> Self {
        let mut builder = Self {
            field_names: paths.field_names.clone(),
            level_options,
            nodes: vec![Node {
                component: None,
                children: Vec::new(),
                row: None,
            }],
            global_order: vec![IndexSet::new(); paths.field_names.len()],
            nodes_by_level_key: HashMap::new(),
            measure_cache: HashMap::new(),
            step: 0.0,
        };

        for (row, path) in paths.paths.into_iter().enumerate() {
            builder.insert_path(path, row as u32);
        }

        builder
    }

    fn leaf_level(&self) -> usize {
        self.field_names.len() - 1
    }

    /// Lays the bands out as a band scale lays out its bands: the outermost level's span, in leaf
    /// steps, sets the step, `align` places the space left over, and `round` rounds the step and
    /// the start.
    fn solve(&mut self, config: &ScaleConfig) -> Result<NestedBandLayout, AvengerScaleError> {
        let (range_start, range_end) = config.numeric_interval_range()?;
        if !range_start.is_finite() || !range_end.is_finite() {
            return Err(AvengerScaleError::InvalidScalePropertyValue(format!(
                "nested_band range ({range_start}, {range_end}) must be finite"
            )));
        }
        let (range_min, range_max, reverse) = band_extent(config)?;

        let align = config.option_f64("align", 0.5);
        let round = config.option_boolean("round", false);
        let span = self.measure_children(&[0], 0);
        let inner_span = span - 2.0 * self.level_options[0].padding_outer;
        self.step = layout_step(range_min, range_max, span, round);
        let start = layout_start(range_min, range_max, self.step, inner_span, align, round);
        let leaf_bandwidth =
            self.step * (1.0 - self.level_options[self.leaf_level()].padding_inner);

        let mut layout = NestedBandLayout {
            field_names: self.field_names.clone(),
            leaf_level: self.leaf_level(),
            leaf_bandwidth: if round {
                leaf_bandwidth.round()
            } else {
                leaf_bandwidth
            },
            axis_bands: vec![Vec::new(); self.field_names.len()],
            band_index: HashMap::new(),
            leaf_rows: Vec::new(),
        };
        self.layout_children(Some(0), &[0], Vec::new(), 0, start, &mut layout);
        layout.place(reverse, start, config.option_f64("range_offset", 0.0));
        Ok(layout)
    }

    /// Adds a path's missing nodes. A repeated path changes nothing, so its leaf keeps the row of
    /// the first.
    fn insert_path(&mut self, path: NestedBandPath, row: u32) {
        let mut parent = 0;
        for (level, component) in path.components.into_iter().enumerate() {
            self.global_order[level].insert(component.key.clone());
            parent = match self.child_with_key(parent, &component.key) {
                Some(child) => child,
                None => {
                    let child = self.nodes.len();
                    self.nodes_by_level_key
                        .entry((level, component.key.clone()))
                        .or_default()
                        .push(child);
                    self.nodes.push(Node {
                        component: Some(component),
                        children: Vec::new(),
                        row: None,
                    });
                    self.nodes[parent].children.push(child);
                    child
                }
            };
        }
        self.nodes[parent].row.get_or_insert(row);
    }

    /// The span of a level's bands under `source_nodes`, in leaf steps: the bands, the inner
    /// padding between them, and the outer padding at both ends.
    fn measure_children(&mut self, source_nodes: &[usize], child_level: usize) -> f64 {
        if child_level >= self.field_names.len() {
            return 0.0;
        }

        let cache_key = (child_level, source_nodes.to_vec());
        if let Some(span) = self.measure_cache.get(&cache_key) {
            return *span;
        }

        let keys = self.template_child_keys(source_nodes, child_level);
        if keys.is_empty() {
            return 0.0;
        }

        let mut span = 0.0;
        for key in &keys {
            let child_sources = self.template_child_source_nodes(source_nodes, child_level, key);
            span += self.measure_band(&child_sources, child_level);
        }

        let option = &self.level_options[child_level];
        let span = span
            + keys.len().saturating_sub(1) as f64 * option.padding_inner
            + 2.0 * option.padding_outer;
        self.measure_cache.insert(cache_key, span);
        span
    }

    /// A band's span in leaf steps: a leaf is its bandwidth, a fraction of the step, and a parent
    /// spans its children.
    fn measure_band(&mut self, source_nodes: &[usize], level: usize) -> f64 {
        if level == self.leaf_level() {
            1.0 - self.level_options[level].padding_inner
        } else {
            self.measure_children(source_nodes, level + 1)
        }
    }

    /// Records the bands of `child_level` under `source_nodes`, from `start`, and then their
    /// children. `concrete_node` is the domain node these bands sit under, or `None` under a
    /// shared level's reserved slot.
    fn layout_children(
        &mut self,
        concrete_node: Option<usize>,
        source_nodes: &[usize],
        path_prefix: Vec<NestedBandPathComponent>,
        child_level: usize,
        start: f64,
        layout: &mut NestedBandLayout,
    ) {
        if child_level >= self.field_names.len() {
            return;
        }

        let option = &self.level_options[child_level];
        let inner_gap = option.padding_inner * self.step;
        // The outermost level's outer padding is the space that `align` places.
        let mut cursor = if child_level == 0 {
            start
        } else {
            start + option.padding_outer * self.step
        };

        for key in self.template_child_keys(source_nodes, child_level) {
            let concrete_child = concrete_node.and_then(|node| self.child_with_key(node, &key));
            let child_sources = self.template_child_source_nodes(source_nodes, child_level, &key);
            let span = self.measure_band(&child_sources, child_level) * self.step;
            let end = if child_level == self.leaf_level() {
                cursor + layout.leaf_bandwidth
            } else {
                cursor + span
            };
            let component = self.template_component(concrete_child, &child_sources);
            let mut path = path_prefix.clone();
            path.push(component.clone());
            layout.record(
                child_level,
                NestedBandAxisBand {
                    label: component.label,
                    path: path.clone(),
                    start: cursor,
                    end,
                },
                concrete_child.is_some(),
                concrete_child.and_then(|node| self.nodes[node].row),
            );
            self.layout_children(
                concrete_child,
                &child_sources,
                path,
                child_level + 1,
                cursor,
                layout,
            );
            cursor += span + inner_gap;
        }
    }

    fn template_child_keys(&self, source_nodes: &[usize], child_level: usize) -> Vec<Vec<u8>> {
        if child_level == 0
            || self.level_options[child_level].nest_scope == NestedBandNestScope::Free
        {
            let mut keys = IndexSet::new();
            for node in source_nodes {
                for child in &self.nodes[*node].children {
                    let component = self.nodes[*child]
                        .component
                        .as_ref()
                        .expect("child component");
                    keys.insert(component.key.clone());
                }
            }
            keys.into_iter().collect()
        } else {
            self.global_order[child_level].iter().cloned().collect()
        }
    }

    fn template_child_source_nodes(
        &self,
        source_nodes: &[usize],
        child_level: usize,
        key: &[u8],
    ) -> Vec<usize> {
        if child_level > 0
            && self.level_options[child_level].nest_scope == NestedBandNestScope::Shared
        {
            return self
                .nodes_by_level_key
                .get(&(child_level, key.to_vec()))
                .cloned()
                .unwrap_or_default();
        }

        source_nodes
            .iter()
            .filter_map(|node| self.child_with_key(*node, key))
            .collect()
    }

    /// The component a band shows: its domain node's, or else the first source's. A band always
    /// has a source: free levels take their keys from the sources' children, and shared levels
    /// from the nodes the domain has.
    fn template_component(
        &self,
        concrete_node: Option<usize>,
        source_nodes: &[usize],
    ) -> NestedBandPathComponent {
        let node = concrete_node
            .or_else(|| source_nodes.first().copied())
            .expect("a band has a source node");
        self.nodes[node]
            .component
            .clone()
            .expect("a band's node has a component")
    }

    fn child_with_key(&self, node: usize, key: &[u8]) -> Option<usize> {
        self.nodes[node].children.iter().copied().find(|child| {
            self.nodes[*child]
                .component
                .as_ref()
                .is_some_and(|component| component.key == key)
        })
    }
}

fn invalid_level(level: usize, level_count: usize) -> AvengerScaleError {
    AvengerScaleError::InvalidScalePropertyValue(format!(
        "nested_band level {level} is invalid for {level_count} levels"
    ))
}

/// The domain rows at `indices`, with each labeled level reduced to its keys, as values give them.
fn take_domain_key_indices(
    domain: &ArrayRef,
    indices: Vec<u32>,
) -> Result<ArrayRef, AvengerScaleError> {
    let indices = UInt32Array::from(indices);
    Ok(take::take(
        key_only_domain(domain).as_ref(),
        &indices,
        None,
    )?)
}

/// The domain with each labeled `{key, label}` level replaced by its keys. The layout has already
/// validated the domain's structure.
fn key_only_domain(domain: &ArrayRef) -> ArrayRef {
    let domain = domain.as_struct();
    let (fields, columns): (Vec<Field>, Vec<ArrayRef>) = domain
        .fields()
        .iter()
        .zip(domain.columns())
        .map(|(field, column)| match column.as_struct_opt() {
            Some(component) => {
                let (index, key_field) = component
                    .fields()
                    .find("key")
                    .expect("validated labeled component");
                let nullable = field.is_nullable() || key_field.is_nullable();
                (
                    Field::new(field.name(), key_field.data_type().clone(), nullable),
                    component.column(index).clone(),
                )
            }
            None => (field.as_ref().clone(), column.clone()),
        })
        .unzip();
    Arc::new(StructArray::new(fields.into(), columns, None))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{create_array, Array, ArrayRef, Date32Array, Int32Array, StringArray, StructArray},
        datatypes::DataType,
    };

    use super::*;
    use crate::scales::band::{self, BandScale};

    fn utf8_struct(fields: &[(&str, Vec<Option<&str>>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = fields
            .iter()
            .map(|(name, values)| {
                (
                    *name,
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    fn i32_struct(fields: &[(&str, Vec<Option<i32>>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = fields
            .iter()
            .map(|(name, values)| {
                (
                    *name,
                    Arc::new(Int32Array::from(values.clone())) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    #[allow(
        clippy::type_complexity,
        reason = "The tuple describes the input or output of this test fixture."
    )]
    fn labeled_i32_struct(fields: &[(&str, Vec<Option<i32>>, Vec<Option<&str>>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = fields
            .iter()
            .map(|(name, keys, labels)| {
                let component: Vec<(&str, ArrayRef)> = vec![
                    ("key", Arc::new(Int32Array::from(keys.clone()))),
                    ("label", Arc::new(StringArray::from(labels.clone()))),
                ];
                (
                    *name,
                    Arc::new(StructArray::try_from(component).unwrap()) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    fn invalid_labeled_i32_struct_missing_label() -> ArrayRef {
        let component: ArrayRef = Arc::new(
            StructArray::try_from(vec![("key", create_array!(Int32, [1]) as ArrayRef)]).unwrap(),
        );
        Arc::new(StructArray::try_from(vec![("month", component)]).unwrap())
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
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
            .with_option("nest_scopes", "shared");
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
            .with_option("nest_scopes", vec!["free", "shared", "free"]);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
            .with_option("nest_scopes", vec!["free", "shared", "free"]);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
            .with_option("nest_scopes", vec!["free", "shared", "free"]);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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

        // Values sit at their band's lower edge, as on a band scale
        assert_eq!(positions(&scale, &domain), vec![Some(100.0), Some(0.0)]);
    }

    #[test]
    fn nested_band_clip_padding_insets_each_end() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let padded = |range: (f64, f64)| {
            NestedBandScale::configured(domain.clone(), range)
                .with_option("clip_padding_lower", 20.0)
                .with_option("clip_padding_upper", 60.0)
        };

        // The leaves fill [20, 140]
        let scale = padded((0.0, 200.0));
        assert_eq!(positions(&scale, &domain), vec![Some(20.0), Some(80.0)]);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let leaf_bands = layout.axis_bands(1).expect("leaf bands");
        assert_eq!((leaf_bands[1].start, leaf_bands[1].end), (80.0, 140.0));

        // A reversed range starts at 200, so the leaves fill [60, 180] from the top
        let scale = padded((200.0, 0.0));
        assert_eq!(positions(&scale, &domain), vec![Some(120.0), Some(60.0)]);

        // Padding that fills the range is rejected
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 200.0))
            .with_option("clip_padding_lower", 150.0)
            .with_option("clip_padding_upper", 50.0);
        assert!(scale.scale(&domain).is_err());
    }

    #[test]
    fn nested_band_zero_leaf_padding_produces_touching_leaf_bands() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let leaf_bands = layout.axis_bands(1).expect("leaf bands");

        assert_eq!(leaf_bands[0].end, leaf_bands[1].start);
    }

    #[test]
    fn nested_band_struct_field_order_defines_levels_and_leaf() {
        let domain = utf8_struct(&[
            ("outer", vec![Some("A"), Some("A"), Some("B")]),
            ("leaf", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 300.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
        let swapped_layout = NestedBandLayout::from_config(&swapped_scale.config).expect("layout");

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
        let columns: Vec<(&str, ArrayRef)> = vec![
            ("group", create_array!(Int32, [1, 1])),
            ("date", Arc::new(Date32Array::from(vec![10, 11]))),
        ];
        let domain: ArrayRef = Arc::new(StructArray::try_from(columns).unwrap());
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 200.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let bands = layout.axis_bands(0).expect("month bands");

        assert_eq!(
            bands
                .iter()
                .map(|band| band.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Jan", "Feb"]
        );
        // Explicit labels leave nothing for guides to format
        assert_ne!(bands[0].path[0].key, bands[1].path[0].key);
        assert!(bands.iter().all(|band| band.path[0].value.is_none()));
    }

    #[test]
    fn nested_band_unlabeled_components_carry_their_keys() {
        let domain = i32_struct(&[("year", vec![Some(2024), Some(2025), None])]);
        let scale = NestedBandScale::configured(domain, (0.0, 300.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let bands = layout.axis_bands(0).expect("year bands");

        assert_eq!(
            bands
                .iter()
                .map(|band| band.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2024", "2025", "null"]
        );
        let values = bands
            .iter()
            .map(|band| {
                band.path[0].value.as_ref().map(|value| {
                    value
                        .as_any()
                        .downcast_ref::<Int32Array>()
                        .unwrap()
                        .value(0)
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec![Some(2024), Some(2025), None]);
    }

    #[test]
    fn nested_band_keys_tell_null_from_the_text_null() {
        let domain = utf8_struct(&[("team", vec![Some("null"), None])]);
        let scale = NestedBandScale::configured(domain, (0.0, 200.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

        assert_eq!(layout.axis_bands(0).expect("team bands").len(), 2);
        assert_eq!(
            positions(&scale, &utf8_struct(&[("team", vec![None, Some("null")])])),
            vec![Some(100.0), Some(0.0)]
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
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

        assert_eq!(layout.axis_bands(0).unwrap()[0].label, "1");
    }

    #[test]
    fn nested_band_component_struct_missing_key_or_label_errors() {
        let domain = invalid_labeled_i32_struct_missing_label();
        let scale = NestedBandScale::configured(domain, (0.0, 100.0));
        let err = NestedBandLayout::from_config(&scale.config).expect_err("invalid component");

        assert!(
            err.to_string().contains("key") && err.to_string().contains("label"),
            "{err}"
        );
    }

    #[test]
    fn nested_band_scalar_domain_behavior_unchanged() {
        let domain = utf8_struct(&[("month", vec![Some("Jan"), Some("Feb")])]);
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 200.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

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
        // A step of 100, so the leaves fill [0, 80] and [100, 180]
        let scale = NestedBandScale::configured(domain, (0.0, 180.0))
            .with_option("padding_inner", vec![0.0, 0.2]);

        let inverted = scale
            .invert_range_interval((90.0, 90.0))
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
        // A step of 100 and a fifth of a step between the groups
        let scale = NestedBandScale::configured(domain, (0.0, 220.0))
            .with_option("padding_inner", vec![0.2, 0.0]);

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
            .with_option("nest_scopes", vec!["free", "shared"]);

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

    #[test]
    fn one_level_nested_band_positions_like_a_band_scale() {
        let keys = vec!["a", "b", "c", "d", "e"];
        let flat = Arc::new(StringArray::from(keys.clone())) as ArrayRef;
        let nested = utf8_struct(&[("key", keys.into_iter().map(Some).collect())]);
        let settings = [
            (0.0, 0.0, 0.5, false, 0.0, 0.0, 0.0),
            (0.2, 0.1, 0.5, false, 0.0, 0.0, 0.0),
            (0.3, 0.5, 0.0, false, 0.0, 0.0, 0.0),
            (0.25, 0.4, 1.0, true, 0.0, 0.0, 0.0),
            (0.1, 0.2, 0.3, false, 5.0, 10.0, 4.0),
        ];
        for range in [(0.0, 503.0), (503.0, 0.0)] {
            for (inner, outer, align, round, offset, lower, upper) in settings {
                for position in [0.0, 0.5] {
                    let configure = |scale: ConfiguredScale| {
                        scale
                            .with_option("padding_inner", inner)
                            .with_option("padding_outer", outer)
                            .with_option("align", align)
                            .with_option("round", round)
                            .with_option("range_offset", offset)
                            .with_option("clip_padding_lower", lower)
                            .with_option("clip_padding_upper", upper)
                            .with_option("band", position)
                    };
                    let band_scale = configure(BandScale::configured(flat.clone(), range));
                    let nested_scale =
                        configure(NestedBandScale::configured(nested.clone(), range));
                    let expected = band_scale.scale_to_numeric(&flat).unwrap().as_vec(5, None);
                    let actual = positions(&nested_scale, &nested);
                    for (expected, actual) in expected.iter().zip(&actual) {
                        assert!(
                            (expected - actual.unwrap()).abs() < 1e-3,
                            "{range:?} {inner} {outer} {align} {round} {position}: \
                             {expected} != {actual:?}"
                        );
                    }
                    let layout = NestedBandLayout::from_config(&nested_scale.config).unwrap();
                    assert!(
                        (band::bandwidth(&band_scale.config).unwrap() - layout.leaf_bandwidth())
                            .abs()
                            < 1e-9
                    );
                }
            }
        }
    }

    #[test]
    fn nested_band_padding_number_sets_every_level() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        // Three leaves of half a step, half a step between leaves and between groups: 2.5 steps
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 250.0))
            .with_option("padding_inner", 0.5);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 50.0);
        let groups = layout.axis_bands(0).unwrap();
        assert_eq!((groups[0].start, groups[0].end), (0.0, 150.0));
        assert_eq!((groups[1].start, groups[1].end), (200.0, 250.0));
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(0.0), Some(100.0), Some(200.0)]
        );
    }

    #[test]
    fn nested_band_padding_list_sets_each_level() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        // A step between groups, and a quarter step inside each group's ends: 5 steps
        let scale = NestedBandScale::configured(domain.clone(), (0.0, 500.0))
            .with_option("padding_inner", vec![1.0, 0.0])
            .with_option("padding_outer", vec![0.0, 0.25]);
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

        assert_eq!(layout.leaf_bandwidth(), 100.0);
        let groups = layout.axis_bands(0).unwrap();
        assert_eq!((groups[0].start, groups[0].end), (0.0, 250.0));
        assert_eq!((groups[1].start, groups[1].end), (350.0, 500.0));
        assert_eq!(
            positions(&scale, &domain),
            vec![Some(25.0), Some(125.0), Some(375.0)]
        );
    }

    #[test]
    fn nested_band_level_lists_must_match_the_levels() {
        let domain = utf8_struct(&[("group", vec![Some("A")]), ("series", vec![Some("x")])]);
        let err = NestedBandScale::configured(domain.clone(), (0.0, 100.0))
            .with_option("padding_inner", vec![0.1])
            .scale(&domain)
            .expect_err("one value for two levels");
        assert!(
            err.to_string().contains("padding_inner lists 1 values"),
            "{err}"
        );

        let err = NestedBandScale::configured(domain.clone(), (0.0, 100.0))
            .with_option("padding_inner", 1.5)
            .scale(&domain)
            .expect_err("leaf padding past the step");
        assert!(err.to_string().contains("leaf level"), "{err}");
    }
}
