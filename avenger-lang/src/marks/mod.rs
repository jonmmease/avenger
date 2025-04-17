use avenger_scales::{scales::coerce::Coercer, utils::ScalarValueUtils};
use avenger_scenegraph::marks::rect::SceneRectMark;

use crate::{error::AvengerLangError, task_graph::value::ArrowTable};


pub fn build_rect_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneRectMark, AvengerLangError> {

    // Build coercer
    let coercer = Coercer::default();

    // Convert config table to map of scalar values
    let config = config.first_row_to_scalars()?;

    // Make a default mark for fallback logic below
    let default_mark = SceneRectMark::default();

    // Compute scalar config values
    let clip = config.get("clip").and_then(
        |s| s.as_boolean().ok()
    ).unwrap_or(default_mark.clip);

    let zindex = config.get("zindex").and_then(
        |s| s.as_i32().ok()
    );
    
    // Compute encoded data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let width = encoded_data.column("width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    );
    let height = encoded_data.column("height").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    );
    let x2 = encoded_data.column("x2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    );
    let y2 = encoded_data.column("y2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    );
    let fill = encoded_data.column("fill").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.fill.clone());
    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke.clone());

    let stroke_width = encoded_data.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke_width.clone());

    let corner_radius = encoded_data.column("corner_radius").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.corner_radius.clone());
    
    Ok(SceneRectMark {
        name: "rect".to_string(), // Do we need a name here?
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        x,
        y,
        width,
        height,
        x2,
        y2,
        fill,
        stroke,
        stroke_width,
        corner_radius,
        indices: None,
        zindex,
    })
}
