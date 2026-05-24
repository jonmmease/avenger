use std::any::Any;

use avenger_common::value::ScalarOrArray;
use datafusion::common::ScalarValue;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{FacetAxis, OverflowSpaceRequirement, SerializableScalar};

/// Position information for a single band in a band scale.
#[derive(Debug, Clone)]
pub struct BandPosition {
    /// Domain value associated with this band.
    pub value: ScalarValue,
    /// Numeric position of the band start. Use `start()`, `center()`, or
    /// `end()` to make intent explicit.
    position: f32,
    /// Width/height of the band.
    pub bandwidth: f32,
}

impl BandPosition {
    pub fn new(value: ScalarValue, position: f32, bandwidth: f32) -> Self {
        Self {
            value,
            position,
            bandwidth,
        }
    }

    pub fn start(&self) -> f32 {
        self.position
    }

    pub fn center(&self) -> f32 {
        self.position + self.bandwidth / 2.0
    }

    pub fn end(&self) -> f32 {
        self.position + self.bandwidth
    }
}

#[typetag::serde(tag = "type")]
pub trait PlotGeometry: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
}

/// Geometry type for point-based coordinate systems (Cartesian, Polar, ZeroD).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointGeometry {
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
}

#[typetag::serde]
impl PlotGeometry for PointGeometry {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubplotRect {
    /// Facet value for this subplot (e.g., "setosa" for species faceting)
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub value: ScalarValue,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SubplotRect {
    pub fn new(value: ScalarValue, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            value,
            x,
            y,
            width,
            height,
        }
    }

    pub fn scalar_value(&self) -> ScalarValue {
        self.value.clone()
    }
}

impl Default for SubplotRect {
    fn default() -> Self {
        SubplotRect::new(ScalarValue::Null, 0.0, 0.0, 0.0, 0.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubplotGeometry {
    pub rects: Vec<SubplotRect>,
}

impl SubplotGeometry {
    pub fn new(rects: Vec<SubplotRect>) -> Self {
        Self { rects }
    }

    pub fn count(&self) -> usize {
        self.rects.len()
    }

    pub fn rect_at(&self, index: usize) -> Option<&SubplotRect> {
        self.rects.get(index)
    }

    pub fn iter_rects(&self) -> impl Iterator<Item = &SubplotRect> {
        self.rects.iter()
    }

    pub fn from_band_positions(
        iter: impl IntoIterator<Item = BandPosition>,
        axis: FacetAxis,
        cross_extent: f32,
    ) -> Self {
        let rects = iter
            .into_iter()
            .map(|band| {
                let value = band.value.clone();
                let start = band.start();
                let bandwidth = band.bandwidth;
                match axis {
                    FacetAxis::Row => {
                        SubplotRect::new(value.clone(), 0.0, start, cross_extent, bandwidth)
                    }
                    FacetAxis::Column => {
                        SubplotRect::new(value, start, 0.0, bandwidth, cross_extent)
                    }
                }
            })
            .collect();
        Self { rects }
    }
}

#[typetag::serde]
impl PlotGeometry for SubplotGeometry {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Padding specification for coordinate system transforms.
///
/// Facet coordinate systems need padding between subplots to accommodate overflow
/// from axes, legends, and other guides. This enum supports single-dimension
/// padding for FacetRow/FacetCol coordinate systems.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaddingSpec {
    /// Single-dimension padding for row or column facets.
    Single {
        /// Padding in pixels between subplots.
        padding_px: f32,
        /// Overflow measurements for each subplot.
        overflow: Vec<OverflowSpaceRequirement>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::common::ScalarValue;

    #[test]
    fn band_position_methods() {
        let bp = BandPosition::new(ScalarValue::Utf8(Some("test".into())), 100.0, 50.0);

        assert_eq!(bp.start(), 100.0);
        assert_eq!(bp.center(), 125.0);
        assert_eq!(bp.end(), 150.0);
        assert_eq!(bp.bandwidth, 50.0);
    }

    #[test]
    fn band_position_zero_bandwidth() {
        let bp = BandPosition::new(ScalarValue::Utf8(Some("zero".into())), 100.0, 0.0);

        assert_eq!(bp.start(), 100.0);
        assert_eq!(bp.center(), 100.0);
        assert_eq!(bp.end(), 100.0);
    }

    #[test]
    fn band_position_negative_position() {
        let bp = BandPosition::new(ScalarValue::Utf8(Some("negative".into())), -50.0, 20.0);

        assert_eq!(bp.start(), -50.0);
        assert_eq!(bp.center(), -40.0);
        assert_eq!(bp.end(), -30.0);
    }

    #[test]
    fn subplot_geometry_row_helpers() {
        let band_positions = vec![
            BandPosition::new(ScalarValue::from("A"), 0.0, 20.0),
            BandPosition::new(ScalarValue::from("B"), 20.0, 20.0),
        ];

        let geometry =
            SubplotGeometry::from_band_positions(band_positions.clone(), FacetAxis::Row, 100.0);

        assert_eq!(geometry.count(), 2);
        let first = geometry.rect_at(0).unwrap();
        assert_eq!(first.value, ScalarValue::from("A"));
        assert_eq!(first.x, 0.0);
        assert_eq!(first.y, 0.0);
        assert_eq!(first.width, 100.0);
        assert_eq!(first.height, 20.0);

        let second = geometry.rect_at(1).unwrap();
        assert_eq!(second.value, ScalarValue::from("B"));
        assert_eq!(second.y, 20.0);

        let collected: Vec<_> = geometry.iter_rects().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn subplot_geometry_column_helpers() {
        let band_positions = vec![
            BandPosition::new(ScalarValue::from("L"), 5.0, 15.0),
            BandPosition::new(ScalarValue::from("R"), 20.0, 15.0),
        ];

        let geometry =
            SubplotGeometry::from_band_positions(band_positions.clone(), FacetAxis::Column, 80.0);

        let first = geometry.rect_at(0).unwrap();
        assert_eq!(first.value, ScalarValue::from("L"));
        assert_eq!(first.x, 5.0);
        assert_eq!(first.y, 0.0);
        assert_eq!(first.width, 15.0);
        assert_eq!(first.height, 80.0);

        let second = geometry.rect_at(1).unwrap();
        assert_eq!(second.value, ScalarValue::from("R"));
        assert_eq!(second.x, 20.0);
    }
}
