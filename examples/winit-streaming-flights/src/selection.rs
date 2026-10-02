use anyhow::{Context, Result};
use arrow::array::Float32Array;
use avenger_scales::scales::{ConfiguredScale, linear::LinearScale};
use avenger_scales_datafusion::BuiltinScale;
use avenger_selection::*;
use datafusion::logical_expr::col;
use std::sync::Arc;

pub struct Plot {
    pub name: &'static str,
    pub title: &'static str,
    pub domain: [f32; 2],
    pub step: f32,
}
impl Plot {
    pub fn scale(&self, width: f32) -> ConfiguredScale {
        LinearScale::configured((self.domain[0], self.domain[1]), (0., width))
    }
}
pub const PLOTS: [Plot; 3] = [
    Plot {
        name: "delay",
        title: "Arrival Delay (min)",
        domain: [-60., 190.],
        step: 10.,
    },
    Plot {
        name: "time",
        title: "Scheduled Departure (hour)",
        domain: [0., 24.],
        step: 1.,
    },
    Plot {
        name: "distance",
        title: "Flight Distance (miles)",
        domain: [0., 5000.],
        step: 200.,
    },
];

#[derive(Clone)]
pub struct Selections {
    pub state: SelectionSet,
    pub producers: Vec<ProducerDefinition>,
    pub bounds: [Option<[f64; 2]>; 3],
}
impl Selections {
    pub fn new(width: f32) -> Result<Self> {
        let selection = SelectionId::new("brush")?;
        let producers = PLOTS
            .iter()
            .map(|p| {
                let projection = ProjectionId::new("value")?;
                ProducerDefinition::new(
                    selection.clone(),
                    ProducerId::new(p.name)?,
                    ViewId::new(p.name)?,
                    [Projection::new(projection.clone(), col(p.name))?],
                )?
                .with_pixel_grids([(
                    projection,
                    PixelGrid::new(
                        BuiltinScale::Linear,
                        Arc::new(Float32Array::from(p.domain.to_vec())),
                        Arc::new(Float32Array::from(vec![0., width])),
                        Default::default(),
                        0.,
                        1.,
                    )?,
                )])
            })
            .collect::<avenger_selection::Result<Vec<_>>>()?;
        Ok(Self {
            state: SelectionSet::new([(selection, Resolution::Intersect)])?,
            producers,
            bounds: [None; 3],
        })
    }
    pub fn consumer(&self, plot: usize) -> Result<ConsumerFilter> {
        Ok(ConsumerFilter::new(
            ViewId::new(PLOTS.get(plot).map_or("carriers", |p| p.name))?,
            SelectionFilter::cross_filter([self.producers[0].selection()]),
        ))
    }
    pub fn set(&mut self, plot: usize, bounds: Option<[f64; 2]>) -> Result<()> {
        let producer = &self.producers[plot];
        self.state = if let Some([lo, hi]) = bounds {
            self.state.set(
                producer,
                SelectionValue::tuple([(ProjectionId::new("value")?, ValueTest::range(lo..hi))]),
            )?
        } else {
            self.state.clear(producer)?
        };
        self.bounds[plot] = bounds;
        Ok(())
    }
    pub fn pixels(&self, plot: usize, bounds: [f64; 2]) -> Result<[f32; 2]> {
        let grid = self.producers[plot]
            .pixel_grid(&ProjectionId::new("value")?)
            .unwrap();
        let pixel = |value: f64| -> Result<f32> {
            Ok(
                (grid.cell(&value.into())?.context("finite brush bound")? as f64 * grid.size()
                    + grid.origin()) as f32,
            )
        };
        Ok([pixel(bounds[0])?, pixel(bounds[1])?])
    }
}
