//! The text form of a pattern. The grammar is in the documentation of [`super`].

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use avenger_color::parse_color_string;
use thiserror::Error;

use super::{
    default_pattern_opacity, PatternAnchor, PatternFill, PatternInk, PatternLayer,
    PatternLayerKind, PatternLayerOperation, PatternPreset, PatternSymbol, PatternSymbolShape,
    PatternValidationError, StripeDash, StripeLayer, SymbolLattice2d, SymbolLayer, SymbolPaint,
};

/// An error in the text of a pattern.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatternParseError {
    /// The text does not follow the grammar. The column counts characters from 1.
    #[error("at column {column}: {message}")]
    Syntax { column: usize, message: String },
    /// The text follows the grammar, and the pattern is invalid.
    #[error(transparent)]
    Invalid(#[from] PatternValidationError),
}

impl FromStr for PatternFill {
    type Err = PatternParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let pattern = Parser::new(text)?.pattern()?;
        pattern.validate()?;
        Ok(pattern)
    }
}

impl Display for PatternFill {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some((preset, scale)) = PatternPreset::matching(&self.layers) {
            f.write_str(preset.name())?;
            if scale != 1.0 {
                write!(f, " {scale}")?;
            }
        } else if self.layers.is_empty() {
            f.write_str("none")?;
        } else {
            write_layers(f, &self.layers)?;
        }
        if self.ink != PatternInk::default() {
            f.write_str("; ink ")?;
            let opacity = match &self.ink {
                PatternInk::AutoContrast { opacity } => {
                    f.write_str("auto")?;
                    *opacity
                }
                PatternInk::Solid { color, opacity } => {
                    write_color(f, *color)?;
                    *opacity
                }
            };
            if opacity != default_pattern_opacity() {
                write!(f, " {opacity}")?;
            }
        }
        match self.anchor {
            PatternAnchor::Group => Ok(()),
            PatternAnchor::Mark => f.write_str("; anchor mark"),
            PatternAnchor::Scene => f.write_str("; anchor scene"),
        }
    }
}

fn write_layers(f: &mut Formatter<'_>, layers: &[PatternLayer]) -> fmt::Result {
    for (index, layer) in layers.iter().enumerate() {
        let op = match layer.operation {
            PatternLayerOperation::Add => "+",
            PatternLayerOperation::Subtract => "-",
            PatternLayerOperation::Xor => "^",
        };
        if index > 0 {
            write!(f, " {op} ")?;
        } else if layer.operation != PatternLayerOperation::Add {
            write!(f, "{op} ")?;
        }
        match &layer.kind {
            PatternLayerKind::Stripe(stripe) => write_stripes(f, stripe)?,
            PatternLayerKind::Symbol(symbol) => write_symbols(f, symbol)?,
        }
    }
    Ok(())
}

fn write_stripes(f: &mut Formatter<'_>, stripe: &StripeLayer) -> fmt::Result {
    write!(
        f,
        "stripes {} {} {}",
        stripe.angle, stripe.spacing, stripe.stroke_width
    )?;
    if let Some(dash) = &stripe.dash {
        write!(f, " dash {} {}", dash.length, dash.gap)?;
        if dash.phase != 0.0 {
            write!(f, " {}", dash.phase)?;
        }
    }
    if stripe.phase != 0.0 {
        write!(f, " offset {}", stripe.phase)?;
    }
    Ok(())
}

