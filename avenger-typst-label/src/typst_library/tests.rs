//! Tests for Avenger's parts of the shared kernel: the `elem!`, `cast!` and `derive_cast!`
//! stand-ins for upstream's procedural macros, the style chain semantics the pipeline depends
//! on, and colors.

use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::{
    Args, Construct, Content, Datetime, Fold, IntoValue, NativeElement, Packed, Repr,
    Resolve, SequenceElem, ShowSet, Smart, StyleChain, StyledElem, Styles, Value, dict,
    elem,
};
use crate::typst_library::layout::{Abs, Dir, Em, Length};
use crate::typst_library::math::{EquationElem, MathSize};
use crate::typst_library::text::{Lang, TextDir, TextElem, TextSize};
use crate::typst_library::visualize::{
    Color, ColorExt, FillRule, FixedStroke, LineCap, Paint, Stroke,
};
use crate::typst_syntax::{FileId, Span};

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
    Span::from_range(FileId::LABEL, start..start + 1)
}

mod elements {
    use super::*;

    #[test]
    fn element_identity_and_names() {
        assert_eq!(ProbeElem::ELEM.name(), "probe");
        assert_eq!(ProbeElem::ELEM, Content::new(probe("x")).elem());
        assert_ne!(ProbeElem::ELEM, TextElem::ELEM);
        assert_eq!(TextElem::ELEM.name(), "text");
        assert_eq!(EquationElem::ELEM.name(), "equation");
    }

    #[test]
    fn fields_are_numbered_in_declaration_order_with_kebab_names() {
        assert_eq!(ProbeElem::body.index(), 0);
        assert_eq!(ProbeElem::stroke_width.index(), 2);
        assert_eq!(ProbeElem::hidden.index(), 5);
        let names: Vec<_> =
            (0..6).map(|id| ProbeElem::ELEM.field_name(id).unwrap()).collect();
        assert_eq!(names, ["body", "list", "stroke-width", "count", "depth", "hidden"]);
        assert_eq!(ProbeElem::ELEM.field_name(6), None);
        assert_eq!(
            EquationElem::ELEM.field_name(EquationElem::script_scale.index()),
            Some("script-scale")
        );
    }

    #[test]
    fn builders_set_stored_fields() {
        let elem = probe("x").with_list(vec![1]).with_count(3);
        assert_eq!(elem.list.as_option(), &Some(vec![1]));
        assert_eq!(elem.count, Some(3));
        assert!(!elem.stroke_width.is_set());
        assert_eq!(elem.hidden, None);
    }

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

    #[test]
    fn generic_repr_lists_present_fields_that_are_not_internal() {
        assert_eq!(Content::new(probe("x")).repr(), "probe(body: [x])");
        let elem = probe("x").with_list(vec![1, 2]).with_count(3).with_hidden(4);
        assert_eq!(
            Content::new(elem.clone()).repr(),
            "probe(body: [x], list: (1, 2), count: 3)"
        );
        // Like upstream's, a long field list breaks over lines.
        assert_eq!(
            Content::new(elem.with_stroke_width(Abs::pt(2.0))).repr(),
            "probe(\n  body: [x],\n  list: (1, 2),\n  stroke-width: 2pt,\n  count: 3,\n)"
        );
    }

