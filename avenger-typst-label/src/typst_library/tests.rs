//! Tests for Avenger's parts of the shared kernel: the `elem!`, `cast!` and `derive_cast!`
//! stand-ins for upstream's procedural macros, the style chain semantics the pipeline depends
//! on, and colors.

use crate::label::label_span;
use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Args, Construct, Content, Fold, IntoValue, NativeElement, Packed, ShowSet, Smart,
    StyleChain, StyledElem, Styles, Value, dict, elem,
};
use crate::typst_library::layout::{Abs, Dir, Em, Length};
use crate::typst_library::math::EquationElem;
use crate::typst_library::text::{TextDir, TextElem};
use crate::typst_library::visualize::{Color, ColorExt, Paint, Stroke};
use typst_syntax::Span;

/// A type that accumulates depth when folded, like upstream's list depth.
#[derive(Debug, Default, Copy, Clone, PartialEq, Hash)]
pub struct Depth(pub usize);

impl Fold for Depth {
    fn fold(self, outer: Self) -> Self {
        Self(outer.0 + self.0)
    }
}

/// Opaque white.
fn white() -> Color {
    Color::from_srgb(1.0, 1.0, 1.0, 1.0)
}

elem! {
    /// An element that exercises each kind of field.
    #[elem(name = "probe", Construct, ShowSet)]
    pub struct ProbeElem {
        /// A required field.
        #[required]
        pub body: Content,

        /// A settable field that folds.
        #[fold]
        pub list: Vec<i64>,

        /// A settable field with a default and a two-word name.
        #[default(Abs::pt(1.0))]
        pub stroke_width: Abs,

        /// A synthesized field.
        #[synthesized]
        pub count: i64,

        /// A ghost property that folds.
        #[ghost]
        #[fold]
        pub depth: Depth,

        /// An internal synthesized field.
        #[internal]
        #[synthesized]
        pub hidden: i64,
    }
}

// A public ghost field rules out the generated constructor, as upstream.
impl Construct for ProbeElem {
    fn construct(_: &mut Engine, args: &mut Args) -> SourceResult<Content> {
        Ok(Self::new(args.expect("body")?).pack())
    }
}

impl ShowSet for Packed<ProbeElem> {
    fn show_set(&self, _: StyleChain) -> Styles {
        let mut out = Styles::new();
        out.set(ProbeElem::stroke_width, Abs::pt(5.0));
        out
    }
}

fn probe(text: &str) -> ProbeElem {
    ProbeElem::new(TextElem::packed(text))
}

fn span(start: usize) -> Span {
    label_span(start..start + 1)
}

mod elements {
    use super::*;

    #[test]
    fn equality_compares_stored_fields_that_are_not_internal() {
        assert_eq!(probe("x"), probe("x"));
        assert_ne!(probe("x"), probe("y"));
        assert_ne!(probe("x"), probe("x").with_list(vec![1]));
        assert_ne!(probe("x"), probe("x").with_count(1));
        assert_eq!(probe("x"), probe("x").with_hidden(1));
        assert_ne!(Content::new(probe("x")), TextElem::packed("x"));
        assert_eq!(Content::new(probe("x")).spanned(span(1)), Content::new(probe("x")));
    }
}

mod content {
    use super::*;

    #[test]
    fn downcasts_and_capabilities() {
        let content = Content::new(probe("x"));
        assert!(content.is::<ProbeElem>());
        assert!(!content.is::<TextElem>());
        assert!(content.to_packed::<ProbeElem>().is_some());
        assert!(content.to_packed::<TextElem>().is_none());
        assert!(!TextElem::packed("x").can::<dyn ShowSet>());
        assert!(Content::new(EquationElem::new(Content::empty())).can::<dyn ShowSet>());
        let root = Styles::new();
        let shown = content
            .with::<dyn ShowSet>()
            .unwrap()
            .show_set(StyleChain::new(&root));
        assert_eq!(StyleChain::new(&shown).get(ProbeElem::stroke_width), Abs::pt(5.0));
        assert_eq!(content.clone().unpack::<ProbeElem>().unwrap(), probe("x"));
        assert!(content.unpack::<TextElem>().is_err());
    }

