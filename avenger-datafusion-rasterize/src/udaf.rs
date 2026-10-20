//! The raster aggregate: an inner aggregate over the cells of a dense pixel grid.
//!
//! Each row's pixels give its cell, and the cell is the row's group in the inner aggregate's
//! groups accumulator, so no hash table groups the rows. The inner accumulator holds a dense
//! grid of its own state for each group of the query, and its states, as DataFusion defines
//! them, carry the partial grids from each partition to the final merge.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use arrow::array::{new_empty_array, Array, ArrayRef, AsArray, BooleanArray, ListArray};
use arrow::buffer::{BooleanBuffer, OffsetBuffer};
use arrow::compute::{cast, SortOptions};
use arrow::datatypes::{DataType, Field, FieldRef, Float32Type, Schema};
use datafusion::common::{internal_err, not_impl_err, Result, ScalarValue};
use datafusion::logical_expr::function::{AccumulatorArgs, StateFieldsArgs};
use datafusion::logical_expr::type_coercion::functions::fields_with_udf;
use datafusion::logical_expr::utils::format_state_name;
use datafusion::logical_expr::{
    Accumulator, AggregateUDF, AggregateUDFImpl, EmitTo, GroupsAccumulator, Signature, Volatility,
};
use datafusion::physical_expr::{GroupsAccumulatorAdapter, PhysicalExpr, PhysicalSortExpr};

/// A grid of pixel cells, and whether rows bin by y as well as x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Grid {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) binned_y: bool,
}

impl Grid {
    fn cells(&self) -> usize {
        self.width * self.height
    }

    /// The pixel arguments before the inner aggregate's: x, and y when rows bin by it.
    fn pixels(&self) -> usize {
        1 + usize::from(self.binned_y)
    }
}

/// An inner aggregate over the cells of a grid. Its arguments are the rows' x pixels, their y
/// pixels when rows bin by y, the inner aggregate's arguments, and the values that order its
/// rows. Its result is a list of the inner aggregate's values, one for each cell, row by row
/// from the top.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct RasterizeUdaf {
    inner: Arc<AggregateUDF>,
    grid: Grid,
    /// The sort options of the ordering values, which follow the inner aggregate's arguments.
    ordering: Vec<SortOptions>,
    ignore_nulls: bool,
    name: String,
    signature: Signature,
}