    #[test]
    fn elements_with_their_own_repr_use_it() {
        assert_eq!(TextElem::packed("a").repr(), "[a]");
        assert_eq!(Content::empty().repr(), "[]");
        let sequence = TextElem::packed("a") + TextElem::packed("b");
        assert_eq!(sequence.repr(), "sequence([a], [b])");
        let styled = TextElem::packed("a").set(ProbeElem::list, vec![1]);
        assert_eq!(styled.repr(), "styled(child: [a], ..)");
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

    #[test]
    fn addition_flattens_sequences() {
        let a = TextElem::packed("a");
        let b = TextElem::packed("b");
        let c = TextElem::packed("c");
        let left = (a.clone() + b.clone()) + c.clone();
        let right = a.clone() + (b.clone() + c.clone());
        let children = |content: &Content| {
            content.to_packed::<SequenceElem>().unwrap().children.clone()
        };
        assert_eq!(children(&left), [a.clone(), b.clone(), c.clone()]);
        assert_eq!(children(&right), [a.clone(), b, c]);
        assert_eq!(Content::sequence([a.clone()]), a);
        assert!(Content::sequence([]).is_empty());
    }

    #[test]
    fn styling_merges_into_one_styled_element() {
        let styled = TextElem::packed("a")
            .set(ProbeElem::list, vec![2])
            .set(ProbeElem::list, vec![1]);
        let elem = styled.to_packed::<StyledElem>().unwrap();
        assert!(elem.child.is::<TextElem>());
        assert_eq!(elem.styles.as_slice().len(), 2);
        // The later (outer) style comes first, so it folds as the outer value.
        let styles = StyleChain::new(&elem.styles);
        assert_eq!(styles.get_cloned(ProbeElem::list), [1, 2]);
    }
}

mod styles {
    use super::*;

    #[test]
    fn unset_properties_take_their_defaults() {
        let root = Styles::new();
        let styles = StyleChain::new(&root);
        assert_eq!(styles.get(ProbeElem::stroke_width), Abs::pt(1.0));
        assert_eq!(styles.get_cloned(ProbeElem::list), Vec::<i64>::new());
        assert_eq!(styles.get(TextElem::lang), Lang::ENGLISH);
        assert_eq!(styles.resolve(TextElem::size), Abs::pt(11.0));
        assert!(!styles.has(ProbeElem::stroke_width));
    }

    #[test]
    fn inner_values_win_unless_the_property_folds() {
        let mut outer = Styles::new();
        outer.set(ProbeElem::stroke_width, Abs::pt(2.0));
        outer.set(ProbeElem::list, vec![1]);
        let mut inner = Styles::new();
        inner.set(ProbeElem::stroke_width, Abs::pt(3.0));
        inner.set(ProbeElem::list, vec![2]);

        let outer = StyleChain::new(&outer);
        let styles = outer.chain(&inner);
        assert_eq!(styles.get(ProbeElem::stroke_width), Abs::pt(3.0));
        // `Vec` folds with the outer values first.
        assert_eq!(styles.get_cloned(ProbeElem::list), [1, 2]);
        assert!(styles.has(ProbeElem::list));
    }

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
    fn ghost_properties_fold() {
        let mut outer = Styles::new();
        outer.set(ProbeElem::depth, Depth(1));
        let mut inner = Styles::new();
        inner.set(ProbeElem::depth, Depth(2));
        let outer = StyleChain::new(&outer);
        assert_eq!(outer.chain(&inner).get(ProbeElem::depth), Depth(3));
    }

    #[test]
    fn text_sizes_multiply() {
        let mut outer = Styles::new();
        outer.set(TextElem::size, TextSize(Abs::pt(10.0).into()));
        let mut inner = Styles::new();
        inner.set(TextElem::size, TextSize(Em::new(1.5).into()));
        let mut innermost = Styles::new();
        innermost.set(
            TextElem::size,
            TextSize(Length { abs: Abs::pt(1.0), em: Em::new(2.0) }),
        );

        let outer = StyleChain::new(&outer);
        let inner = outer.chain(&inner);
        assert_eq!(inner.resolve(TextElem::size), Abs::pt(15.0));
        assert_eq!(inner.chain(&innermost).resolve(TextElem::size), Abs::pt(31.0));
        // Ems resolve against the text size.
        assert_eq!(Em::one().resolve(inner), Abs::pt(15.0));
    }

    #[test]
    fn script_sizes_scale_text() {
        let mut root = Styles::new();
        root.set(TextElem::size, TextSize(Abs::pt(10.0).into()));
        let mut script = Styles::new();
        script.set(EquationElem::size, MathSize::Script);
        let mut script_script = Styles::new();
        script_script.set(EquationElem::size, MathSize::ScriptScript);
        script_script.set(EquationElem::script_scale, (80, 60));

        let root = StyleChain::new(&root);
        assert_eq!(root.chain(&script).resolve(TextElem::size), Abs::pt(7.0));
        assert_eq!(root.chain(&script_script).resolve(TextElem::size), Abs::pt(6.0));
    }

