use crate::error::VegaWgpuError;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::symbol::{SymbolItemSpec, SymbolShape};

#[derive(Debug, Clone)]
pub struct SymbolMark {
    pub instances: Vec<SymbolInstance>,
    pub shape: SymbolShape,
    pub clip: bool,
}

impl SymbolMark {
    pub fn from_spec(spec: &MarkContainerSpec<SymbolItemSpec>) -> Result<Self, VegaWgpuError> {
        let instances = SymbolInstance::from_specs(spec.items.as_slice())?;

        // For now, grab the shape of the first item and use this for all items.
        // Eventually we'll need to handle marks with mixed symbols
        let first_shape = spec.items.get(0).and_then(|item| item.shape).unwrap_or_default();

        Ok(Self {
            instances,
            shape: first_shape,
            clip: spec.clip,
        })
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SymbolInstance {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub size: f32,
}

impl SymbolInstance {
    pub fn from_spec(item_spec: &SymbolItemSpec) -> Result<Self, VegaWgpuError> {
        let color = if let Some(fill) = &item_spec.fill {
            let c = csscolorparser::parse(fill)?;
            [c.r as f32, c.g as f32, c.b as f32]
        } else {
            [0.5f32, 0.5, 0.5]
        };
        Ok(Self {
            position: [item_spec.x, item_spec.y],
            color,
            size: item_spec.size.unwrap_or(20.0),
        })
    }

    pub fn from_specs(item_specs: &[SymbolItemSpec]) -> Result<Vec<Self>, VegaWgpuError> {
        item_specs.iter().map(Self::from_spec).collect()
    }
}