    #[test]
    fn mutable_access_copies_shared_content() {
        let original = Content::new(probe("x"));
        let mut copy = original.clone();
        copy.to_packed_mut::<ProbeElem>().unwrap().count = Some(1);
        assert_eq!(original.to_packed::<ProbeElem>().unwrap().count, None);
        assert_eq!(copy.to_packed::<ProbeElem>().unwrap().count, Some(1));
    }

    #[test]
    fn spans_are_set_once() {
        let content = Content::new(probe("x")).spanned(span(1)).spanned(span(2));
        assert_eq!(content.span(), span(1));
        assert!(Content::new(probe("x")).span().is_detached());
    }
}

mod styles {
    use super::*;

    #[test]
    fn a_settable_field_folds_with_the_chain() {
        let mut root = Styles::new();
        root.set(ProbeElem::list, vec![1]);
        let styles = StyleChain::new(&root);
        let elem = probe("x").with_list(vec![2]).with_stroke_width(Abs::pt(4.0));
        assert_eq!(elem.list.get_cloned(styles), [1, 2]);
        assert_eq!(elem.stroke_width.get(styles), Abs::pt(4.0));
        assert_eq!(probe("x").list.get_cloned(styles), [1]);
    }

    #[test]
    fn chains_compare_by_identity() {
        // Realization relies on this: in `*a**b*`, each strong element styles its body with a
        // style list of its own, so the two runs stay separate segments although the lists
        // are equal.
        let mut first = Styles::new();
        first.set(ProbeElem::stroke_width, Abs::pt(2.0));
        let second = first.clone();
        let mut third = Styles::new();
        third.set(ProbeElem::stroke_width, Abs::pt(2.0));

        let root = Styles::new();
        let root = StyleChain::new(&root);
        assert_eq!(root.chain(&first), root.chain(&first));
        // A cloned `Styles` shares the allocation, as upstream's `EcoVec` does.
        assert_eq!(root.chain(&first), root.chain(&second));
        assert_ne!(root.chain(&first), root.chain(&third));
        assert_eq!(root.chain(&Styles::new()), root);
    }
}

mod casts {
    use super::*;

    #[test]
    fn strokes_reject_unknown_keys() {
        let error = Value::Dict(dict! { "paint" => Color::BLACK, "width" => 1 })
            .cast::<Stroke>()
            .unwrap_err();
        assert_eq!(
            error.message(),
            r#"unexpected key "width", valid keys are "paint", "thickness", "cap", "join", "dash", and "miter-limit""#
        );
    }

    #[test]
    fn strings_numbers_and_none_cast_to_content() {
        assert_eq!(
            Value::Str("a".into()).cast::<Content>().unwrap(),
            TextElem::packed("a")
        );
        assert!(Value::None.cast::<Content>().unwrap().is_empty());
        // Numbers display, as an Avenger leniency (D4), with upstream's minus sign.
        assert_eq!(
            Value::Int(-1).cast::<Content>().unwrap(),
            TextElem::packed("\u{2212}1")
        );
        assert_eq!(Value::Float(2.5).cast::<Content>().unwrap(), TextElem::packed("2.5"));
        let error = Value::Bool(true).cast::<Content>().unwrap_err();
        assert_eq!(error.message(), "expected content, found boolean");
        // A run of line breaks in a string becomes one space (D5).
        assert_eq!(
            Value::Str("a\r\n\nb\u{2028}c".into()).cast::<Content>().unwrap(),
            TextElem::packed("a b c")
        );
    }

    #[test]
    fn text_direction_must_be_horizontal() {
        assert_eq!(Value::Auto.cast::<TextDir>().unwrap(), TextDir(Smart::Auto));
        let error = Value::dynamic(Dir::TTB).cast::<TextDir>().unwrap_err();
        assert_eq!(error.message(), "text direction must be horizontal");
    }

    #[test]
    fn lengths_hint_at_missing_units() {
        let error = Value::Int(12).cast::<Length>().unwrap_err();
        assert_eq!(error.message(), "expected length, found integer");
        assert_eq!(error.hints(), ["a length needs a unit - did you mean 12pt?"]);
    }
}