impl RasterizeUdaf {
    pub(crate) fn new(
        inner: Arc<AggregateUDF>,
        grid: Grid,
        ordering: Vec<SortOptions>,
        ignore_nulls: bool,
    ) -> Self {
        Self {
            name: format!("rasterize_{}", inner.name()),
            inner,
            grid,
            ordering,
            ignore_nulls,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    /// Where the inner aggregate's arguments end and the ordering values start.
    fn values_end(&self, args: usize) -> usize {
        args - self.ordering.len()
    }

    /// The inner aggregate's groups accumulator, over the outer call's arguments after the
    /// pixels. The inner aggregate keeps its own argument expressions, so it can read a literal
    /// argument, such as a percentile.
    fn inner_accumulator(&self, args: &AccumulatorArgs) -> Result<Box<dyn GroupsAccumulator>> {
        let (pixels, values_end) = (self.grid.pixels(), self.values_end(args.exprs.len()));
        let order_bys = args.exprs[values_end..]
            .iter()
            .zip(&self.ordering)
            .map(|(expr, options)| PhysicalSortExpr::new(expr.clone(), *options))
            .collect();
        let inner = Arc::new(InnerArgs {
            udaf: self.inner.clone(),
            return_field: self
                .inner
                .return_field(&args.expr_fields[pixels..values_end])?,
            schema: args.schema.clone(),
            exprs: args.exprs[pixels..values_end].to_vec(),
            fields: args.expr_fields[pixels..values_end].to_vec(),
            order_bys,
            ignore_nulls: self.ignore_nulls,
        });
        if inner.udaf.groups_accumulator_supported(inner.args()) {
            inner.udaf.create_groups_accumulator(inner.args())
        } else {
            Ok(Box::new(GroupsAccumulatorAdapter::new(move || {
                inner.udaf.accumulator(inner.args())
            })))
        }
    }

    fn groups(&self, args: &AccumulatorArgs) -> Result<RasterGroups> {
        let fields = &args.expr_fields[self.grid.pixels()..];
        let mut groups = RasterGroups {
            inner: self.inner_accumulator(args)?,
            grid: self.grid,
            types: fields
                .iter()
                .map(|field| field.data_type().clone())
                .collect(),
            groups: 0,
            indices: Vec::new(),
            inside: Vec::new(),
        };
        groups.touch(0)?;
        Ok(groups)
    }
}

/// What the inner aggregate's accumulators take: its arguments' expressions over the query's
/// input, and the expressions of its ordering values.
struct InnerArgs {
    udaf: Arc<AggregateUDF>,
    return_field: FieldRef,
    schema: Schema,
    exprs: Vec<Arc<dyn PhysicalExpr>>,
    fields: Vec<FieldRef>,
    order_bys: Vec<PhysicalSortExpr>,
    ignore_nulls: bool,
}

impl InnerArgs {
    fn args(&self) -> AccumulatorArgs<'_> {
        AccumulatorArgs {
            return_field: self.return_field.clone(),
            schema: &self.schema,
            ignore_nulls: self.ignore_nulls,
            order_bys: &self.order_bys,
            is_reversed: false,
            name: self.udaf.name(),
            is_distinct: false,
            exprs: &self.exprs,
            expr_fields: &self.fields,
        }
    }
}

impl AggregateUDFImpl for RasterizeUdaf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Pixels are `Float32`, and the inner aggregate coerces its arguments.
    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        let (pixels, values_end) = (self.grid.pixels(), self.values_end(types.len()));
        let fields: Vec<FieldRef> = types[pixels..values_end]
            .iter()
            .map(|data_type| Arc::new(Field::new("", data_type.clone(), true)))
            .collect();
        let mut coerced = vec![DataType::Float32; pixels];
        coerced.extend(
            fields_with_udf(&fields, self.inner.as_ref())?
                .iter()
                .map(|field| field.data_type().clone()),
        );
        coerced.extend_from_slice(&types[values_end..]);
        Ok(coerced)
    }

    fn return_type(&self, _types: &[DataType]) -> Result<DataType> {
        internal_err!("a raster's type comes from its fields")
    }

    /// A list with a value of the inner aggregate for each cell.
    fn return_field(&self, fields: &[FieldRef]) -> Result<FieldRef> {
        let values = &fields[self.grid.pixels()..self.values_end(fields.len())];
        let inner = self.inner.return_field(values)?;
        let list = DataType::new_list(inner.data_type().clone(), true);
        Ok(Arc::new(Field::new(self.name(), list, true)))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        let fields = args.input_fields;
        let values_end = self.values_end(fields.len());
        let values = &fields[self.grid.pixels()..values_end];
        let ordering: Vec<FieldRef> = fields[values_end..]
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let name = format_state_name(args.name, &format!("order_{index}"));
                Arc::new(Field::new(name, field.data_type().clone(), true))
            })
            .collect();
        let inner = self.inner.state_fields(StateFieldsArgs {
            name: args.name,
            input_fields: values,
            return_field: self.inner.return_field(values)?,
            ordering_fields: &ordering,
            is_distinct: false,
        })?;
        Ok(inner
            .iter()
            .map(|field| {
                let list = DataType::new_list(field.data_type().clone(), true);
                Arc::new(Field::new(field.name(), list, true))
            })
            .collect())
    }

    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        Ok(Box::new(RasterAccumulator {
            groups: Mutex::new(self.groups(&args)?),
            zeros: Vec::new(),
        }))
    }

    fn groups_accumulator_supported(&self, _args: AccumulatorArgs) -> bool {
        true
    }

    fn create_groups_accumulator(
        &self,
        args: AccumulatorArgs,
    ) -> Result<Box<dyn GroupsAccumulator>> {
        Ok(Box::new(self.groups(&args)?))
    }
}