fn write_symbols(f: &mut Formatter<'_>, layer: &SymbolLayer) -> fmt::Result {
    let symbol = &layer.symbol;
    write!(f, "{} {}", symbol.shape.name(), symbol.size)?;
    if symbol.rotation != 0.0 {
        write!(f, " rotate {}", symbol.rotation)?;
    }
    if let SymbolPaint::Open { stroke_width } = layer.paint {
        write!(f, " open {stroke_width}")?;
    }
    let lattice = &layer.lattice;
    let at = if lattice.v_angle == lattice.u_angle + GRID_STEP_ANGLE {
        write!(f, " on grid {}", lattice.u_spacing)?;
        if lattice.v_spacing != lattice.u_spacing {
            write!(f, " {}", lattice.v_spacing)?;
        }
        true
    } else if lattice.v_angle == lattice.u_angle + HEX_STEP_ANGLE
        && lattice.v_spacing == lattice.u_spacing
    {
        write!(f, " on hex {}", lattice.u_spacing)?;
        true
    } else {
        write!(
            f,
            " on lattice {} {} {} {}",
            lattice.u_spacing, lattice.u_angle, lattice.v_spacing, lattice.v_angle
        )?;
        false
    };
    if at && lattice.u_angle != 0.0 {
        write!(f, " at {}", lattice.u_angle)?;
    }
    if lattice.u_phase != 0.0 || lattice.v_phase != 0.0 {
        write!(f, " offset {} {}", lattice.u_phase, lattice.v_phase)?;
    }
    Ok(())
}

/// An opaque color as `#rrggbb`, and another as `rgba(…)` with its alpha in full.
fn write_color(f: &mut Formatter<'_>, [r, g, b, a]: [f32; 4]) -> fmt::Result {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a == 1.0 {
        write!(f, "#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
    } else {
        write!(f, "rgba({}, {}, {}, {a})", byte(r), byte(g), byte(b))
    }
}

/// The angle from the first step of a `grid` lattice to its second.
const GRID_STEP_ANGLE: f32 = 90.0;
/// The angle from the first step of a `hex` lattice to its second.
const HEX_STEP_ANGLE: f32 = 60.0;

const SHAPE_EXPECTED: &str = "`stripes` or a symbol shape, such as `circle`";

#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    column: usize,
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    next: usize,
    /// The column after the last character, where an error at the end points.
    end_column: usize,
}