mod engine {
    use super::*;
    use crate::label::fixtures;
    use crate::typst_library::diag::SourceDiagnostic;
    use crate::typst_library::engine::{Engine, Sink};
    use crate::typst_library::foundations::Symbol;

    #[test]
    fn sinks_drop_repeated_warnings() {
        let mut sink = Sink::new();
        sink.warn(SourceDiagnostic::warning(span(1), "a"));
        sink.warn(SourceDiagnostic::warning(span(1), "a"));
        sink.warn(SourceDiagnostic::warning(span(2), "a"));
        sink.warn(SourceDiagnostic::warning(span(1), "b"));
        let warnings: Vec<_> =
            sink.warnings().iter().map(|w| (w.span, w.message.clone())).collect();
        assert_eq!(
            warnings,
            [
                (span(1).into(), "a".into()),
                (span(2).into(), "a".into()),
                (span(1).into(), "b".into())
            ]
        );
    }

    #[test]
    fn symbol_deprecations_warn_through_the_engine() {
        let world = fixtures::world();
        let mut sink = Sink::new();
        let mut engine = Engine { world: &world, sink: &mut sink };
        let codex::Def::Symbol(gt) = codex::SYM.get("gt").unwrap().def else {
            unreachable!()
        };
        let tri = Symbol::from(gt).modified((&mut engine, span(3)), "tri").unwrap();
        assert_eq!(tri.get(), "⊳");
        let [warning] = <[_; 1]>::try_from(sink.warnings().to_vec()).unwrap();
        assert_eq!(warning.message, "`gt.tri` is deprecated, use `gt.closed` instead");
        assert_eq!(warning.span, span(3).into());
    }
}

mod frames {
    use super::*;
    use crate::typst_library::layout::{Frame, FrameItem, Point, Size};
    use crate::typst_library::visualize::Geometry;
    use typst_syntax::Span;

    fn rect(frame: &mut Frame) {
        let shape = Geometry::Rect(Size::splat(Abs::pt(1.0))).filled(Color::BLACK);
        frame.push(Point::zero(), FrameItem::Shape(shape, Span::detached()));
    }

    fn positions(frame: &Frame) -> Vec<(f64, f64, &'static str)> {
        frame
            .items()
            .map(|(pos, item)| {
                let kind = match item {
                    FrameItem::Group(_) => "group",
                    FrameItem::Text(_) => "text",
                    FrameItem::Shape(..) => "shape",
                };
                (pos.x.to_pt(), pos.y.to_pt(), kind)
            })
            .collect()
    }

    #[test]
    fn small_soft_frames_are_inlined_and_hard_ones_grouped() {
        let mut outer = Frame::soft(Size::splat(Abs::pt(10.0)));
        rect(&mut outer);
        let mut soft = Frame::soft(Size::splat(Abs::pt(2.0)));
        rect(&mut soft);
        outer.push_frame(Point::new(Abs::pt(1.0), Abs::pt(2.0)), soft);
        let mut hard = Frame::hard(Size::splat(Abs::pt(2.0)));
        rect(&mut hard);
        outer.push_frame(Point::new(Abs::pt(3.0), Abs::pt(4.0)), hard);
        assert_eq!(
            positions(&outer),
            [(0.0, 0.0, "shape"), (1.0, 2.0, "shape"), (3.0, 4.0, "group")]
        );
    }

    #[test]
    fn translation_moves_items_and_the_baseline() {
        let mut frame = Frame::soft(Size::splat(Abs::pt(10.0)));
        rect(&mut frame);
        frame.set_baseline(Abs::pt(8.0));
        frame.translate(Point::new(Abs::pt(1.0), Abs::pt(2.0)));
        assert_eq!(positions(&frame), [(1.0, 2.0, "shape")]);
        assert_eq!(frame.baseline(), Abs::pt(10.0));
        assert_eq!(frame.ascent(), Abs::pt(10.0));
        assert_eq!(frame.descent(), Abs::zero());
    }
}

mod ops {
    use super::*;
    use crate::typst_library::foundations::ops;
    use crate::typst_library::layout::{Ratio, Rel};

