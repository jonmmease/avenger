use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, Mark, MarkRuntimeContext,
    ScaleTypePreference, coerce_bool_channel_with_renderer, coerce_image_align_channel,
    coerce_image_baseline_channel, coerce_numeric_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common,
};
use avenger_chart_marks::{Image, image_channel_defaults};
use avenger_common::types::{ImageAlign, ImageBaseline};
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{image::SceneImageMark, mark::SceneMark};
use datafusion::{arrow::array::RecordBatch, arrow::datatypes::DataType, common::ScalarValue};
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Image<Cartesian> {
    impl_mark_trait_common!(Image);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianImage {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianImage {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianImage {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "image"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "image",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "height",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "align",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "baseline",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "aspect",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "smooth",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        image_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("image" | "align" | "baseline" | "aspect" | "smooth", _) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianImage {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data);
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;
        let coercer = Coercer::default();

        let image = if let Some(array) = data.and_then(|data| data.column_by_name("image")) {
            coercer.to_image(array).map_err(|error| {
                AvengerChartError::InternalError(format!("Error coercing channel 'image': {error}"))
            })?
        } else if let Some(array) = scalars.column_by_name("image") {
            coercer
                .to_image(array)
                .map(|values| values.to_scalar_if_len_one())
                .map_err(|error| {
                    AvengerChartError::InternalError(format!(
                        "Error coercing channel 'image': {error}"
                    ))
                })?
        } else {
            SceneImageMark::default().image
        };
        let width =
            coerce_numeric_channel_with_renderer(self, data, scalars, "width", &mark_context, 0.0)?;
        let height = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "height",
            &mark_context,
            0.0,
        )?;
        let align = coerce_image_align_channel(data, scalars, "align", ImageAlign::Left)?;
        let baseline =
            coerce_image_baseline_channel(data, scalars, "baseline", ImageBaseline::Top)?;
        let aspect =
            coerce_bool_channel_with_renderer(self, None, scalars, "aspect", &mark_context, true)?
                .first()
                .cloned()
                .unwrap_or(true);
        let smooth =
            coerce_bool_channel_with_renderer(self, None, scalars, "smooth", &mark_context, true)?
                .first()
                .cloned()
                .unwrap_or(true);

        Ok(vec![
            SceneImageMark {
                name: "image".to_string(),
                clip: true,
                len,
                aspect,
                smooth,
                image,
                x: position.x,
                y: position.y,
                width,
                height,
                align,
                baseline,
                indices: None,
                zindex: self.state.zindex,
            }
            .into(),
        ])
    }
}