impl<'a> Parser<'a> {
    /// Splits `text` at whitespace, outside parentheses, and around each `;`.
    fn new(text: &'a str) -> Result<Self, PatternParseError> {
        let column_of = |index: usize| text[..index].chars().count() + 1;
        let mut tokens = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some(&(start, ch)) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
                continue;
            }
            let mut end = start + ch.len_utf8();
            chars.next();
            if ch != ';' {
                let mut depth = usize::from(ch == '(');
                if ch == ')' {
                    return Err(syntax(column_of(start), "a `)` without its `(`"));
                }
                while let Some(&(index, ch)) = chars.peek() {
                    if depth == 0 && (ch.is_whitespace() || ch == ';') {
                        break;
                    }
                    match ch {
                        '(' => depth += 1,
                        ')' if depth == 0 => {
                            return Err(syntax(column_of(index), "a `)` without its `(`"))
                        }
                        ')' => depth -= 1,
                        _ => {}
                    }
                    end = index + ch.len_utf8();
                    chars.next();
                }
                if depth > 0 {
                    return Err(syntax(column_of(start), "a `(` without its `)`"));
                }
            }
            tokens.push(Token {
                text: &text[start..end],
                column: column_of(start),
            });
        }
        Ok(Self {
            tokens,
            next: 0,
            end_column: text.chars().count() + 1,
        })
    }

    fn pattern(&mut self) -> Result<PatternFill, PatternParseError> {
        let mut pattern = PatternFill::default();
        // A preset and `none` are whole patterns, which no operator extends.
        let mut end = "`+`, `-`, `^`, `;`, or the end";
        let preset = self
            .peek()
            .and_then(|token| PatternPreset::from_name(token.text));
        if let Some(preset) = preset {
            self.next += 1;
            let scale = match self.peek() {
                Some(token) if token.text != ";" => {
                    self.next += 1;
                    finite(token.text)
                        .filter(|scale| *scale > 0.0)
                        .ok_or_else(|| unexpected(token, "a positive scale, `;`, or the end"))?
                }
                _ => 1.0,
            };
            pattern.layers = preset.layers(scale);
            end = "`;` or the end";
        } else if self.eat("none") {
            end = "`;` or the end";
        } else {
            let first = self.operation().unwrap_or_default();
            pattern.layers.push(self.layer()?.with_operation(first));
            while let Some(operation) = self.operation() {
                pattern.layers.push(self.layer()?.with_operation(operation));
            }
        }
        let (mut ink, mut anchor) = (None, None);
        while self.eat(";") {
            let token = self.take("`ink` or `anchor`")?;
            match token.text {
                "ink" if ink.is_none() => ink = Some(self.ink()?),
                "anchor" if anchor.is_none() => anchor = Some(self.anchor()?),
                "ink" | "anchor" => return Err(repeated(token)),
                _ => return Err(unexpected(token, "`ink` or `anchor`")),
            }
        }
        if let Some(token) = self.peek() {
            return Err(unexpected(token, end));
        }
        pattern.ink = ink.unwrap_or_default();
        pattern.anchor = anchor.unwrap_or_default();
        Ok(pattern)
    }

    fn operation(&mut self) -> Option<PatternLayerOperation> {
        let operation = match self.peek()?.text {
            "+" => PatternLayerOperation::Add,
            "-" => PatternLayerOperation::Subtract,
            "^" => PatternLayerOperation::Xor,
            _ => return None,
        };
        self.next += 1;
        Some(operation)
    }

    fn layer(&mut self) -> Result<PatternLayer, PatternParseError> {
        let token = self.take(SHAPE_EXPECTED)?;
        if token.text == "stripes" {
            return Ok(self.stripes()?.into());
        }
        match PatternSymbolShape::ALL
            .into_iter()
            .find(|shape| shape.name() == token.text)
        {
            Some(shape) => Ok(self.symbols(shape)?.into()),
            None => Err(unexpected(token, SHAPE_EXPECTED)),
        }
    }

    fn stripes(&mut self) -> Result<StripeLayer, PatternParseError> {
        let angle = self.number("a stripe angle")?;
        let spacing = self.number("a stripe spacing")?;
        let width = self.number("a stripe width")?;
        let mut stripes = StripeLayer::new(angle, spacing, width);
        let mut offset = None;
        while let Some(token) = self.peek() {
            match token.text {
                "dash" if stripes.dash.is_none() => {
                    self.next += 1;
                    stripes.dash = Some(StripeDash {
                        length: self.number("a dash length")?,
                        gap: self.number("a dash gap")?,
                        phase: self.optional_number().unwrap_or(0.0),
                    });
                }
                "offset" if offset.is_none() => {
                    self.next += 1;
                    offset = Some(self.number("a stripe offset")?);
                }
                "dash" | "offset" => return Err(repeated(token)),
                _ => break,
            }
        }
        stripes.phase = offset.unwrap_or(0.0);
        Ok(stripes)
    }

    fn symbols(&mut self, shape: PatternSymbolShape) -> Result<SymbolLayer, PatternParseError> {
        let size = self.number("a symbol size")?;
        let (mut rotation, mut paint) = (None, None);
        loop {
            let token = self.take("`rotate`, `open`, or `on` and a lattice")?;
            match token.text {
                "rotate" if rotation.is_none() => {
                    rotation = Some(self.number("a symbol rotation")?);
                }
                "open" if paint.is_none() => {
                    paint = Some(SymbolPaint::Open {
                        stroke_width: self.number("a symbol stroke width")?,
                    });
                }
                "rotate" | "open" => return Err(repeated(token)),
                "on" => break,
                _ => return Err(unexpected(token, "`rotate`, `open`, or `on` and a lattice")),
            }
        }
        Ok(SymbolLayer {
            lattice: self.lattice()?,
            symbol: PatternSymbol {
                shape,
                size,
                rotation: rotation.unwrap_or(0.0),
            },
            paint: paint.unwrap_or(SymbolPaint::Filled),
        })
    }

    fn lattice(&mut self) -> Result<SymbolLattice2d, PatternParseError> {
        let token = self.take("`grid`, `hex`, or `lattice`")?;
        let (u_spacing, v_spacing, step_angle) = match token.text {
            "grid" => {
                let u_spacing = self.number("a grid spacing")?;
                let v_spacing = self.optional_number().unwrap_or(u_spacing);
                (u_spacing, v_spacing, Some(GRID_STEP_ANGLE))
            }
            "hex" => {
                let spacing = self.number("a hex spacing")?;
                (spacing, spacing, Some(HEX_STEP_ANGLE))
            }
            "lattice" => (0.0, 0.0, None),
            _ => return Err(unexpected(token, "`grid`, `hex`, or `lattice`")),
        };
        let mut lattice = match step_angle {
            Some(step_angle) => SymbolLattice2d {
                u_spacing,
                u_angle: 0.0,
                v_spacing,
                v_angle: step_angle,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            None => SymbolLattice2d {
                u_spacing: self.number("the spacing of the first step")?,
                u_angle: self.number("the angle of the first step")?,
                v_spacing: self.number("the spacing of the second step")?,
                v_angle: self.number("the angle of the second step")?,
                u_phase: 0.0,
                v_phase: 0.0,
            },
        };
        let (mut at, mut offset) = (None, None);
        while let Some(token) = self.peek() {
            match (token.text, step_angle) {
                ("at", Some(step_angle)) if at.is_none() => {
                    self.next += 1;
                    let angle = self.number("a lattice angle")?;
                    lattice.u_angle = angle;
                    lattice.v_angle = angle + step_angle;
                    at = Some(angle);
                }
                ("offset", _) if offset.is_none() => {
                    self.next += 1;
                    lattice.u_phase = self.number("the offset along the first step")?;
                    lattice.v_phase = self.number("the offset along the second step")?;
                    offset = Some(());
                }
                ("at", Some(_)) | ("offset", _) => return Err(repeated(token)),
                _ => break,
            }
        }
        Ok(lattice)
    }

    fn ink(&mut self) -> Result<PatternInk, PatternParseError> {
        let token = self.take("`auto` or a CSS color")?;
        let color = match token.text {
            "auto" => None,
            text => Some(
                parse_color_string(text)
                    .ok_or_else(|| unexpected(token, "`auto` or a CSS color"))?,
            ),
        };
        let opacity = self
            .optional_number()
            .unwrap_or_else(default_pattern_opacity);
        Ok(match color {
            None => PatternInk::AutoContrast { opacity },
            Some(color) => PatternInk::Solid { color, opacity },
        })
    }

    fn anchor(&mut self) -> Result<PatternAnchor, PatternParseError> {
        let token = self.take("`group`, `mark`, or `scene`")?;
        match token.text {
            "group" => Ok(PatternAnchor::Group),
            "mark" => Ok(PatternAnchor::Mark),
            "scene" => Ok(PatternAnchor::Scene),
            _ => Err(unexpected(token, "`group`, `mark`, or `scene`")),
        }
    }

    fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.next).copied()
    }

    /// Takes the next token if its text is `text`.
    fn eat(&mut self, text: &str) -> bool {
        let found = self.peek().is_some_and(|token| token.text == text);
        self.next += usize::from(found);
        found
    }

    /// Takes the next token, which the grammar requires.
    fn take(&mut self, expected: &str) -> Result<Token<'a>, PatternParseError> {
        let token = self.peek().ok_or_else(|| {
            syntax(
                self.end_column,
                &format!("expected {expected}, found the end"),
            )
        })?;
        self.next += 1;
        Ok(token)
    }

    fn number(&mut self, expected: &str) -> Result<f32, PatternParseError> {
        let token = self.take(expected)?;
        finite(token.text).ok_or_else(|| unexpected(token, expected))
    }

    /// Takes the next token if it is a number.
    fn optional_number(&mut self) -> Option<f32> {
        let number = finite(self.peek()?.text)?;
        self.next += 1;
        Some(number)
    }
}