    #[test]
    fn text_direction_follows_the_language() {
        let root = Styles::new();
        let root = StyleChain::new(&root);
        assert_eq!(root.resolve(TextElem::dir), Dir::LTR);

        let mut arabic = Styles::new();
        arabic.set(TextElem::lang, Lang::ARABIC);
        assert_eq!(root.chain(&arabic).resolve(TextElem::dir), Dir::RTL);

        let mut explicit = Styles::new();
        explicit.set(TextElem::dir, TextDir(Smart::Custom(Dir::LTR)));
        let arabic = root.chain(&arabic);
        assert_eq!(arabic.chain(&explicit).resolve(TextElem::dir), Dir::LTR);
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

    #[test]
    fn trunk_finds_the_shared_prefix() {
        let mut shared = Styles::new();
        shared.set(ProbeElem::stroke_width, Abs::pt(2.0));
        let mut a = Styles::new();
        a.set(ProbeElem::list, vec![1]);
        let mut b = Styles::new();
        b.set(ProbeElem::list, vec![1]);

        let root = Styles::new();
        let root = StyleChain::new(&root);
        let shared = root.chain(&shared);
        let trunk = StyleChain::trunk([shared.chain(&a), shared.chain(&b)]).unwrap();
        assert_eq!(trunk, shared);
        assert_eq!(trunk.to_map().as_slice().len(), 1);
    }
}

mod casts {
    use super::*;