    #[test]
    fn arithmetic_follows_upstream() {
        let pt = |v: f64| Abs::pt(v).into_value();
        assert_eq!(ops::add(pt(1.0), pt(2.0)).unwrap(), pt(3.0));
        assert_eq!(
            ops::add(pt(15.0), Em::new(0.5).into_value()).unwrap(),
            (Length::from(Abs::pt(15.0)) + Length::from(Em::new(0.5))).into_value()
        );
        assert_eq!(
            ops::add(Ratio::new(0.5).into_value(), pt(1.0)).unwrap(),
            Rel::<Length>::new(Ratio::new(0.5), Abs::pt(1.0).into()).into_value()
        );
        assert_eq!(ops::div(Value::Int(1), Value::Int(4)).unwrap(), Value::Float(0.25));
        assert_eq!(ops::neg(Value::Int(3)).unwrap(), Value::Int(-3));
        assert_eq!(
            ops::mul(Value::Str("ab".into()), Value::Int(2)).unwrap(),
            Value::Str("abab".into())
        );
        let error = ops::div(Value::Int(1), Value::Int(0)).unwrap_err();
        assert_eq!(error.message(), "cannot divide by zero");
        let error = ops::add(Value::Bool(true), Value::Int(1)).unwrap_err();
        assert_eq!(error.message(), "cannot add boolean and integer");
    }
}

mod functions {
    use super::*;
    use crate::label::fixtures;
    use crate::typst_library::Library;
    use crate::typst_library::engine::Sink;
    use crate::typst_library::foundations::{Arg, Func, Str};
    use typst_syntax::Spanned;

    /// Arguments from `(name, value)` pairs, each spanned by its position.
    fn args(items: Vec<(Option<&str>, Value)>) -> Args {
        items
            .into_iter()
            .enumerate()
            .map(|(i, (name, value))| Arg {
                span: span(i),
                name: name.map(Str::from),
                value: Spanned::new(value, span(i)),
            })
            .collect::<Args>()
            .spanned(span(99))
    }

    /// Calls the global function `name`.
    fn call(name: &str, items: Vec<(Option<&str>, Value)>) -> SourceResult<Value> {
        let func = Library::get().global.scope().get(name).unwrap().read().clone();
        let func = func.cast::<Func>().unwrap();
        let mut sink = Sink::new();
        let mut engine = Engine { world: fixtures::shared(), sink: &mut sink };
        func.call(&mut engine, args(items))
    }

    fn content(value: Value) -> Content {
        value.cast::<Content>().unwrap()
    }

    #[test]
    fn text_styles_its_body() {
        let body = TextElem::packed("x").into_value();
        let styled = content(
            call(
                "text",
                vec![
                    (None, Abs::pt(20.0).into_value()),
                    (None, white().into_value()),
                    (None, body.clone()),
                ],
            )
            .unwrap(),
        );
        let styled = styled.to_packed::<StyledElem>().unwrap();
        let chain = StyleChain::new(&styled.styles);
        assert_eq!(chain.resolve(TextElem::size), Abs::pt(20.0));
        assert_eq!(chain.get_ref(TextElem::fill), &Paint::Solid(white()));
        // Text properties outside the label subset are unexpected (D14).
        let error =
            call("text", vec![(Some("stroke"), Abs::pt(1.0).into_value()), (None, body)])
                .unwrap_err();
        assert_eq!(error[0].message, "unexpected argument: stroke");
    }

    #[test]
    fn case_functions_change_strings_and_style_content() {
        assert_eq!(
            call("upper", vec![(None, "ab".into_value())]).unwrap(),
            "AB".into_value()
        );
        let lowered = content(
            call("lower", vec![(None, TextElem::packed("AB").into_value())]).unwrap(),
        );
        assert!(lowered.is::<StyledElem>());
    }

