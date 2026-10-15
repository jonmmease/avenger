//! Scale configurations and domain options as Arrow structs, the form in which queries pass them.

use std::collections::{HashMap, HashSet};
use std::slice;
use std::sync::Arc;

use arrow::array::{
    new_empty_array, new_null_array, Array, ArrayRef, AsArray, BooleanArray, Float64Array,
    StringArray, StructArray,
};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Field, Fields, Float64Type};
use arrow::row::{RowConverter, SortField};
use datafusion::common::{exec_err, plan_err, Result};

use crate::scalar::Scalar;
use crate::scales::domain_summary::encoding::one_row_list;
use crate::scales::domain_summary::{
    CategoryOptions, CategoryOrder, DomainOptions, IntervalOptions,
};
use crate::scales::{ScaleConfig, ScaleContext};

const DOMAIN: &str = "domain";
const RANGE: &str = "range";
const OPTIONS: &str = "options";

/// A one-row struct of `columns`, each a field of its own type.
fn struct_of(columns: Vec<(&str, ArrayRef)>) -> Result<StructArray> {
    let (fields, arrays): (Vec<_>, Vec<_>) = columns
        .into_iter()
        .map(|(name, array)| (Field::new(name, array.data_type().clone(), true), array))
        .unzip();
    if fields.is_empty() {
        return Ok(StructArray::new_empty_fields(1, None));
    }
    Ok(StructArray::try_new(Fields::from(fields), arrays, None)?)
}

/// `config` as a one-row struct of its `domain` and `range`, each a list, and its `options`.
pub(crate) fn config_struct(config: &ScaleConfig) -> Result<StructArray> {
    struct_of(vec![
        (DOMAIN, Arc::new(one_row_list(config.domain.clone()))),
        (RANGE, Arc::new(one_row_list(config.range.clone()))),
        (OPTIONS, Arc::new(options_struct(&config.options)?)),
    ])
}

/// Scale options as a one-row struct, with fields in name order.
pub(crate) fn options_struct(options: &HashMap<String, Scalar>) -> Result<StructArray> {
    let mut entries: Vec<_> = options.iter().collect();
    entries.sort_by_key(|(name, _)| *name);
    let mut columns = Vec::new();
    for (name, value) in entries {
        if value.0.len() != 1 {
            return plan_err!("scale option {name} must hold one value");
        }
        columns.push((name.as_str(), value.to_array()));
    }
    struct_of(columns)
}

fn field<'a>(configs: &'a StructArray, name: &str) -> Result<&'a ArrayRef> {
    match configs.column_by_name(name) {
        Some(column) => Ok(column),
        None => exec_err!("a scale configuration has no {name}"),
    }
}

/// The configuration at `row` of a struct array of configurations.
pub(crate) fn config_at(configs: &StructArray, row: usize) -> Result<ScaleConfig> {
    let list = |name: &str| -> Result<ArrayRef> {
        match field(configs, name)?.as_list_opt::<i32>() {
            Some(list) if list.is_valid(row) => Ok(list.value(row)),
            _ => exec_err!("a scale configuration's {name} is a non-null list"),
        }
    };
    let Some(options) = field(configs, OPTIONS)?.as_struct_opt() else {
        return exec_err!("a scale configuration's options are a struct");
    };
    Ok(ScaleConfig {
        domain: list(DOMAIN)?,
        range: list(RANGE)?,
        options: options_at(options, row)?,
        context: ScaleContext::default(),
    })
}

/// The options at `row` of a struct array of options.
pub(crate) fn options_at(options: &StructArray, row: usize) -> Result<HashMap<String, Scalar>> {
    if options.is_null(row) {
        return exec_err!("scale options are a non-null struct");
    }
    let mut names = HashSet::new();
    let mut map = HashMap::new();
    for (field, array) in options.fields().iter().zip(options.columns()) {
        if !names.insert(field.name()) {
            return exec_err!("duplicate scale option {}", field.name());
        }
        map.insert(field.name().clone(), Scalar(array.slice(row, 1)));
    }
    Ok(map)
}

/// The value types of a configuration type's domain and range, and its options' fields.
pub(crate) fn config_types(data_type: &DataType) -> Result<(DataType, DataType, Fields)> {
    let DataType::Struct(fields) = data_type else {
        return plan_err!("a scale configuration is a struct, not {data_type}");
    };
    let field = |name: &str| match fields.find(name) {
        Some((_, field)) => Ok(field.data_type()),
        None => plan_err!("a scale configuration has no {name}"),
    };
    let list = |name: &str| match field(name)? {
        DataType::List(item) => Ok(item.data_type().clone()),
        other => plan_err!("a scale configuration's {name} is a list, not {other}"),
    };
    let DataType::Struct(options) = field(OPTIONS)? else {
        return plan_err!("a scale configuration's options are a struct");
    };
    Ok((list(DOMAIN)?, list(RANGE)?, options.clone()))
}