fn finite(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|number| number.is_finite())
}

fn syntax(column: usize, message: &str) -> PatternParseError {
    PatternParseError::Syntax {
        column,
        message: message.to_string(),
    }
}

fn unexpected(token: Token<'_>, expected: &str) -> PatternParseError {
    syntax(
        token.column,
        &format!("expected {expected}, found `{}`", token.text),
    )
}

fn repeated(token: Token<'_>) -> PatternParseError {
    syntax(token.column, &format!("`{}` appears twice", token.text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> PatternFill {
        text.parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    fn stripes(angle: f32, spacing: f32, width: f32) -> PatternLayer {
        StripeLayer::new(angle, spacing, width).into()
    }

    fn lattice(u_spacing: f32, u_angle: f32, v_spacing: f32, v_angle: f32) -> SymbolLattice2d {
        SymbolLattice2d {
            u_spacing,
            u_angle,
            v_spacing,
            v_angle,
            u_phase: 0.0,
            v_phase: 0.0,
        }
    }

    fn symbols(shape: PatternSymbolShape, size: f32, lattice: SymbolLattice2d) -> SymbolLayer {
        SymbolLayer {
            lattice,
            symbol: PatternSymbol {
                shape,
                size,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        }
    }

    fn layers(layers: Vec<PatternLayer>) -> PatternFill {
        PatternFill {
            layers,
            ..Default::default()
        }
    }

    #[test]
    fn canonical_text_reads_as_its_pattern() {
        let dashed = StripeLayer {
            dash: Some(StripeDash {
                length: 4.0,
                gap: 3.0,
                phase: 0.0,
            }),
            ..StripeLayer::new(0.0, 6.0, 1.5)
        };
        let triangles = SymbolLayer {
            symbol: PatternSymbol {
                shape: PatternSymbolShape::TriangleUp,
                size: 30.0,
                rotation: 15.0,
            },
            paint: SymbolPaint::Open { stroke_width: 1.0 },
            ..symbols(
                PatternSymbolShape::TriangleUp,
                30.0,
                lattice(12.0, 0.0, 12.0, 60.0),
            )
        };
        let stars = SymbolLayer {
            lattice: SymbolLattice2d {
                u_phase: 5.0,
                ..lattice(10.0, 0.0, 12.0, 75.0)
            },
            ..symbols(PatternSymbolShape::Star, 20.0, lattice(0.0, 0.0, 0.0, 0.0))
        };
        let cases = [
            ("stripes 30 8 2", layers(vec![stripes(30.0, 8.0, 2.0)])),
            (
                "stripes 30 8 1.5 + stripes -60 8 1.5",
                layers(vec![stripes(30.0, 8.0, 1.5), stripes(-60.0, 8.0, 1.5)]),
            ),
            ("stripes 0 6 1.5 dash 4 3", layers(vec![dashed.into()])),
            (
                "circle 12 on grid 10",
                layers(vec![symbols(
                    PatternSymbolShape::Circle,
                    12.0,
                    lattice(10.0, 0.0, 10.0, 90.0),
                )
                .into()]),
            ),
            (
                "triangle-up 30 rotate 15 open 1 on hex 12",
                layers(vec![triangles.into()]),
            ),
            (
                "stripes 90 6 3 - circle 40 on grid 12; ink #1a3399 0.6; anchor mark",
                PatternFill {
                    anchor: PatternAnchor::Mark,
                    ink: PatternInk::Solid {
                        color: [26.0 / 255.0, 51.0 / 255.0, 153.0 / 255.0, 1.0],
                        opacity: 0.6,
                    },
                    layers: vec![
                        stripes(90.0, 6.0, 3.0),
                        PatternLayer::from(symbols(
                            PatternSymbolShape::Circle,
                            40.0,
                            lattice(12.0, 0.0, 12.0, 90.0),
                        ))
                        .with_operation(PatternLayerOperation::Subtract),
                    ],
                },
            ),
            (
                "star 20 on lattice 10 0 12 75 offset 5 0",
                layers(vec![stars.into()]),
            ),
            ("none", PatternFill::default()),
        ];
        for (text, pattern) in cases {
            assert_eq!(parse(text), pattern, "{text}");
            assert_eq!(pattern.to_string(), text);
        }
    }

    #[test]
    fn display_writes_the_shortest_equal_text() {
        for (text, canonical) in [
            (
                "stripes 45 8 2 offset 0 dash 4 3 0",
                "stripes 45 8 2 dash 4 3",
            ),
            (
                "stripes 45 8 2 offset 2 dash 4 3 1",
                "stripes 45 8 2 dash 4 3 1 offset 2",
            ),
            (
                "circle 12 open 1 rotate 0 on grid 10 10 at 0",
                "circle 12 open 1 on grid 10",
            ),
            ("square 9 on grid 6 8 at 30", "square 9 on grid 6 8 at 30"),
            ("square 9 on lattice 6 30 6 120", "square 9 on grid 6 at 30"),
            ("square 9 on lattice 6 0 6 60", "square 9 on hex 6"),
            (
                "square 9 on lattice 6 0 6 120",
                "square 9 on lattice 6 0 6 120",
            ),
            (
                "circle 12 on grid 10; anchor group; ink auto 0.18",
                "circle 12 on grid 10",
            ),
            ("+ stripes 0 4 1.5", "stripes 0 4 1.5"),
            (
                "^ stripes 0 4 1.5 ^ stripes 90 4 1.5",
                "^ stripes 0 4 1.5 ^ stripes 90 4 1.5",
            ),
            (
                "stripes 0 4 1.5; ink auto 0.3",
                "stripes 0 4 1.5; ink auto 0.3",
            ),
            ("stripes 0 4 1.5; ink navy", "stripes 0 4 1.5; ink #000080"),
            (
                "stripes 0 4 1.5;ink rgba(26, 51, 153, 0.5) 1;anchor scene",
                "stripes 0 4 1.5; ink rgba(26, 51, 153, 0.5) 1; anchor scene",
            ),
            ("none; anchor mark", "none; anchor mark"),
        ] {
            let pattern = parse(text);
            assert_eq!(pattern.to_string(), canonical, "{text}");
            assert_eq!(parse(canonical), pattern, "{canonical}");
        }
    }

    #[test]
    fn presets_read_and_write_by_name_and_scale() {
        for preset in PatternPreset::ALL {
            let name = preset.name();
            assert_eq!(parse(name).layers, preset.layers(1.0), "{name}");
            assert_eq!(parse(name).to_string(), name);
            for scale in ["0.5", "0.75", "1.3", "2", "2.5", "3.7"] {
                let text = format!("{name} {scale}");
                let pattern = parse(&text);
                assert_eq!(
                    pattern.layers,
                    preset.layers(scale.parse().unwrap()),
                    "{text}"
                );
                assert_eq!(pattern.to_string(), text);
            }
        }
    }

    #[test]
    fn presets_are_the_layers_that_their_documentation_gives() {
        for (preset, layers) in [
            (PatternPreset::DiagonalDown, "stripes 45 8 2"),
            (PatternPreset::DiagonalUp, "stripes -45 8 2"),
            (PatternPreset::Horizontal, "stripes 0 8 2"),
            (PatternPreset::Vertical, "stripes 90 8 2"),
            (PatternPreset::Crosshatch, "stripes 45 8 1.5 + stripes -45 8 1.5"),
            (PatternPreset::Gridlines, "stripes 0 8 1.5 + stripes 90 8 1.5"),
            (PatternPreset::Dashes, "stripes 0 6 2 dash 6 3"),
            (PatternPreset::Dots, "circle 16 on hex 8"),
            (PatternPreset::Rings, "circle 36 open 1 on hex 10"),
            (PatternPreset::Triangles, "triangle-up 30 on hex 10"),
            (PatternPreset::Checkerboard, "stripes 0 8 4 ^ stripes 90 8 4"),
            (
                PatternPreset::Bricks,
                "stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8",
            ),
        ] {
            assert_eq!(parse(layers).to_string(), preset.name());
        }
        // Layers of a preset at another scale write the preset with that scale.
        assert_eq!(parse("stripes 45 16 4").to_string(), "diagonal-down 2");
        assert_eq!(
            parse("circle 64 on hex 16; ink navy 0.6; anchor mark").to_string(),
            "dots 2; ink #000080 0.6; anchor mark"
        );
    }

    #[test]
    fn every_shape_reads_back_by_name() {
        for shape in PatternSymbolShape::ALL {
            let pattern = layers(vec![
                symbols(shape, 16.0, lattice(8.0, 15.0, 8.0, 105.0)).into()
            ]);
            assert_eq!(parse(&pattern.to_string()), pattern, "{}", shape.name());
        }
    }

    #[test]
    fn errors_point_at_their_column() {
        for (text, column, message) in [
            (
                "",
                1,
                "expected `stripes` or a symbol shape, such as `circle`, found the end",
            ),
            ("stripes 45 8", 13, "expected a stripe width, found the end"),
            (
                "stripes 45 eight 2",
                12,
                "expected a stripe spacing, found `eight`",
            ),
            (
                "stripes 45 NaN 2",
                12,
                "expected a stripe spacing, found `NaN`",
            ),
            (
                "heart 12 on grid 10",
                1,
                "expected `stripes` or a symbol shape, such as `circle`, found `heart`",
            ),
            (
                "circle 12 grid 10",
                11,
                "expected `rotate`, `open`, or `on` and a lattice, found `grid`",
            ),
            (
                "circle 12 on square 10",
                14,
                "expected `grid`, `hex`, or `lattice`, found `square`",
            ),
            (
                "circle 12 on grid 10 rotate 15",
                22,
                "expected `+`, `-`, `^`, `;`, or the end, found `rotate`",
            ),
            (
                "circle 12 on lattice 10 0 10 90 at 45",
                33,
                "expected `+`, `-`, `^`, `;`, or the end, found `at`",
            ),
            (
                "stripes 45 8 2 -stripes 0 8 2",
                16,
                "expected `+`, `-`, `^`, `;`, or the end, found `-stripes`",
            ),
            (
                "stripes 45 8 2 dash 4 3 dash 4 3",
                25,
                "`dash` appears twice",
            ),
            (
                "stripes 45 8 2; ink nocolor",
                21,
                "expected `auto` or a CSS color, found `nocolor`",
            ),
            (
                "stripes 45 8 2; ink auto; ink auto",
                27,
                "`ink` appears twice",
            ),
            (
                "stripes 45 8 2; fill red",
                17,
                "expected `ink` or `anchor`, found `fill`",
            ),
            ("stripes 45 8 2; ink rgba(1, 2", 21, "a `(` without its `)`"),
            ("stripes 45 8 2)", 15, "a `)` without its `(`"),
            (
                "dots + vertical",
                6,
                "expected a positive scale, `;`, or the end, found `+`",
            ),
            ("dots 2 + vertical", 8, "expected `;` or the end, found `+`"),
            (
                "dots 0",
                6,
                "expected a positive scale, `;`, or the end, found `0`",
            ),
            (
                "dots -2",
                6,
                "expected a positive scale, `;`, or the end, found `-2`",
            ),
            ("none + dots", 6, "expected `;` or the end, found `+`"),
        ] {
            assert_eq!(
                text.parse::<PatternFill>(),
                Err(PatternParseError::Syntax {
                    column,
                    message: message.to_string()
                }),
                "{text}"
            );
        }
    }

    #[test]
    fn text_that_follows_the_grammar_still_validates() {
        assert!(matches!(
            "stripes 45 0 2".parse::<PatternFill>(),
            Err(PatternParseError::Invalid(_))
        ));
        assert!(matches!(
            "square 400 on grid 4".parse::<PatternFill>(),
            Err(PatternParseError::Invalid(
                PatternValidationError::SymbolOverlap
            ))
        ));
    }
}