    #[test]
    fn colors_follow_css_and_typst_arguments() {
        let color =
            |items| call("rgb", items).unwrap().cast::<Color>().unwrap().to_rgba8();
        assert_eq!(color(vec![(None, "tomato".into_value())]), [255, 99, 71, 255]);
        assert_eq!(color(vec![(None, "#ff413680".into_value())]), [255, 65, 54, 128]);
        assert_eq!(
            color(vec![
                (None, Value::Int(255)),
                (None, Value::Int(0)),
                (None, Value::Int(51))
            ]),
            [255, 0, 51, 255]
        );
        let error = call(
            "rgb",
            vec![(None, Value::Int(256)), (None, Value::Int(0)), (None, Value::Int(0))],
        )
        .unwrap_err();
        assert_eq!(error[0].message, "number must be between 0 and 255");
        let error = call("rgb", vec![(None, "nope".into_value())]).unwrap_err();
        assert_eq!(error[0].message, "invalid color string 'nope'");
        let gray = call("luma", vec![(None, Value::Int(51))])
            .unwrap()
            .cast::<Color>()
            .unwrap();
        assert_eq!(gray.to_rgba8(), [51, 51, 51, 255]);
    }

    #[test]
    fn fields_reach_into_modules_symbols_and_lengths() {
        let sym = Library::get().global.scope().get("sym").unwrap().read().clone();
        let arrow = sym.field("arrow", ()).unwrap();
        let right = arrow.field("r", ()).unwrap();
        assert_eq!(right.cast::<Str>().unwrap().as_str(), "→");
        let error = sym.field("nope", ()).unwrap_err();
        assert_eq!(error, "module `sym` does not contain `nope`");
        let length =
            (Length::from(Abs::pt(2.0)) + Length::from(Em::new(1.0))).into_value();
        assert_eq!(length.field("em", ()).unwrap(), Value::Float(1.0));
        let error = Value::Int(1).field("x", ()).unwrap_err();
        assert_eq!(error, "cannot access fields on type integer");
    }
}

mod fonts {
    use crate::typst_library::layout::Abs;
    use crate::typst_library::text::{
        AxisValue, FontAxis, FontStretch, FontStyle, FontVariant, FontVariations,
        FontWeight, Tag,
    };

    fn axis(tag: &[u8; 4], min: f32, max: f32) -> FontAxis {
        FontAxis {
            tag: Tag::from_bytes(tag),
            min: AxisValue(min),
            max: AxisValue(max),
            default: AxisValue(min.max(0.0).min(max)),
        }
    }

    /// The variations for a style and weight at 100pt.
    fn resolve(axes: &[FontAxis], style: FontStyle, weight: u16) -> Vec<([u8; 4], f32)> {
        let variant =
            FontVariant::new(style, FontWeight::from_number(weight), FontStretch::NORMAL);
        let variations = FontVariations::resolve(axes, variant, Abs::pt(100.0));
        variations
            .0
            .iter()
            .map(|(tag, value)| (tag.to_bytes(), value.0))
            .collect()
    }

    #[test]
    fn variations_serve_styles_weights_and_optical_sizes() {
        let ital = axis(b"ital", 0.0, 1.0);
        let slnt = axis(b"slnt", -12.0, 0.0);
        let wght = axis(b"wght", 100.0, 900.0);
        let opsz = axis(b"opsz", 8.0, 72.0);
        let all = [ital.clone(), slnt.clone(), wght.clone(), opsz.clone()];
        // Italic takes the italic axis, oblique the slant axis at its most negative slant,
        // and the optical size follows the font size within the axis's range.
        assert_eq!(
            resolve(&all, FontStyle::Italic, 700),
            [(*b"ital", 1.0), (*b"wght", 700.0), (*b"opsz", 72.0)]
        );
        assert_eq!(
            resolve(&all, FontStyle::Oblique, 400),
            [(*b"slnt", -12.0), (*b"wght", 400.0), (*b"opsz", 72.0)]
        );
        assert_eq!(
            resolve(&all, FontStyle::Normal, 300),
            [(*b"wght", 300.0), (*b"opsz", 72.0)]
        );
        // Each style falls back to the other's axis.
        assert_eq!(resolve(&[slnt], FontStyle::Italic, 400), [(*b"slnt", -12.0)]);
        assert_eq!(resolve(&[ital], FontStyle::Oblique, 400), [(*b"ital", 1.0)]);
        // A slant axis without negative slants slants positively.
        assert_eq!(
            resolve(&[axis(b"slnt", 0.0, 10.0)], FontStyle::Oblique, 400),
            [(*b"slnt", 10.0)]
        );
    }
}
