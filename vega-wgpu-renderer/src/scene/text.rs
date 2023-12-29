use crate::error::VegaWgpuError;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::symbol::{SymbolItemSpec, SymbolShape};
use crate::specs::text::{
    FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec, TextItemSpec,
};

#[derive(Debug, Clone)]
pub struct TextMark {
    pub instances: Vec<TextInstance>,
    pub clip: bool,
}

impl TextMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<TextItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {
        let instances = TextInstance::from_specs(spec.items.as_slice(), origin)?;
        Ok(Self {
            instances,
            clip: spec.clip,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TextInstance {
    pub text: String,
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub opacity: f32,
    pub align: TextAlignSpec,
    pub angle: f32,
    pub baseline: TextBaselineSpec,
    pub dx: f32,
    pub dy: f32,
    pub font: String,
    pub font_size: f32,
    pub font_weight: FontWeightSpec,
    pub font_style: FontStyleSpec,
    pub limit: f32,
}

impl TextInstance {
    pub fn from_spec(item_spec: &TextItemSpec, origin: [f32; 2]) -> Result<Self, VegaWgpuError> {
        let color = if let Some(fill) = &item_spec.fill {
            let c = csscolorparser::parse(fill)?;
            [c.r as f32, c.g as f32, c.b as f32]
        } else {
            [0.0, 0.0, 0.0]
        };
        Ok(Self {
            text: item_spec.text.clone(),
            position: [item_spec.x + origin[0], item_spec.y + origin[1]],
            color,
            align: item_spec.align.unwrap_or_default(),
            angle: item_spec.angle.unwrap_or(0.0),
            baseline: item_spec.baseline.unwrap_or_default(),
            dx: item_spec.dx.unwrap_or(0.0),
            dy: item_spec.dy.unwrap_or(0.0),
            opacity: item_spec.fill_opacity.unwrap_or(1.0),
            font: item_spec
                .font
                .clone()
                .unwrap_or_else(|| "Liberation Sans".to_string()),
            font_size: item_spec.fill_opacity.unwrap_or(12.0),
            font_weight: item_spec.font_weight.unwrap_or_default(),
            font_style: item_spec.font_style.unwrap_or_default(),
            limit: item_spec.limit.unwrap_or(0.0),
        })
    }

    pub fn from_specs(
        item_specs: &[TextItemSpec],
        origin: [f32; 2],
    ) -> Result<Vec<Self>, VegaWgpuError> {
        item_specs
            .iter()
            .map(|item| Self::from_spec(item, origin))
            .collect()
    }
}
