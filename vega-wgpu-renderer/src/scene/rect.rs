use crate::error::VegaWgpuError;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::rect::RectItemSpec;

#[derive(Debug, Clone)]
pub struct RectMark {
    pub instances: Vec<RectInstance>,
    pub clip: bool,
}

impl RectMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<RectItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {
        let instances = RectInstance::from_specs(spec.items.as_slice(), origin)?;

        Ok(Self {
            instances,
            clip: spec.clip,
        })
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub width: f32,
    pub height: f32,
}

impl RectInstance {
    pub fn from_spec(item_spec: &RectItemSpec, origin: [f32; 2]) -> Result<Self, VegaWgpuError> {
        // TODO: x2, y2
        let color = if let Some(fill) = &item_spec.fill {
            let c = csscolorparser::parse(fill)?;
            [c.r as f32, c.g as f32, c.b as f32]
        } else {
            [0.5f32, 0.5, 0.5]
        };

        Ok(Self {
            position: [item_spec.x + origin[0], item_spec.y + origin[1]],
            color,
            width: item_spec.width.unwrap(),
            height: item_spec.height.unwrap(),
        })
    }

    pub fn from_specs(
        item_specs: &[RectItemSpec],
        origin: [f32; 2],
    ) -> Result<Vec<Self>, VegaWgpuError> {
        item_specs
            .iter()
            .map(|item| Self::from_spec(item, origin))
            .collect::<Result<Vec<_>, VegaWgpuError>>()
    }
}