    #[test]
    fn derived_casts_use_kebab_case_variant_names() {
        assert_eq!(FillRule::NonZero.into_value(), "non-zero".into_value());
        assert_eq!(
            Value::Str("even-odd".into()).cast::<FillRule>().unwrap(),
            FillRule::EvenOdd
        );
        assert_eq!(Value::Str("round".into()).cast::<LineCap>().unwrap(), LineCap::Round);
        let error = Value::Str("bevel".into()).cast::<LineCap>().unwrap_err();
        assert_eq!(error.message(), r#"expected "butt", "round", or "square""#);
        let error = Value::Int(1).cast::<LineCap>().unwrap_err();
        assert_eq!(
            error.message(),
            r#"expected "butt", "round", or "square", found integer"#
        );
    }

    #[test]
    fn strokes_cast_from_lengths_colors_and_dictionaries() {
        let length = Value::Length(Abs::pt(2.0).into()).cast::<Stroke>().unwrap();
        assert_eq!(length.thickness, Smart::Custom(Abs::pt(2.0).into()));
        assert_eq!(length.paint, Smart::Auto);

        let color = Value::Color(white()).cast::<Stroke>().unwrap();
        assert_eq!(color.paint, Smart::Custom(Paint::Solid(white())));

        let dict = dict! { "thickness" => Abs::pt(3.0), "cap" => "round" };
        let stroke = Value::Dict(dict).cast::<Stroke>().unwrap();
        assert_eq!(stroke.cap, Smart::Custom(LineCap::Round));
        let fixed = stroke.clone().map(|length| length.abs).unwrap_or_default();
        assert_eq!(
            fixed,
            FixedStroke {
                thickness: Abs::pt(3.0),
                cap: LineCap::Round,
                ..FixedStroke::default()
            }
        );

        let error = Value::Dict(dict! { "paint" => Color::BLACK, "width" => 1 })
            .cast::<Stroke>()
            .unwrap_err();
        assert_eq!(
            error.message(),
            r#"unexpected key "width", valid keys are "paint", "thickness", "cap", "join", "dash", and "miter-limit""#
        );
        assert_eq!(Value::dynamic(stroke.clone()).cast::<Stroke>().unwrap(), stroke);
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
    fn values_display_as_content_or_fail_with_a_hint() {
        assert_eq!(Value::Int(-7).display().unwrap(), TextElem::packed("\u{2212}7"));
        assert_eq!(Value::Str("a\nb".into()).display().unwrap(), TextElem::packed("a b"));
        assert!(Value::None.display().unwrap().is_empty());
        let error = Value::Bool(true).display().unwrap_err();
        assert_eq!(error.message(), "cannot display boolean in a label");
        assert_eq!(error.hints(), ["use a string instead"]);
        let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        let error = Value::Datetime(Datetime::Date(date)).display().unwrap_err();
        assert_eq!(error.hints(), ["format it with `#datetimefmt`"]);
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

mod colors {
    use super::*;

    #[test]
    fn colors_display_as_rgb_hex_strings() {
        assert_eq!(Color::BLACK.repr(), r##"rgb("#000000")"##);
        assert_eq!(
            Color::from_u8(0xff, 0x41, 0x36, 0x80).repr(),
            r##"rgb("#ff413680")"##
        );
        assert_eq!(Paint::Solid(white()).repr(), r##"rgb("#ffffff")"##);
        assert_eq!(Value::Color(Color::BLACK).ty().long_name(), "color");
    }

    #[test]
    fn strokes_display_like_upstream() {
        let stroke = Stroke::<Length> {
            paint: Smart::Custom(Color::BLACK.into()),
            thickness: Smart::Custom(Abs::pt(2.0).into()),
            ..Default::default()
        };
        assert_eq!(stroke.repr(), r##"2pt + rgb("#000000")"##);
        assert_eq!(Stroke::<Length>::default().repr(), "1pt + black");
    }
}

mod symbols {
    //! Expected values are from `typst eval` at v0.15.1.

    use super::*;
    use crate::typst_library::diag::{HintedString, StrResult, WarningSink};
    use crate::typst_library::foundations::{Str, Symbol, SymbolElem};

    #[derive(Default)]
    struct Warnings(Vec<HintedString>);

    impl WarningSink for &mut Warnings {
        fn emit(&mut self, message: HintedString) {
            self.0.push(message);
        }
    }

    /// Looks a dotted name up in codex's `sym` module and applies its modifiers one at a time,
    /// as `sym.arrow.r` does.
    fn sym(path: &str, warnings: &mut Warnings) -> StrResult<Symbol> {
        let mut parts = path.split('.');
        let binding = codex::SYM.get(parts.next().unwrap()).unwrap();
        let codex::Def::Symbol(symbol) = binding.def else {
            panic!("{path} is a module")
        };
        parts.try_fold(Symbol::from(symbol), |symbol, modifier| {
            symbol.modified(&mut *warnings, modifier)
        })
    }

    fn get(path: &str) -> String {
        sym(path, &mut Warnings::default()).unwrap().get().to_string()
    }

    #[test]
    fn modifiers_select_variants_in_any_order() {
        assert_eq!(get("alpha"), "α");
        assert_eq!(get("arrow"), "→");
        assert_eq!(get("arrow.r.long"), "⟶");
        assert_eq!(get("arrow.long.r"), "⟶");
        assert_eq!(get("arrow.l"), "←");
        assert_eq!(get("arrow.l.r"), "↔\u{fe0e}");
    }

    #[test]
    fn unknown_modifiers_are_errors() {
        let mut warnings = Warnings::default();
        assert_eq!(
            sym("arrow.nope", &mut warnings).unwrap_err(),
            "unknown symbol modifier"
        );
        assert_eq!(sym("alpha.r", &mut warnings).unwrap_err(), "unknown symbol modifier");
        assert!(warnings.0.is_empty());
    }

    #[test]
    fn deprecated_variants_warn_once() {
        let mut warnings = Warnings::default();
        assert_eq!(sym("gt.tri.eq", &mut warnings).unwrap().get(), "⊵");
        let messages: Vec<_> = warnings.0.iter().map(|w| w.message().as_str()).collect();
        assert_eq!(messages, ["`gt.tri` is deprecated, use `gt.closed` instead"]);
    }

    #[test]
    fn reprs_list_the_variants_that_remain() {
        let repr = |path| sym(path, &mut Warnings::default()).unwrap().repr();
        assert_eq!(repr("alpha"), r#"symbol("α")"#);
        assert_eq!(repr("arrows.lr"), r#"symbol("⇆", ("stop", "↹"))"#);
        assert_eq!(
            repr("arrow.r.long"),
            "symbol(\n  (\"bar\", \"⟼\"),\n  (\"double\", \"⟹\"),\n  (\"double.bar\", \"⟾\"),\n  \
             \"⟶\",\n  (\"squiggly\", \"⟿\"),\n  (\"l.double\", \"⟺\"),\n  (\"l\", \"⟷\"),\n)"
        );
        assert_eq!(SymbolElem::packed("→").repr(), "[→]");
    }

    #[test]
    fn symbol_values_cast_to_strings_and_content() {
        let arrow = Value::Symbol(sym("arrow", &mut Warnings::default()).unwrap());
        assert_eq!(arrow.ty().long_name(), "symbol");
        assert_eq!(arrow.clone().cast::<Str>().unwrap().as_str(), "→");
        assert_eq!(arrow.cast::<Content>().unwrap(), SymbolElem::packed("→"));
    }
}

mod args {
    use ecow::EcoString;

    use super::*;
    use crate::typst_library::diag::SourceDiagnostic;
    use crate::typst_library::foundations::{Arg, Args, Str};
    use crate::typst_syntax::{DiagSpan, Spanned};

    /// Arguments `0: 1pt, 1: "a", 2: fill: black, 3: 2pt, 4: fill: white`, where `n:` is the
    /// argument's span.
    fn args() -> Args {
        let arg = |start, name: Option<&str>, value: Value| Arg {
            span: span(start),
            name: name.map(Str::from),
            value: Spanned::new(value, span(start)),
        };
        [
            arg(0, None, Abs::pt(1.0).into_value()),
            arg(1, None, "a".into_value()),
            arg(2, Some("fill"), Color::BLACK.into_value()),
            arg(3, None, Abs::pt(2.0).into_value()),
            arg(4, Some("fill"), white().into_value()),
        ]
        .into_iter()
        .collect::<Args>()
        .spanned(span(9))
    }

    /// The message, span and hints of the only error.
    fn single_error(
        errors: Vec<SourceDiagnostic>,
    ) -> (EcoString, DiagSpan, Vec<EcoString>) {
        let [error] = <[_; 1]>::try_from(errors).unwrap();
        (
            error.message,
            error.span,
            error.hints.iter().map(|hint| hint.v.clone()).collect(),
        )
    }

    #[test]
    fn positional_arguments_are_taken_in_order() {
        let mut args = args();
        assert_eq!(args.remaining(), 3);
        assert_eq!(args.eat::<Length>().unwrap(), Some(Abs::pt(1.0).into()));
        // `find` skips arguments of other types; `eat` would fail on the string.
        assert_eq!(args.find::<Length>().unwrap(), Some(Abs::pt(2.0).into()));
        assert_eq!(args.expect::<Str>("body").unwrap().as_str(), "a");
        assert_eq!(args.eat::<Length>().unwrap(), None);
    }

    #[test]
    fn named_arguments_take_the_last_value_and_remove_all() {
        let mut args = args();
        assert_eq!(args.named::<Color>("fill").unwrap(), Some(white()));
        assert_eq!(args.named::<Color>("fill").unwrap(), None);
        assert_eq!(args.named_or_find::<Str>("body").unwrap(), Some("a".into()));
    }

    #[test]
    fn cast_errors_point_at_the_argument() {
        let mut args = args();
        args.eat::<Length>().unwrap();
        let (message, at, _) = single_error(args.eat::<Length>().unwrap_err().to_vec());
        assert_eq!(message, "expected length, found string");
        assert_eq!(at, span(1).into());
    }

    #[test]
    fn missing_and_unexpected_arguments_are_errors() {
        let mut args = args();
        args.all::<Value>().unwrap();
        let (message, at, _) =
            single_error(args.expect::<Value>("body").unwrap_err().to_vec());
        assert_eq!(message, "missing argument: body");
        assert_eq!(at, span(9).into());

        let (message, at, _) = single_error(args.clone().finish().unwrap_err().to_vec());
        assert_eq!(message, "unexpected argument: fill");
        assert_eq!(at, span(2).into());

        let (message, _, hints) =
            single_error(args.expect::<Value>("fill").unwrap_err().to_vec());
        assert_eq!(message, "the argument `fill` is positional");
        assert_eq!(hints, ["try removing `fill:`"]);

        let mut positional = Args::new(span(0), [1i64]);
        positional.consume(1).unwrap();
        let (message, _, _) =
            single_error(Args::new(span(0), [1i64]).consume(2).unwrap_err().to_vec());
        assert_eq!(message, "not enough arguments");
        let (message, _, _) =
            single_error(Args::new(span(0), [1i64]).finish().unwrap_err().to_vec());
        assert_eq!(message, "unexpected argument");
    }

    #[test]
    fn arguments_display_like_upstream() {
        assert_eq!(
            args().repr(),
            "arguments(\n  1pt,\n  \"a\",\n  fill: rgb(\"#000000\"),\n  2pt,\n  fill: rgb(\"#ffffff\"),\n)"
        );
        assert_eq!(Args::new(span(0), [1i64, 2]).repr(), "arguments(1, 2)");
        assert_eq!(args(), args());
        assert_ne!(args(), Args::new(span(0), [1i64]));
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
    use crate::typst_syntax::Span;

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

    #[test]
    fn a_length_and_a_color_make_a_stroke() {
        let stroke =
            ops::add(Abs::pt(2.0).into_value(), Color::BLACK.into_value()).unwrap();
        let stroke = stroke.cast::<Stroke>().unwrap();
        assert_eq!(stroke.thickness, Smart::Custom(Abs::pt(2.0).into()));
        assert_eq!(stroke.paint, Smart::Custom(Paint::Solid(Color::BLACK)));
    }
}

mod functions {
    use super::*;
    use crate::label::fixtures;
    use crate::typst_library::Library;
    use crate::typst_library::engine::Sink;
    use crate::typst_library::foundations::{Arg, Func, Str};
    use crate::typst_library::text::UnderlineElem;
    use crate::typst_syntax::Spanned;

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
    fn element_functions_parse_fields_like_upstream() {
        let body = TextElem::packed("x").into_value();
        let underline = content(
            call(
                "underline",
                vec![
                    (Some("offset"), Abs::pt(2.0).into_value()),
                    (None, body.clone()),
                    (Some("evade"), false.into_value()),
                ],
            )
            .unwrap(),
        );
        let underline = underline.to_packed::<UnderlineElem>().unwrap();
        assert_eq!(
            underline.offset.as_option(),
            &Some(Smart::Custom(Abs::pt(2.0).into()))
        );
        assert_eq!(underline.evade.as_option(), &Some(false));
        assert_eq!(underline.body, TextElem::packed("x"));

        // A missing body, a wrong type and a leftover argument are upstream's errors.
        let error = call("strong", vec![]).unwrap_err();
        assert_eq!(error[0].message, "missing argument: body");
        let error =
            call("strong", vec![(Some("delta"), "a".into_value()), (None, body.clone())])
                .unwrap_err();
        assert_eq!(error[0].message, "expected integer, found string");
        let error = call("strong", vec![(Some("color"), Value::Int(1)), (None, body)])
            .unwrap_err();
        assert_eq!(error[0].message, "unexpected argument: color");
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
    fn the_library_has_the_label_definitions() {
        let global = Library::get().global.scope();
        for name in
            ["strong", "underline", "highlight", "text", "rgb", "sym", "emoji", "math"]
        {
            assert!(global.get(name).is_some(), "{name}");
        }
        // Named colors are CSS's (D22).
        let red = global.get("red").unwrap().read().clone().cast::<Color>().unwrap();
        assert_eq!(red.to_rgba8(), [255, 0, 0, 255]);
        assert!(global.get("tomato").is_some());
        assert!(global.get("linebreak").is_none());
        // Math has the symbols.
        let math = Library::get().math.scope();
        assert!(math.get("alpha").is_some());
        assert!(math.get("arrow").is_some());
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

mod math {
    use super::*;
    use crate::label::fixtures;
    use crate::typst_library::Library;
    use crate::typst_library::engine::Sink;
    use crate::typst_library::foundations::{Arg, Func, Str, Symbol, SymbolElem};
    use crate::typst_library::layout::HElem;
    use crate::typst_library::math::accent::Accent;
    use crate::typst_library::math::{
        AccentElem, AttachElem, FracElem, LrElem, OpElem, RootElem,
    };
    use crate::typst_syntax::Spanned;

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

    fn math(name: &str) -> Value {
        Library::get().math.scope().get(name).unwrap().read().clone()
    }

    fn call(func: Value, items: Vec<(Option<&str>, Value)>) -> SourceResult<Content> {
        let mut sink = Sink::new();
        let mut engine = Engine { world: fixtures::shared(), sink: &mut sink };
        let value = func.cast::<Func>().unwrap().call(&mut engine, args(items))?;
        Ok(value.cast::<Content>().unwrap())
    }

    fn sym(text: &str) -> Value {
        SymbolElem::packed(text).into_value()
    }

    #[test]
    fn math_elements_construct_from_arguments() {
        let frac = call(math("frac"), vec![(None, sym("x")), (None, sym("y"))]).unwrap();
        let frac = frac.to_packed::<FracElem>().unwrap();
        assert_eq!(frac.num, SymbolElem::packed("x"));
        assert_eq!(frac.denom, SymbolElem::packed("y"));

        let attach =
            call(math("attach"), vec![(None, sym("x")), (Some("t"), sym("2"))]).unwrap();
        let attach = attach.to_packed::<AttachElem>().unwrap();
        assert_eq!(attach.t.as_option(), &Some(Some(SymbolElem::packed("2"))));

        // `lr` joins its arguments with commas.
        let lr = call(math("lr"), vec![(None, sym("a")), (None, sym("b"))]).unwrap();
        let lr = lr.to_packed::<LrElem>().unwrap();
        assert_eq!(
            lr.body,
            SymbolElem::packed("a") + SymbolElem::packed(',') + SymbolElem::packed("b")
        );

        let root = call(math("sqrt"), vec![(None, sym("x"))]).unwrap();
        assert!(root.is::<RootElem>());

        let error = call(math("binom"), vec![(None, sym("n"))]).unwrap_err();
        assert_eq!(error[0].message, "missing argument: lower");
        let error = call(math("mat"), vec![(None, Value::Int(1))]).unwrap_err();
        assert_eq!(error[0].message, "matrices are not supported in labels");
    }

    #[test]
    fn symbols_call_their_delimiter_or_accent_function() {
        // A delimiter symbol wraps its argument in a left/right group.
        let floor = call(math("floor"), vec![(None, sym("x"))]).unwrap();
        let floor = floor.to_packed::<LrElem>().unwrap();
        assert_eq!(
            floor.body,
            SymbolElem::packed('⌊') + SymbolElem::packed("x") + SymbolElem::packed('⌋')
        );
        // An accent symbol accents it.
        let hat = call(math("hat"), vec![(None, sym("x"))]).unwrap();
        let hat = hat.to_packed::<AccentElem>().unwrap();
        assert_eq!(hat.accent, Accent('\u{0302}'));
        // Other symbols aren't callable.
        let error = Value::Symbol(Symbol::single("π")).cast::<Func>().unwrap_err();
        assert_eq!(error.message(), "symbol π is not callable");
    }

    #[test]
    fn math_has_operators_spacings_and_bottom_accents() {
        assert!(math("sin").cast::<Content>().unwrap().is::<OpElem>());
        assert!(math("thin").cast::<Content>().unwrap().is::<HElem>());
        assert!(Accent('\u{0332}').is_bottom());
        assert!(Accent('⏟').is_bottom());
        assert!(!Accent('\u{0302}').is_bottom());
        let bold = call(math("bold"), vec![(None, sym("x"))]).unwrap();
        assert!(bold.is::<StyledElem>());
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
