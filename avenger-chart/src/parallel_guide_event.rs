use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart_core::{
    AvengerChartError, CompiledGuide,
    event::{
        PARALLEL_DIMENSION_ID_FIELD, PARALLEL_DISPLACEMENT_PX_FIELD,
        PARALLEL_DISPLACEMENT_SLOTS_FIELD, PARALLEL_DISPLAY_X_FIELD, PARALLEL_EQUILIBRIUM_X_FIELD,
        PARALLEL_ORDER_INDEX_FIELD, PARALLEL_SCALE_NAME_FIELD,
        PARALLEL_SURFACE_KIND_DIMENSION_TITLE, PARALLEL_SURFACE_KIND_FIELD, PARALLEL_TITLE_FIELD,
    },
};
use avenger_chart_parallel::CompiledParallelGuide;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;

use crate::render::EvaluatedEventDatumRows;

pub(crate) fn parallel_guide_event_datums(
    guide: Option<&Arc<dyn CompiledGuide>>,
    guide_marks: &[SceneMark],
    plot_width: f32,
    ctx: &SessionContext,
) -> Result<Vec<EvaluatedEventDatumRows>, AvengerChartError> {
    let Some(parallel) =
        guide.and_then(|guide| guide.as_any().downcast_ref::<CompiledParallelGuide>())
    else {
        return Ok(Vec::new());
    };
    let axis_datums = parallel.axis_guide_datums(plot_width, ctx);
    if axis_datums.is_empty() {
        return Ok(Vec::new());
    }

    let rows = axis_datum_batch(axis_datums)?;
    let title_mark_indices = guide_marks
        .iter()
        .enumerate()
        .filter_map(|(index, mark)| {
            let name = scene_mark_name(mark)?;
            matches!(name, "parallel_axis_title_hit" | "parallel_axis_title").then_some(index)
        })
        .collect::<Vec<_>>();

    Ok(title_mark_indices
        .into_iter()
        .map(|guide_index| EvaluatedEventDatumRows {
            // Guide marks are placed after the data-mark group in PlotComponents.
            mark_path: vec![1 + guide_index],
            subplot_id_path: Vec::new(),
            rows: rows.clone(),
        })
        .collect())
}

fn axis_datum_batch(
    datums: Vec<avenger_chart_parallel::ParallelAxisGuideDatum>,
) -> Result<RecordBatch, AvengerChartError> {
    let schema = Arc::new(Schema::new(vec![
        Field::new(PARALLEL_SURFACE_KIND_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_DIMENSION_ID_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_SCALE_NAME_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_TITLE_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_ORDER_INDEX_FIELD, DataType::Int64, false),
        Field::new(PARALLEL_EQUILIBRIUM_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLAY_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_PX_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_SLOTS_FIELD, DataType::Float64, false),
    ]));
    let len = datums.len();
    let surface_kind = vec![PARALLEL_SURFACE_KIND_DIMENSION_TITLE.to_string(); len];
    let dimension_ids = datums
        .iter()
        .map(|datum| datum.dimension_id.clone())
        .collect::<Vec<_>>();
    let scale_names = datums
        .iter()
        .map(|datum| datum.scale_name.clone())
        .collect::<Vec<_>>();
    let titles = datums
        .iter()
        .map(|datum| datum.title.clone())
        .collect::<Vec<_>>();
    let order_indices = datums
        .iter()
        .map(|datum| datum.order_index as i64)
        .collect::<Vec<_>>();
    let equilibrium_x = datums
        .iter()
        .map(|datum| f64::from(datum.equilibrium_x))
        .collect::<Vec<_>>();
    let display_x = datums
        .iter()
        .map(|datum| f64::from(datum.display_x))
        .collect::<Vec<_>>();
    let displacement_px = datums
        .iter()
        .map(|datum| f64::from(datum.displacement_px))
        .collect::<Vec<_>>();
    let displacement_slots = datums
        .iter()
        .map(|datum| f64::from(datum.displacement_slots))
        .collect::<Vec<_>>();

    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(surface_kind)) as ArrayRef,
            Arc::new(StringArray::from(dimension_ids)),
            Arc::new(StringArray::from(scale_names)),
            Arc::new(StringArray::from(titles)),
            Arc::new(Int64Array::from(order_indices)),
            Arc::new(Float64Array::from(equilibrium_x)),
            Arc::new(Float64Array::from(display_x)),
            Arc::new(Float64Array::from(displacement_px)),
            Arc::new(Float64Array::from(displacement_slots)),
        ],
    )
    .map_err(AvengerChartError::ArrowError)
}