/// A configuration of `data_type` for planning: an empty domain, a range from 0 to 1 when it is
/// numeric, and options without values.
pub(crate) fn planning_config(data_type: &DataType) -> Result<ScaleConfig> {
    let (domain, range, options) = config_types(data_type)?;
    let range = if range.is_numeric() {
        cast(
            &(Arc::new(Float64Array::from(vec![0.0, 1.0])) as ArrayRef),
            &range,
        )?
    } else {
        new_empty_array(&range)
    };
    let options = options
        .iter()
        .map(|field| {
            let value = Scalar(new_null_array(field.data_type(), 1));
            (field.name().clone(), value)
        })
        .collect();
    Ok(ScaleConfig {
        domain: new_empty_array(&domain),
        range,
        options,
        context: ScaleContext::default(),
    })
}

/// The rows of a struct array of configurations, grouped by equal configurations. Rows with a
/// null configuration are in no group.
pub(crate) fn config_groups(configs: &StructArray) -> Result<Vec<Vec<u32>>> {
    let converter = RowConverter::new(vec![SortField::new(configs.data_type().clone())])?;
    let array: ArrayRef = Arc::new(configs.clone());
    let rows = converter.convert_columns(slice::from_ref(&array))?;
    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for row in 0..configs.len() {
        if configs.is_null(row) {
            continue;
        }
        let key = rows.row(row);
        let key: &[u8] = key.as_ref();
        match index.get(key) {
            Some(&group) => groups[group].push(row as u32),
            None => {
                index.insert(key.to_vec(), groups.len());
                groups.push(vec![row as u32]);
            }
        }
    }
    Ok(groups)
}

const ZERO: &str = "zero";
const NICE: &str = "nice";
const EMPTY: &str = "empty";
const ORDER: &str = "order";
const CATEGORIES: &str = "categories";

/// `options` as a one-row struct. Interval options have `zero`, `nice`, and `empty` fields, and
/// category options have `order`, which is `ascending`, `descending`, or `explicit`, and
/// `categories` for an explicit order. Sample options have no fields.
pub(crate) fn domain_options_struct(options: &DomainOptions) -> Result<StructArray> {
    let optional_list = |values: &Option<ArrayRef>| -> ArrayRef {
        match values {
            Some(values) => Arc::new(one_row_list(values.clone())),
            None => new_null_array(&DataType::new_list(DataType::Null, true), 1),
        }
    };
    match options {
        DomainOptions::Interval(options) => struct_of(vec![
            (ZERO, Arc::new(BooleanArray::from(vec![options.zero]))),
            (NICE, Arc::new(Float64Array::from(vec![options.nice]))),
            (EMPTY, optional_list(&options.empty)),
        ]),
        DomainOptions::Categories(options) => {
            let (order, categories) = match &options.order {
                CategoryOrder::Ascending => ("ascending", None),
                CategoryOrder::Descending => ("descending", None),
                CategoryOrder::Explicit(categories) => ("explicit", Some(categories.clone())),
            };
            struct_of(vec![
                (ORDER, Arc::new(StringArray::from(vec![order]))),
                (CATEGORIES, optional_list(&categories)),
            ])
        }
        DomainOptions::Sample => struct_of(vec![]),
    }
}

/// The domain options at `row` of a struct array of them, with an empty domain or explicit
/// categories cast to `domain_type`, the type of the fitted domain. A null row has the default
/// options of its kind.
pub(crate) fn domain_options_at(
    options: &StructArray,
    row: usize,
    domain_type: &DataType,
) -> Result<DomainOptions> {
    let names: Vec<&str> = options.fields().iter().map(|f| f.name().as_str()).collect();
    let null = options.is_null(row);
    let optional_list = |name: &str| -> Result<Option<ArrayRef>> {
        let Some(list) = field(options, name)?.as_list_opt::<i32>() else {
            return exec_err!("the domain option {name} is a list");
        };
        if null || list.is_null(row) {
            return Ok(None);
        }
        Ok(Some(cast(&list.value(row), domain_type)?))
    };
    match names.as_slice() {
        [ZERO, NICE, EMPTY] => {
            let (Some(zero), Some(nice)) = (
                field(options, ZERO)?.as_boolean_opt(),
                field(options, NICE)?.as_primitive_opt::<Float64Type>(),
            ) else {
                return exec_err!("interval options are a boolean zero and an f64 nice");
            };
            Ok(DomainOptions::Interval(IntervalOptions {
                zero: !null && zero.is_valid(row) && zero.value(row),
                nice: (!null && nice.is_valid(row)).then(|| nice.value(row)),
                empty: optional_list(EMPTY)?,
            }))
        }
        [ORDER, CATEGORIES] => {
            let Some(order) = field(options, ORDER)?.as_string_opt::<i32>() else {
                return exec_err!("a category order is a string");
            };
            let order = match (null || order.is_null(row), order.value(row)) {
                (true, _) | (false, "ascending") => CategoryOrder::Ascending,
                (false, "descending") => CategoryOrder::Descending,
                (false, "explicit") => match optional_list(CATEGORIES)? {
                    Some(categories) => CategoryOrder::Explicit(categories),
                    None => return exec_err!("an explicit category order has categories"),
                },
                (false, other) => return exec_err!("unknown category order {other}"),
            };
            Ok(DomainOptions::Categories(CategoryOptions { order }))
        }
        [] => Ok(DomainOptions::Sample),
        _ => exec_err!("no kind of domain options has fields {names:?}"),
    }
}
