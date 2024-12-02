use crate::band::opts::BandScaleOptions;

#[derive(Debug, Clone, Copy, Default)]
pub struct PointScaleOptions {
    pub range_offset: Option<f32>,
}

impl From<&PointScaleOptions> for BandScaleOptions {
    fn from(opts: &PointScaleOptions) -> Self {
        Self {
            range_offset: opts.range_offset,
            ..Default::default()
        }
    }
}
