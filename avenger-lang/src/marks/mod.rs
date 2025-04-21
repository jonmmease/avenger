use avenger_scales::{scales::coerce::Coercer, utils::ScalarValueUtils};
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::arc::SceneArcMark;
use avenger_scenegraph::marks::area::SceneAreaMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::marks::image::SceneImageMark;
use avenger_scenegraph::marks::line::SceneLineMark;
use avenger_scenegraph::marks::trail::SceneTrailMark;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_common::types::{AreaOrientation, StrokeCap, StrokeJoin};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use datafusion_common::ScalarValue;

use crate::{error::AvengerLangError, task_graph::value::ArrowTable};


pub fn build_rect_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneRectMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneRectMark::default();

    // Compute scalar config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
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
        name: "rect".to_string(),
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

pub fn build_arc_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneArcMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneArcMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let start_angle = encoded_data.column("start_angle").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.start_angle.clone());

    let end_angle = encoded_data.column("end_angle").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.end_angle.clone());

    let outer_radius = encoded_data.column("outer_radius").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.outer_radius.clone());

    let inner_radius = encoded_data.column("inner_radius").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.inner_radius.clone());

    let pad_angle = encoded_data.column("pad_angle").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.pad_angle.clone());

    let corner_radius = encoded_data.column("corner_radius").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.corner_radius.clone());

    let fill = encoded_data.column("fill").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.fill.clone());

    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke.clone());

    let stroke_width = encoded_data.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke_width.clone());
    
    Ok(SceneArcMark {
        name: "arc".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        x,
        y,
        start_angle,
        end_angle,
        outer_radius,
        inner_radius,
        pad_angle,
        corner_radius,
        fill,
        stroke,
        stroke_width,
        indices: None,
        zindex,
    })
}

pub fn build_area_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneAreaMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneAreaMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Get orientation from config
    let orientation = config.column("orientation").ok().and_then(
        |arr| coercer.to_area_orientation(&arr).ok().map(|orientations| {
            orientations.first().copied().unwrap_or(default_mark.orientation)
        })
    ).unwrap_or(default_mark.orientation);

    // Get stroke styling from config
    let stroke_cap = config.column("stroke_cap").ok().and_then(
        |arr| coercer.to_stroke_cap(&arr).ok().map(|caps| {
            caps.first().copied().unwrap_or(default_mark.stroke_cap)
        })
    ).unwrap_or(default_mark.stroke_cap);

    let stroke_join = config.column("stroke_join").ok().and_then(
        |arr| coercer.to_stroke_join(&arr).ok().map(|joins| {
            joins.first().copied().unwrap_or(default_mark.stroke_join)
        })
    ).unwrap_or(default_mark.stroke_join);

    let stroke_dash = config.column("stroke_dash").ok().and_then(
        |arr| coercer.to_stroke_dash(&arr).ok().map(|dashes| {
            dashes.first().cloned()
        })
    ).flatten();
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let x2 = encoded_data.column("x2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x2.clone());

    let y2 = encoded_data.column("y2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y2.clone());

    // Default to all points defined
    let defined = ScalarOrArray::new_scalar(true);

    // Extract fill color (SceneAreaMark.fill is ColorOrGradient, not ScalarOrArray)
    let fill = encoded_data.column("fill").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok().and_then(|colors| {
            colors.first().cloned()
        })
    ).unwrap_or_else(|| default_mark.fill.clone());

    // Extract stroke color (SceneAreaMark.stroke is ColorOrGradient, not ScalarOrArray)
    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok().and_then(|colors| {
            colors.first().cloned()
        })
    ).unwrap_or_else(|| default_mark.stroke.clone());

    // Extract stroke width (SceneAreaMark.stroke_width is f32, not ScalarOrArray)
    let stroke_width = encoded_data.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().and_then(|widths| {
            widths.first().copied()
        })
    ).unwrap_or(default_mark.stroke_width);
    
    Ok(SceneAreaMark {
        name: "area".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        orientation,
        gradients: default_mark.gradients.clone(),
        x,
        y,
        x2,
        y2,
        defined,
        fill,
        stroke,
        stroke_width,
        stroke_cap,
        stroke_join,
        stroke_dash,
        zindex,
    })
}

