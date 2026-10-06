//! Tests for Avenger's parts of the shared kernel: the `elem!`, `cast!` and `derive_cast!`
//! stand-ins for upstream's procedural macros, the style chain semantics the pipeline depends
//! on, and colors.

use crate::typst_library::foundations::{
    Content, Depth, IntoValue, NativeElement, Packed, Repr, Resolve, SequenceElem,
    ShowSet, Smart, StyleChain, StyledElem, Styles, Value, dict, elem,
};
use crate::typst_library::layout::{Abs, Dir, Em, Length};
use crate::typst_library::math::{EquationElem, MathSize};
use crate::typst_library::text::{Lang, TextDir, TextElem, TextSize};
use crate::typst_library::visualize::{
    Color, ColorExt, FillRule, FixedStroke, LineCap, Paint, Stroke,
};
use crate::typst_syntax::{FileId, Span};

elem! {
    /// An element that exercises each kind of field.
    #[elem(name = "probe", ShowSet)]
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
        assert!(!Content::new(EquationElem::new(Content::empty())).can::<dyn ShowSet>());
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

        let color = Value::Color(Color::WHITE).cast::<Stroke>().unwrap();
        assert_eq!(color.paint, Smart::Custom(Paint::Solid(Color::WHITE)));

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
    fn strings_and_none_cast_to_content() {
        assert_eq!(
            Value::Str("a".into()).cast::<Content>().unwrap(),
            TextElem::packed("a")
        );
        assert!(Value::None.cast::<Content>().unwrap().is_empty());
        let error = Value::Int(1).cast::<Content>().unwrap_err();
        assert_eq!(error.message(), "expected content, found integer");
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
        assert_eq!(Paint::Solid(Color::WHITE).repr(), r##"rgb("#ffffff")"##);
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