fn scene_mark_name(mark: &SceneMark) -> Option<&str> {
    match mark {
        SceneMark::Arc(mark) => Some(mark.name.as_str()),
        SceneMark::Area(mark) => Some(mark.name.as_str()),
        SceneMark::Path(mark) => Some(mark.name.as_str()),
        SceneMark::Symbol(mark) => Some(mark.name.as_str()),
        SceneMark::Line(mark) => Some(mark.name.as_str()),
        SceneMark::Trail(mark) => Some(mark.name.as_str()),
        SceneMark::Rect(mark) => Some(mark.name.as_str()),
        SceneMark::Rule(mark) => Some(mark.name.as_str()),
        SceneMark::Text(mark) => Some(mark.name.as_str()),
        SceneMark::Image(mark) => Some(mark.name.as_str()),
        SceneMark::Group(mark) => Some(mark.name.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use arrow::array::{Float64Array, Int64Array, StringArray};
    use avenger_chart_core::{
        AxisGuideVisibilityConfig, AxisPosition, AxisVisibility, ChildFrameGuideSharingView,
        CompiledGuide, CoordinationAxis, EmptyCoordMeasurement, FacetGuideSharingView,
        GuideSharingContext, LayoutBounds, OverflowSpaceRequirement, SharingLevel, Theme,
        guide_sharing::AxisOwnershipMode,
    };
    use avenger_chart_parallel::{CompiledParallelGuide, ParallelAxis};
    use avenger_scales::scales::linear::LinearScale;
    use datafusion::{common::ScalarValue, prelude::SessionContext};
    use indexmap::IndexMap;

    use super::*;

    #[test]
    fn parallel_guide_event_datums_retain_title_rows() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let concrete = CompiledParallelGuide { axes };
        let guide: Arc<dyn CompiledGuide> = Arc::new(concrete.clone());
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "cost".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
        ]);
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let guide_marks = futures::executor::block_on(concrete.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
        ))
        .expect("evaluate parallel guide");

        let event_rows = parallel_guide_event_datums(Some(&guide), &guide_marks, 300.0, &ctx)
            .expect("parallel guide event datums");

        assert_eq!(event_rows.len(), 2);
        assert_eq!(event_rows[0].mark_path, vec![3]);
        assert_eq!(event_rows[1].mark_path, vec![4]);
        for rows in &event_rows {
            assert_eq!(rows.rows.num_rows(), 2);
            assert_eq!(
                rows.rows.schema().field(0).name(),
                PARALLEL_SURFACE_KIND_FIELD
            );
        }

        let retained = &event_rows[0].rows;
        let surface_kind = retained
            .column_by_name(PARALLEL_SURFACE_KIND_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("surface kind column");
        let dimensions = retained
            .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("dimension id column");
        let titles = retained
            .column_by_name(PARALLEL_TITLE_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("title column");
        let order_indices = retained
            .column_by_name(PARALLEL_ORDER_INDEX_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<Int64Array>())
            .expect("order index column");
        let display_x = retained
            .column_by_name(PARALLEL_DISPLAY_X_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
            .expect("display x column");

        assert_eq!(surface_kind.value(0), PARALLEL_SURFACE_KIND_DIMENSION_TITLE);
        assert_eq!(surface_kind.value(1), PARALLEL_SURFACE_KIND_DIMENSION_TITLE);
        assert_eq!(dimensions.value(0), "speed");
        assert_eq!(dimensions.value(1), "cost");
        assert_eq!(titles.value(0), "Speed");
        assert_eq!(titles.value(1), "Cost");
        assert_eq!(order_indices.value(0), 0);
        assert_eq!(order_indices.value(1), 1);
        assert_eq!(display_x.value(0), 0.0);
        assert_eq!(display_x.value(1), 300.0);
    }

    struct TestFacetGuideSharingView;

    impl FacetGuideSharingView for TestFacetGuideSharingView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            None
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            None
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            false
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            SharingLevel::FREE
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
        }
    }

    struct TestChildFrameGuideSharingView;

    impl ChildFrameGuideSharingView for TestChildFrameGuideSharingView {
        fn position_indices(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_counts(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_axes(&self) -> Vec<CoordinationAxis> {
            Vec::new()
        }

        fn axis_guide_visibility_config_for_axis(
            &self,
            _axis: CoordinationAxis,
        ) -> AxisGuideVisibilityConfig {
            AxisGuideVisibilityConfig::auto()
        }
    }
}
