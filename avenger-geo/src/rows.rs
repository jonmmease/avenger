//! Reading GeoArrow arrays row by row.

use crate::collect::Kind;
use crate::error::AvengerGeoError;
use crate::stream::GeoSink;
use crate::walk::{walk, Rings};
use geo_traits::GeometryTrait;
use geoarrow_array::{downcast_geoarrow_array, GeoArrowArray, GeoArrowArrayAccessor};
use geoarrow_schema::error::GeoArrowResult;

/// A row's geometry, which streams with its rings as the visit asked.
pub(crate) trait RowGeometry {
    fn stream(&self, sink: &mut dyn GeoSink);
    fn kind(&self) -> Kind;
}

struct Row<'a, G> {
    geometry: &'a G,
    rings: Rings,
}

impl<G: GeometryTrait<T = f64>> RowGeometry for Row<'_, G> {
    fn stream(&self, sink: &mut dyn GeoSink) {
        walk(self.geometry, sink, self.rings);
    }
    fn kind(&self) -> Kind {
        Kind::of(self.geometry)
    }
}

/// Visit each row of `geometry` in order: its geometry, which streams its
/// rings as `rings` says, or `None` for a null row. WKB and WKT rows are
/// parsed as they're read.
///
/// # Errors
/// Returns the first row that can't be read.
pub(crate) fn for_each_row(
    geometry: &dyn GeoArrowArray,
    rings: Rings,
    mut visit: impl FnMut(Option<&dyn RowGeometry>),
) -> Result<(), AvengerGeoError> {
    downcast_geoarrow_array!(geometry, visit_rows, rings, &mut visit)
        .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
}

fn visit_rows<'a>(
    array: &'a impl GeoArrowArrayAccessor<'a>,
    rings: Rings,
    visit: &mut dyn FnMut(Option<&dyn RowGeometry>),
) -> GeoArrowResult<()> {
    for row in array.iter() {
        match row {
            None => visit(None),
            Some(value) => visit(Some(&Row {
                geometry: &value?,
                rings,
            })),
        }
    }
    Ok(())
}
