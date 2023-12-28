use crate::error::VegaWgpuError;
use crate::scene::rect::RectInstance;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::rule::RuleItemSpec;

#[derive(Debug, Clone)]
pub struct RuleMark {
    pub instances: Vec<RuleInstance>,
    pub clip: bool,
}

impl RuleMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<RuleItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {
        let instances = RuleInstance::from_specs(spec.items.as_slice(), origin)?;

        Ok(Self {
            instances,
            clip: spec.clip,
        })
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RuleInstance {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub stroke: [f32; 3],
    pub stroke_width: f32,
}

impl RuleInstance {
    pub fn from_spec(item_spec: &RuleItemSpec, origin: [f32; 2]) -> Result<Self, VegaWgpuError> {
        let stroke = if let Some(stroke) = &item_spec.stroke {
            let c = csscolorparser::parse(stroke)?;
            [c.r as f32, c.g as f32, c.b as f32]
        } else {
            [0.5f32, 0.5, 0.5]
        };

        let x0 = item_spec.x + origin[0];
        let y0 = item_spec.y + origin[1];
        let x1 = item_spec.x2.unwrap_or(item_spec.x) + origin[0];
        let y1 = item_spec.y2.unwrap_or(item_spec.y) + origin[1];
        let stroke_width = item_spec.stroke_width.unwrap_or(1.0);

        Ok(Self {
            x0,
            y0,
            x1,
            y1,
            stroke,
            stroke_width,
        })
    }

    pub fn from_specs(
        item_specs: &[RuleItemSpec],
        origin: [f32; 2],
    ) -> Result<Vec<Self>, VegaWgpuError> {
        item_specs
            .iter()
            .map(|item| Self::from_spec(item, origin))
            .collect::<Result<Vec<_>, VegaWgpuError>>()
    }
}