pub fn build_image_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneImageMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneImageMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();

    let aspect = config.column("aspect").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.aspect)
        })
    ).unwrap_or(default_mark.aspect);

    let smooth = config.column("smooth").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.smooth)
        })
    ).unwrap_or(default_mark.smooth);
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let width = encoded_data.column("width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.width.clone());

    let height = encoded_data.column("height").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.height.clone());

    let align = encoded_data.column("align").ok().and_then(
        |arr| coercer.to_image_align(&arr).ok()
    ).unwrap_or_else(|| default_mark.align.clone());

    let baseline = encoded_data.column("baseline").ok().and_then(
        |arr| coercer.to_image_baseline(&arr).ok()
    ).unwrap_or_else(|| default_mark.baseline.clone());

    let image = encoded_data.column("image").ok().and_then(
        |arr| coercer.to_image(&arr).ok()
    ).unwrap_or_else(|| default_mark.image.clone());
    
    Ok(SceneImageMark {
        name: "image".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        aspect,
        smooth,
        image,
        x,
        y,
        width,
        height,
        align,
        baseline,
        indices: None,
        zindex,
    })
}

pub fn build_line_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneLineMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneLineMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Get stroke styling from config
    let stroke_cap = config.column("stroke_cap").ok().and_then(
        |arr| coercer.to_stroke_cap(&arr).ok().map(|caps| {
            caps.first().copied().unwrap_or(default_mark.stroke_cap)
        })
    ).unwrap_or(default_mark.stroke_cap);

    let stroke_join = config.column("stroke_join").ok().and_then(
        |arr| coercer.to_stroke_join(&arr).ok().map(|joins| {
            joins.first().copied().unwrap_or(default_mark.stroke_join)
        })
    ).unwrap_or(default_mark.stroke_join);

    let stroke_dash = config.column("stroke_dash").ok().and_then(
        |arr| coercer.to_stroke_dash(&arr).ok().map(|dashes| {
            dashes.first().cloned()
        })
    ).flatten();
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    // Extract defined data (whether each point should be included in the line)
    let defined = encoded_data.column("defined").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok()
    ).unwrap_or_else(|| default_mark.defined.clone());

    // Extract stroke color
    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok().and_then(|colors| {
            colors.first().cloned()
        })
    ).unwrap_or_else(|| default_mark.stroke.clone());

    // Extract stroke width
    let stroke_width = encoded_data.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().and_then(|widths| {
            widths.first().copied()
        })
    ).unwrap_or(default_mark.stroke_width);
    
    Ok(SceneLineMark {
        name: "line".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        x,
        y,
        defined,
        stroke,
        stroke_width,
        stroke_cap,
        stroke_join,
        stroke_dash,
        zindex,
    })
}

pub fn build_path_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<ScenePathMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = ScenePathMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Get stroke styling from config
    let stroke_cap = config.column("stroke_cap").ok().and_then(
        |arr| coercer.to_stroke_cap(&arr).ok().map(|caps| {
            caps.first().copied().unwrap_or(default_mark.stroke_cap)
        })
    ).unwrap_or(default_mark.stroke_cap);

    let stroke_join = config.column("stroke_join").ok().and_then(
        |arr| coercer.to_stroke_join(&arr).ok().map(|joins| {
            joins.first().copied().unwrap_or(default_mark.stroke_join)
        })
    ).unwrap_or(default_mark.stroke_join);

    let stroke_width = config.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().copied()
        })
    ).unwrap_or(default_mark.stroke_width);
    
    // Extract data values
    let path = encoded_data.column("path").ok().and_then(
        |arr| coercer.to_path(&arr).ok()
    ).unwrap_or_else(|| default_mark.path.clone());

    let fill = encoded_data.column("fill").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.fill.clone());

    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke.clone());

    let transform = encoded_data.column("transform").ok().and_then(
        |arr| coercer.to_path_transform(&arr).ok()
    ).unwrap_or_else(|| default_mark.transform.clone());
    
    Ok(ScenePathMark {
        name: "path".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        stroke_cap,
        stroke_join,
        stroke_width,
        path,
        fill,
        stroke,
        transform,
        indices: None,
        zindex,
    })
}

pub fn build_rule_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneRuleMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneRuleMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Get stroke dash from config
    let stroke_dash = config.column("stroke_dash").ok().and_then(
        |arr| coercer.to_stroke_dash(&arr).ok()
    );
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let x2 = encoded_data.column("x2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x2.clone());

    let y2 = encoded_data.column("y2").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y2.clone());

    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke.clone());

    let stroke_width = encoded_data.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke_width.clone());

    let stroke_cap = encoded_data.column("stroke_cap").ok().and_then(
        |arr| coercer.to_stroke_cap(&arr).ok()
    ).unwrap_or_else(|| default_mark.stroke_cap.clone());
    
    Ok(SceneRuleMark {
        name: "rule".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        stroke_dash,
        x,
        y,
        x2,
        y2,
        stroke,
        stroke_width,
        stroke_cap,
        indices: None,
        zindex,
    })
}

