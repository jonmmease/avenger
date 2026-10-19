//! Fetching and decoding images in the browser.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, ColorSpaceConversion, ImageBitmap, ImageBitmapOptions, OffscreenCanvas,
    OffscreenCanvasRenderingContext2d, PremultiplyAlpha, Response,
};

use crate::cache::Failure;
use crate::RgbaImage;

/// Fetch `url` with the browser's `fetch`, and decode it with `createImageBitmap`, which runs off
/// the main thread. The pixels come back through an `OffscreenCanvas`. A 2D canvas stores its
/// pixels premultiplied, so a translucent pixel can round differently than a native decode does.
pub(crate) async fn fetch_image(url: &str) -> Result<RgbaImage, Failure> {
    if url.starts_with("data:") {
        return RgbaImage::from_str(url, None).map_err(Failure::lasting);
    }
    let window = web_sys::window()
        .ok_or_else(|| Failure::lasting("fetching images needs a browser window"))?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|error| Failure::passing(js_message(url, error)))?
        .dyn_into()
        .map_err(|_| Failure::passing(format!("fetch of {url} gave no response")))?;
    if !response.ok() {
        return Err(Failure::status(response.status(), url));
    }
    let blob: Blob = JsFuture::from(
        response
            .blob()
            .map_err(|error| Failure::passing(js_message(url, error)))?,
    )
    .await
    .map_err(|error| Failure::passing(js_message(url, error)))?
    .unchecked_into();
    let options = ImageBitmapOptions::new();
    options.set_premultiply_alpha(PremultiplyAlpha::None);
    options.set_color_space_conversion(ColorSpaceConversion::None);
    let decode = window
        .create_image_bitmap_with_blob_and_image_bitmap_options(&blob, &options)
        .map_err(|error| Failure::lasting(js_message(url, error)))?;
    let bitmap: ImageBitmap = JsFuture::from(decode)
        .await
        .map_err(|error| Failure::lasting(js_message(url, error)))?
        .unchecked_into();
    let (width, height) = (bitmap.width(), bitmap.height());
    let pixels = read_pixels(&bitmap, width, height)
        .map_err(|error| Failure::passing(js_message(url, error)));
    bitmap.close();
    Ok(RgbaImage {
        width,
        height,
        data: pixels?,
    })
}

fn read_pixels(bitmap: &ImageBitmap, width: u32, height: u32) -> Result<Vec<u8>, JsValue> {
    let canvas = OffscreenCanvas::new(width, height)?;
    let context: OffscreenCanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("no 2D context"))?
        .unchecked_into();
    context.draw_image_with_image_bitmap(bitmap, 0.0, 0.0)?;
    let data = context.get_image_data(0.0, 0.0, f64::from(width), f64::from(height))?;
    Ok(data.data().0)
}

fn js_message(url: &str, error: JsValue) -> String {
    let message = error
        .as_string()
        .or_else(|| {
            error
                .dyn_ref::<js_sys::Error>()
                .map(|error| String::from(error.message()))
        })
        .unwrap_or_else(|| format!("{error:?}"));
    format!("loading {url} failed: {message}")
}