/// The query's groups, each with a grid of the inner aggregate's groups: cell `c` of group `g`
/// is the inner group `g * cells + c`.
struct RasterGroups {
    inner: Box<dyn GroupsAccumulator>,
    grid: Grid,
    /// The types of the inner aggregate's arguments and ordering values.
    types: Vec<DataType>,
    /// The groups that the accumulator holds.
    groups: usize,
    /// The inner groups of a batch's rows, and whether each row lies inside the grid, reused
    /// from batch to batch.
    indices: Vec<usize>,
    inside: Vec<bool>,
}

impl RasterGroups {
    /// Size the inner state for `groups` groups, with an empty batch that a filter marks.
    ///
    /// While every inner group so far has a value, DataFusion's `NullState` takes a batch
    /// without a filter or nulls to give a value to every group below its total, as it does when
    /// a hash table numbers the groups. A grid's cells that no row reached have no value, so the
    /// filtered batch makes `NullState` track which groups have values from the start.
    fn touch(&mut self, groups: usize) -> Result<()> {
        let empty: Vec<ArrayRef> = self.types.iter().map(new_empty_array).collect();
        let filter = BooleanArray::from(Vec::<bool>::new());
        self.inner
            .update_batch(&empty, &[], Some(&filter), groups * self.grid.cells())
    }

    /// Hold at least `groups` groups, each with a grid of inner groups that no row reached.
    fn reserve(&mut self, groups: usize) -> Result<()> {
        if groups > self.groups {
            self.touch(groups)?;
            self.groups = groups;
        }
        Ok(())
    }

    /// `arrays`, `groups * cells` long, as one list for each group.
    fn lists(&self, arrays: Vec<ArrayRef>, groups: usize) -> Result<Vec<ArrayRef>> {
        let offsets = OffsetBuffer::from_lengths(std::iter::repeat_n(self.grid.cells(), groups));
        arrays
            .into_iter()
            .map(|values| {
                let field = Arc::new(Field::new_list_field(values.data_type().clone(), true));
                Ok(Arc::new(ListArray::try_new(field, offsets.clone(), values, None)?) as ArrayRef)
            })
            .collect()
    }

    /// The inner emission for `emit_to`, and the groups that it emits.
    fn emit(&mut self, emit_to: EmitTo) -> (EmitTo, usize) {
        let cells = self.grid.cells();
        match emit_to {
            EmitTo::All => (EmitTo::All, std::mem::take(&mut self.groups)),
            EmitTo::First(groups) => {
                self.groups -= groups;
                (EmitTo::First(groups * cells), groups)
            }
        }
    }
}