pub fn build_symbol_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneSymbolMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneSymbolMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();

    let stroke_width = config.column("stroke_width").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().and_then(|nums| {
            nums.first().copied()
        })
    );

    // Get symbol shapes from config
    let shapes = config.column("shapes").ok().and_then(
        |arr| coercer.to_symbol_shapes(&ScalarValue::try_from_array(&arr, 0).ok()?).ok()
    ).unwrap_or_else(|| default_mark.shapes.clone());
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let shape_index = encoded_data.column("shape_index").ok().and_then(
        |arr| coercer.to_usize(&arr).ok()
    ).unwrap_or_else(|| default_mark.shape_index.clone());

    let fill = encoded_data.column("fill").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.fill.clone());

    let size = encoded_data.column("size").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.size.clone());

    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.stroke.clone());

    let angle = encoded_data.column("angle").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.angle.clone());
    
    Ok(SceneSymbolMark {
        name: "symbol".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        shapes,
        stroke_width,
        shape_index,
        x,
        y,
        fill,
        size,
        stroke,
        angle,
        indices: None,
        zindex,
        x_adjustment: None,
        y_adjustment: None,
    })
}

pub fn build_text_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneTextMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneTextMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Extract data values
    let text = encoded_data.column("text").ok().and_then(
        |arr| coercer.to_string(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.text.clone());

    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let align = encoded_data.column("align").ok().and_then(
        |arr| coercer.to_text_align(&arr).ok()
    ).unwrap_or_else(|| default_mark.align.clone());

    let baseline = encoded_data.column("baseline").ok().and_then(
        |arr| coercer.to_text_baseline(&arr).ok()
    ).unwrap_or_else(|| default_mark.baseline.clone());

    let angle = encoded_data.column("angle").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.angle.clone());

    let color = encoded_data.column("color").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.color.clone());

    let font = encoded_data.column("font").ok().and_then(
        |arr| coercer.to_string(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.font.clone());

    let font_size = encoded_data.column("font_size").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.font_size.clone());

    let font_weight = encoded_data.column("font_weight").ok().and_then(
        |arr| coercer.to_font_weight(&arr).ok()
    ).unwrap_or_else(|| default_mark.font_weight.clone());

    let font_style = encoded_data.column("font_style").ok().and_then(
        |arr| coercer.to_font_style(&arr).ok()
    ).unwrap_or_else(|| default_mark.font_style.clone());

    let limit = encoded_data.column("limit").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.limit.clone());
    
    Ok(SceneTextMark {
        name: "text".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        text,
        x,
        y,
        align,
        baseline,
        angle,
        color,
        font,
        font_size,
        font_weight,
        font_style,
        limit,
        indices: None,
        zindex,
    })
}

pub fn build_trail_mark(encoded_data: &ArrowTable, config: &ArrowTable) -> Result<SceneTrailMark, AvengerLangError> {
    // Build coercer
    let coercer = Coercer::default();

    // Make a default mark for fallback logic below
    let default_mark = SceneTrailMark::default();

    // Extract config values
    let clip = config.column("clip").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok().map(|bools| {
            bools.first().map(|v| *v).unwrap_or(default_mark.clip)
        })
    ).unwrap_or(default_mark.clip);

    let zindex = config.column("zindex").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok().map(|nums| {
            nums.first().map(|v| *v as i32)
        })
    ).flatten();
    
    // Extract data values
    let x = encoded_data.column("x").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.x.clone());

    let y = encoded_data.column("y").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.y.clone());

    let size = encoded_data.column("size").ok().and_then(
        |arr| coercer.to_numeric(&arr, None).ok()
    ).unwrap_or_else(|| default_mark.size.clone());

    // Extract defined data (whether each point should be included in the trail)
    let defined = encoded_data.column("defined").ok().and_then(
        |arr| coercer.to_boolean(&arr).ok()
    ).unwrap_or_else(|| default_mark.defined.clone());

    // Extract stroke color
    let stroke = encoded_data.column("stroke").ok().and_then(
        |arr| coercer.to_color(&arr, None).ok().and_then(|colors| {
            colors.first().cloned()
        })
    ).unwrap_or_else(|| default_mark.stroke.clone());
    
    Ok(SceneTrailMark {
        name: "trail".to_string(),
        clip,
        len: encoded_data.num_rows() as u32,
        gradients: default_mark.gradients.clone(),
        stroke,
        x,
        y,
        size,
        defined,
        zindex,
    })
}
