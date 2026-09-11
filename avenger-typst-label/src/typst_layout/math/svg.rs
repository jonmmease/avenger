fn path_artifact_from_simple_row(
    font: &MathFont,
    layout: &SimpleRowLayout,
    fill: crate::typst_library::Color,
) -> PathArtifact {
    let Ok(face) = ttf_parser::Face::parse(&font.data, font.face_index) else {
        return PathArtifact {
            logical_width: layout.metrics.width,
            logical_height: layout.metrics.height,
            items: Vec::new(),
            images: Vec::new(),
        };
    };
    let mut items = Vec::new();
    let mut glyph_run = 0usize;

    for atom in &layout.atoms {
        for item in &atom.draw_order {
            match *item {
                LaidOutDrawItem::Glyph(index) => {
                    let Some(glyph) = atom.glyphs.get(index) else {
                        continue;
                    };
                    let path = outline_glyph_path(
                        &face,
                        glyph.glyph_id,
                        glyph.font_size,
                        glyph.x,
                        glyph.y,
                    );
                    if !path.commands.is_empty() {
                        items.push(PathItem {
                            path,
                            kind: PathKind::GlyphOutline {
                                glyph_run,
                                glyph_index: 0,
                            },
                            fill: Some(fill),
                            stroke: None,
                            transform: Transform::IDENTITY,
                            clip: None,
                        });
                    }
                    glyph_run += 1;
                }
                LaidOutDrawItem::Shape(index) => {
                    let Some(shape) = atom.shapes.get(index) else {
                        continue;
                    };
                    items.push(PathItem {
                        path: shape.path.clone(),
                        kind: PathKind::MathShape,
                        fill: None,
                        stroke: Some(Stroke {
                            color: shape.stroke.paint.unwrap_or(fill),
                            width: shape.stroke.width,
                            line_cap: shape.stroke.line_cap,
                            line_join: shape.stroke.line_join,
                            dash: shape.stroke.dash.clone(),
                            miter_limit: shape.stroke.miter_limit,
                        }),
                        transform: Transform {
                            tx: shape.x,
                            ty: shape.y,
                            ..Transform::IDENTITY
                        },
                        clip: None,
                    });
                }
            }
        }
    }

    PathArtifact {
        logical_width: layout.metrics.width,
        logical_height: layout.metrics.height,
        items,
        images: Vec::new(),
    }
}
