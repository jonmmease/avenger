use anyhow::Result;
use arrow::array::Float32Array;
use avenger_scales_datafusion::BuiltinScale;
use avenger_selection::*;
use datafusion::logical_expr::col;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Scatter,
    Airline,
}

#[derive(Clone, Debug)]
pub struct Selections {
    pub state: SelectionSet,
    pub scatter: ProducerDefinition,
    pub airline: ProducerDefinition,
    pub carriers: BTreeSet<String>,
    pub raw_brush: Option<[f64; 4]>,
}
impl Selections {
    pub fn new(
        carriers: &[String],
        domains: [[f32; 2]; 2],
        size: [f32; 2],
        exact: bool,
    ) -> Result<Self> {
        let selection = SelectionId::new("filters")?;
        let scatter = ProducerDefinition::new(
            selection.clone(),
            ProducerId::new("brush")?,
            ViewId::new("scatter")?,
            [
                Projection::new(ProjectionId::new("x")?, col("dep_delay"))?,
                Projection::new(ProjectionId::new("y")?, col("arr_delay"))?,
            ],
        )?;
        let airline = ProducerDefinition::new(
            selection.clone(),
            ProducerId::new("carriers")?,
            ViewId::new("airline")?,
            [Projection::new(
                ProjectionId::new("carrier")?,
                col("carrier"),
            )?],
        )?;
        let mut result = Self {
            state: SelectionSet::new([(selection, Resolution::Intersect)])?,
            scatter,
            airline,
            carriers: carriers.iter().cloned().collect(),
            raw_brush: None,
        };
        result.regrid(domains, size, exact)?;
        result.select_carriers(result.carriers.clone())?;
        Ok(result)
    }
    pub fn producer(&self, focus: Focus) -> &ProducerDefinition {
        match focus {
            Focus::Scatter => &self.scatter,
            Focus::Airline => &self.airline,
        }
    }
    pub fn consumer(&self, view: &str) -> Result<ConsumerFilter> {
        Ok(ConsumerFilter::new(
            ViewId::new(view)?,
            SelectionFilter::cross_filter([self.scatter.selection()]),
        ))
    }
    pub fn brush(&mut self, bounds: Option<[f64; 4]>) -> Result<()> {
        self.raw_brush = bounds;
        self.state = if let Some([x0, x1, y0, y1]) = bounds {
            self.state.set(
                &self.scatter,
                SelectionValue::tuple([
                    (ProjectionId::new("x")?, ValueTest::range(x0..x1)),
                    (ProjectionId::new("y")?, ValueTest::range(y0..y1)),
                ]),
            )?
        } else {
            self.state.clear(&self.scatter)?
        };
        Ok(())
    }
    pub fn select_carriers(&mut self, carriers: BTreeSet<String>) -> Result<()> {
        self.state = self.state.set(
            &self.airline,
            SelectionValue::tuples(carriers.iter().map(|c| {
                [(
                    ProjectionId::new("carrier").expect("fixed id"),
                    ValueTest::equal(c.as_str()),
                )]
            })),
        )?;
        self.carriers = carriers;
        Ok(())
    }
    pub fn toggle(&mut self, carrier: &str) -> Result<()> {
        let mut carriers = self.carriers.clone();
        if !carriers.remove(carrier) {
            carriers.insert(carrier.to_owned());
        }
        self.select_carriers(carriers)
    }
    pub fn regrid(&mut self, domains: [[f32; 2]; 2], size: [f32; 2], exact: bool) -> Result<()> {
        let grids = if exact {
            vec![]
        } else {
            [
                ("x", domains[0], [0., size[0]]),
                ("y", domains[1], [size[1], 0.]),
            ]
            .into_iter()
            .map(|(id, domain, range)| {
                Ok((
                    ProjectionId::new(id)?,
                    PixelGrid::new(
                        BuiltinScale::Linear,
                        Arc::new(Float32Array::from(domain.to_vec())),
                        Arc::new(Float32Array::from(range.to_vec())),
                        Default::default(),
                        0.,
                        2.,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
        };
        self.scatter = self.scatter.with_pixel_grids(grids)?;
        self.brush(self.raw_brush)
    }
}
