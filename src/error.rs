use thiserror::Error;

#[derive(Error, Debug)]
pub enum VegaWgpuError {
    #[error("css color parse error")]
    InvalidColor(#[from] csscolorparser::ParseColorError),
}
