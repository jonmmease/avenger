use avenger::scene_graph::SceneGraph as RsSceneGraph;
use avenger_vega::scene_graph::VegaSceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasDimensions, PngCanvas};
use avenger_wgpu::register_font_directory as register_font_directory_rs;
use image::{EncodableLayout, ImageOutputFormat};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pythonize::depythonize;
use std::io::Cursor;
use tracing::info_span;
use tracing_subscriber::{fmt, fmt::format::FmtSpan, prelude::*, EnvFilter};

#[pyclass]
pub struct SceneGraph {
    inner: RsSceneGraph,
}

#[pymethods]
impl SceneGraph {
    #[staticmethod]
    fn from_vega_scenegraph(vega_sg: &PyAny) -> PyResult<Self> {
        let vega_sg: VegaSceneGraph =
            info_span!("depythonize").in_scope(|| depythonize(vega_sg))?;
        let inner = vega_sg.to_scene_graph()?;
        Ok(Self { inner })
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_png(&mut self, py: Python, scale: Option<f32>) -> PyResult<PyObject> {
        let img = pollster::block_on(async {
            let mut png_canvas = PngCanvas::new(CanvasDimensions {
                size: [self.inner.width, self.inner.height],
                scale: scale.unwrap_or(1.0),
            })
            .await?;
            png_canvas.set_scene(&self.inner)?;

            png_canvas.render().await
        })?;

        let mut png_data = Vec::new();
        img.write_to(&mut Cursor::new(&mut png_data), ImageOutputFormat::Png)
            .map_err(|err| {
                PyValueError::new_err(format!("Failed to convert image to PNG: {err:?}"))
            })?;

        Ok(PyObject::from(PyBytes::new(py, png_data.as_bytes())))
    }
}

/// Register a directory of fonts for use in subsequent rendering
///
/// Args:
///     font_dir (str): Absolute path to a directory containing font files
#[pyfunction]
#[pyo3(text_signature = "(font_dir)")]
fn register_font_directory(font_dir: &str) {
    register_font_directory_rs(font_dir);
}

#[pymodule]
fn _avenger(_py: Python, m: &PyModule) -> PyResult<()> {
    // Initialize logging controlled by RUST_LOG environment variable
    tracing_subscriber::registry()
        .with(fmt::layer().with_span_events(FmtSpan::CLOSE))
        .with(EnvFilter::from_default_env())
        .init();

    m.add_class::<SceneGraph>()?;
    m.add_function(wrap_pyfunction!(register_font_directory, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
