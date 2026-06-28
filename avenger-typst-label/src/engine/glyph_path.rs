use crate::paths::{MathPathCommand, MathPathData};

pub(crate) fn outline_glyph_path(
    face: &ttf_parser::Face<'_>,
    glyph_id: ttf_parser::GlyphId,
    font_size: f32,
    x_offset: f32,
    y_offset: f32,
) -> MathPathData {
    let mut builder = GlyphPathBuilder {
        path: MathPathData::default(),
        scale: font_size / face.units_per_em() as f32,
        x_offset,
        y_offset,
    };
    face.outline_glyph(glyph_id, &mut builder);
    builder.path
}

struct GlyphPathBuilder {
    path: MathPathData,
    scale: f32,
    x_offset: f32,
    y_offset: f32,
}

impl GlyphPathBuilder {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.x_offset + x * self.scale,
            self.y_offset - y * self.scale,
        )
    }
}

impl ttf_parser::OutlineBuilder for GlyphPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::MoveTo { x, y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::LineTo { x, y });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.path
            .commands
            .push(MathPathCommand::QuadTo { x1, y1, x, y });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    fn close(&mut self) {
        self.path.commands.push(MathPathCommand::Close);
    }
}