impl GroupsAccumulator for RasterGroups {
    fn update_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        opt_filter: Option<&BooleanArray>,
        total_num_groups: usize,
    ) -> Result<()> {
        let grid = self.grid;
        let cells = grid.cells();
        let x = cast(&values[0], &DataType::Float32)?;
        let x = x.as_primitive::<Float32Type>();
        let y = match grid.binned_y {
            true => Some(cast(&values[1], &DataType::Float32)?),
            false => None,
        };
        let y = y.as_ref().map(|y| y.as_primitive::<Float32Type>());
        let rows = x.len();
        let (xs, ys) = (x.values(), y.map(|y| y.values()));
        let (width, height) = (grid.width, grid.height);
        let (right, bottom) = (width as f32, height as f32);
        self.indices.clear();
        self.indices.reserve(rows);
        self.inside.clear();
        self.inside.reserve(rows);
        for (row, &group) in group_indices.iter().enumerate() {
            let px = xs[row];
            let mut inside = px >= 0.0 && px <= right;
            let mut cell = (px as usize).min(width - 1);
            if let Some(ys) = ys {
                let py = ys[row];
                inside &= py >= 0.0 && py <= bottom;
                cell += (py as usize).min(height - 1) * width;
            }
            self.indices
                .push(group * cells + cell * usize::from(inside));
            self.inside.push(inside);
        }
        // Rows outside the grid, rows without pixels, and rows that the aggregate's filter drops
        // update nothing.
        let mut keep = BooleanBuffer::from(self.inside.as_slice());
        for nulls in [x.nulls(), y.and_then(|y| y.nulls())].into_iter().flatten() {
            keep = &keep & nulls.inner();
        }
        if let Some(filter) = opt_filter {
            keep = &keep & filter.values();
            if let Some(nulls) = filter.nulls() {
                keep = &keep & nulls.inner();
            }
        }
        let filter = (keep.count_set_bits() < rows).then(|| BooleanArray::new(keep, None));
        self.inner.update_batch(
            &values[grid.pixels()..],
            &self.indices,
            filter.as_ref(),
            total_num_groups * cells,
        )?;
        self.groups = self.groups.max(total_num_groups);
        Ok(())
    }

    fn evaluate(&mut self, emit_to: EmitTo) -> Result<ArrayRef> {
        let (inner_emit, groups) = self.emit(emit_to);
        let plane = self.inner.evaluate(inner_emit)?;
        self.touch(self.groups)?;
        Ok(self.lists(vec![plane], groups)?.remove(0))
    }

    fn state(&mut self, emit_to: EmitTo) -> Result<Vec<ArrayRef>> {
        let (inner_emit, groups) = self.emit(emit_to);
        let states = self.inner.state(inner_emit)?;
        self.touch(self.groups)?;
        self.lists(states, groups)
    }

    /// Merge each row's grids of states into its group's grid.
    fn merge_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        total_num_groups: usize,
    ) -> Result<()> {
        let cells = self.grid.cells();
        let lists: Vec<&ListArray> = values
            .iter()
            .map(|values| values.as_list::<i32>())
            .collect();
        for (row, &group) in group_indices.iter().enumerate() {
            let states: Vec<ArrayRef> = lists.iter().map(|list| list.value(row)).collect();
            if lists.iter().any(|list| list.is_null(row)) || states.iter().any(|s| s.len() != cells)
            {
                return internal_err!("a raster state holds a value for each of its {cells} cells");
            }
            self.indices.clear();
            self.indices.extend(group * cells..(group + 1) * cells);
            self.inner
                .merge_batch(&states, &self.indices, total_num_groups * cells)?;
        }
        self.groups = self.groups.max(total_num_groups);
        Ok(())
    }

    /// Partial aggregation passes each row on as its own group's state only when the query's
    /// groups don't reduce the rows, and a raster keeps a whole grid for each group.
    fn convert_to_state(
        &self,
        _values: &[ArrayRef],
        _opt_filter: Option<&BooleanArray>,
    ) -> Result<Vec<ArrayRef>> {
        not_impl_err!("a raster keeps a grid for each group, so its groups can't be rows")
    }

    fn size(&self) -> usize {
        self.inner.size() + self.indices.capacity() * size_of::<usize>() + self.inside.capacity()
    }
}

impl fmt::Debug for RasterGroups {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterGroups")
            .field("grid", &self.grid)
            .field("groups", &self.groups)
            .finish_non_exhaustive()
    }
}

/// The accumulator of a query without groups: one group. An accumulator is `Sync`, and the
/// inner groups accumulator isn't, so a mutex holds the groups, which the accumulator's `&mut`
/// methods reach without locking.
#[derive(Debug)]
struct RasterAccumulator {
    groups: Mutex<RasterGroups>,
    /// The group of each row of a batch, reused from batch to batch.
    zeros: Vec<usize>,
}

impl RasterAccumulator {
    fn groups(&mut self) -> &mut RasterGroups {
        self.groups
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Accumulator for RasterAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        let rows = values[0].len();
        self.zeros.resize(rows, 0);
        let groups = self
            .groups
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        groups.update_batch(values, &self.zeros[..rows], None, 1)
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        let rows = states[0].len();
        self.zeros.resize(rows, 0);
        let groups = self
            .groups
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        groups.merge_batch(states, &self.zeros[..rows], 1)
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        let groups = self.groups();
        groups.reserve(1)?;
        groups
            .state(EmitTo::All)?
            .iter()
            .map(|state| ScalarValue::try_from_array(state, 0))
            .collect()
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        let groups = self.groups();
        groups.reserve(1)?;
        ScalarValue::try_from_array(&groups.evaluate(EmitTo::All)?, 0)
    }

    fn size(&self) -> usize {
        let groups = self.groups.lock().unwrap_or_else(PoisonError::into_inner);
        groups.size() + self.zeros.capacity() * size_of::<usize>()
    }
}
