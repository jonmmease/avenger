//! Reading GeoArrow arrays row by row.

use crate::error::AvengerGeoError;
use crate::stream::GeoSink;
use crate::walk::{walk, Rings};
use arrow::array::{Array, ArrayRef, AsArray, Float64Array};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float64Type};
use geo_traits::GeometryTrait;
use geoarrow_array::{downcast_geoarrow_array, GeoArrowArray, GeoArrowArrayAccessor};
use geoarrow_schema::error::GeoArrowResult;

/// A row's geometry, which streams with its rings rewound to RFC 7946.
pub(crate) trait RowGeometry {
    fn stream(&self, sink: &mut dyn GeoSink);
}

struct Rewound<'a, G>(&'a G);

impl<G: GeometryTrait<T = f64>> RowGeometry for Rewound<'_, G> {
    fn stream(&self, sink: &mut dyn GeoSink) {
        walk(self.0, sink, Rings::Rewound);
    }
}

/// Visit each row of `geometry` in order: its geometry, or `None` for a null
/// row. WKB and WKT rows are parsed as they're read.
///
/// # Errors
/// Returns the first row that can't be read.
pub(crate) fn for_each_row(
    geometry: &dyn GeoArrowArray,
    mut visit: impl FnMut(Option<&dyn RowGeometry>),
) -> Result<(), AvengerGeoError> {
    downcast_geoarrow_array!(geometry, visit_rows, &mut visit)
        .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
}

fn visit_rows<'a>(
    array: &'a impl GeoArrowArrayAccessor<'a>,
    visit: &mut dyn FnMut(Option<&dyn RowGeometry>),
) -> GeoArrowResult<()> {
    for row in array.iter() {
        match row {
            None => visit(None),
            Some(value) => visit(Some(&Rewound(&value?))),
        }
    }
    Ok(())
}

/// Longitude and latitude as `Float64` arrays of equal length.
///
/// # Errors
/// Returns an error when either array isn't numeric or their lengths differ.
pub(crate) fn lon_lat(
    lon: &ArrayRef,
    lat: &ArrayRef,
) -> Result<(Float64Array, Float64Array), AvengerGeoError> {
    if lon.len() != lat.len() {
        return Err(AvengerGeoError::Arrow(format!(
            "longitude and latitude have different lengths: {} and {}",
            lon.len(),
            lat.len()
        )));
    }
    let float = |array: &ArrayRef, name: &str| {
        if !array.data_type().is_numeric() {
            return Err(AvengerGeoError::Arrow(format!(
                "{name} must be numeric, not {}",
                array.data_type()
            )));
        }
        cast(array, &DataType::Float64)
            .map(|array| array.as_primitive::<Float64Type>().clone())
            .map_err(|err| AvengerGeoError::Arrow(err.to_string()))
    };
    Ok((float(lon, "longitude")?, float(lat, "latitude")?))
}
