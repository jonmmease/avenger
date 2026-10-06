//! Avenger's stand-in for upstream's `#[elem]` attribute (`crates/typst-macros/src/elem.rs`).
//!
//! `elem!` takes an element struct written exactly as upstream writes it, with upstream's field
//! attributes and doc comments, and generates what the `#[elem]` macro generates for it:
//!
//! - the struct, where `#[required]` fields keep their type, `#[synthesized]` fields become
//!   `Option<T>`, settable fields become `Settable<Self, I>`, and `#[ghost]` and `#[external]`
//!   fields are not stored;
//! - `new` taking the required fields, and a `with_*` builder per stored field that isn't
//!   required;
//! - a `Field<Self, I>` constant per field that isn't external;
//! - the `SettableField`/`SettableProperty` and `RefableProperty` impls, with `#[default(..)]`
//!   (else `Default::default()`) and `#[fold]`;
//! - the `NativeElement` impl, and the `ShowSet` and `Synthesize` capability hooks when the
//!   element lists them. As upstream, those traits are implemented for `Packed<Self>`. Other
//!   capabilities are ignored;
//! - upstream's generated `Construct` impl, unless the element lists `Construct` and
//!   implements it itself. Fields are parsed from the arguments as upstream parses them, and a
//!   `#[parse(..)]` field by the element's `parse_<field>(engine, args)` function, which holds
//!   upstream's parse expression: the expression can't name a macro-made parameter, since
//!   hygiene separates them;
//! - equality over the stored fields, and upstream's generic `name(field: value, ..)` repr over
//!   the fields that are present. Both skip `#[internal]` fields, as upstream's field vtables do.
//!
//! An invocation writes the element unindented, exactly as upstream writes it; rustfmt leaves
//! the bodies of brace-delimited macro calls alone, so they stay that way.
//!
//! Upstream derives an element's name from its type; `elem!` needs it spelled out, as in
//! `#[elem(name = "strong", ...)]`. Elements have no title, which only documentation reads. An
//! element that lists `Debug`, `Repr`, `PartialEq` or `Construct` implements that trait itself,
//! as upstream. Fields are numbered in
//! declaration order, where upstream numbers internal fields last; the numbers are only ever
//! compared with each other.

/// Declares a native element. See the module documentation.
macro_rules! elem {
    // Entry point. The state threaded through the field munching is
    // `[header idx sfields params inits records rest]`.
    (
        $(#[doc = $doc:literal])*
        #[elem $(($($meta:tt)*))?]
        $(#[$attr:meta])*
        $vis:vis struct $Name:ident {
            $($body:tt)*
        }
    ) => {
        $crate::typst_library::foundations::elem! { @munch
            [$Name [$vis] [$(#[doc = $doc])*] [$($($meta)*)?] [$(#[$attr])*]]
            [0] [] [] [] [] [$($body)*]
        }
    };

    // Splits off the next field.
    (@munch $header:tt $idx:tt $sfields:tt $params:tt $inits:tt $records:tt
        [$(#[$($fattr:tt)*])* $fvis:vis $fname:ident : $fty:ty $(, $($rest:tt)*)?]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs
            [$header $idx $sfields $params $inits $records [$($($rest)*)?]]
            [[$fvis] $fname [$fty]]
            [] [[settable] [] [no] [no] [no] [named]]
            [$([$($fattr)*])*]
        }
    };

    // All fields are split off: find the traits the element implements itself.
    (@munch [$Name:ident $vis:tt $docs:tt [$($meta:tt)*] $attrs:tt]
        $idx:tt $sfields:tt $params:tt $inits:tt $records:tt []
    ) => {
        $crate::typst_library::foundations::elem! { @flags
            [[$Name $vis $docs [$($meta)*] $attrs] $sfields $params $inits $records]
            [[derive] [derive] [generic] [generated]]
            [$($meta)*]
        }
    };

    // Field attributes. The state after the field is `[docs] [flags] [attrs]`, where the flags
    // are `[kind] [default] [fold] [internal] [parse] [arg]`. Doc lines, which make up most
    // attributes, are taken up to sixteen at a time to keep the recursion shallow.
    (@attrs $state:tt $field:tt [$($docs:tt)*] $flags:tt
        [[doc = $d0:literal] [doc = $d1:literal] [doc = $d2:literal] [doc = $d3:literal]
         [doc = $d4:literal] [doc = $d5:literal] [doc = $d6:literal] [doc = $d7:literal]
         [doc = $d8:literal] [doc = $d9:literal] [doc = $d10:literal] [doc = $d11:literal]
         [doc = $d12:literal] [doc = $d13:literal] [doc = $d14:literal] [doc = $d15:literal]
         $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field
            [$($docs)* #[doc = $d0] #[doc = $d1] #[doc = $d2] #[doc = $d3]
                #[doc = $d4] #[doc = $d5] #[doc = $d6] #[doc = $d7]
                #[doc = $d8] #[doc = $d9] #[doc = $d10] #[doc = $d11]
                #[doc = $d12] #[doc = $d13] #[doc = $d14] #[doc = $d15]]
            $flags [$($more)*] }
    };
    (@attrs $state:tt $field:tt [$($docs:tt)*] $flags:tt
        [[doc = $d0:literal] [doc = $d1:literal] [doc = $d2:literal] [doc = $d3:literal]
         [doc = $d4:literal] [doc = $d5:literal] [doc = $d6:literal] [doc = $d7:literal]
         $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field
            [$($docs)* #[doc = $d0] #[doc = $d1] #[doc = $d2] #[doc = $d3]
                #[doc = $d4] #[doc = $d5] #[doc = $d6] #[doc = $d7]]
            $flags [$($more)*] }
    };
    (@attrs $state:tt $field:tt [$($docs:tt)*] $flags:tt
        [[doc = $d0:literal] [doc = $d1:literal] [doc = $d2:literal] [doc = $d3:literal]
         $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field
            [$($docs)* #[doc = $d0] #[doc = $d1] #[doc = $d2] #[doc = $d3]]
            $flags [$($more)*] }
    };
    (@attrs $state:tt $field:tt [$($docs:tt)*] $flags:tt
        [[doc = $d0:literal] [doc = $d1:literal] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field
            [$($docs)* #[doc = $d0] #[doc = $d1]] $flags [$($more)*] }
    };
    (@attrs $state:tt $field:tt [$($docs:tt)*] $flags:tt
        [[doc = $doc:literal] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field
            [$($docs)* #[doc = $doc]] $flags [$($more)*] }
    };
    // An external field stays external when it is also required or variadic.
    (@attrs $state:tt $field:tt $docs:tt
        [[external] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[required] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[external] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [[external] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[variadic] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[external] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[required] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[required] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[variadic] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[required] $default $fold $internal $parse [variadic]] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[ghost] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[ghost] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[synthesized] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[synthesized] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[external] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [[external] $default $fold $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[fold] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [$kind $default [yes] $internal $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[internal] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [$kind $default $fold [yes] $parse $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[default($($value:tt)*)] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [$kind [$($value)*] $fold $internal $parse $arg] [$($more)*] }
    };
    // `#[parse(..)]` fields are parsed by the element's `parse_*` function.
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[parse $($expr:tt)*] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [$kind $default $fold $internal [yes] $arg] [$($more)*] }
    };
    (@attrs $state:tt $field:tt $docs:tt
        [$kind:tt $default:tt $fold:tt $internal:tt $parse:tt $arg:tt]
        [[positional] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs
            [$kind $default $fold $internal $parse [positional]] [$($more)*] }
    };
    // `#[named]`, a bare `#[default]`, `#[borrowed]` and the like.
    (@attrs $state:tt $field:tt $docs:tt $flags:tt
        [[$other:ident $($args:tt)*] $($more:tt)*]
    ) => {
        $crate::typst_library::foundations::elem! { @attrs $state $field $docs $flags
            [$($more)*] }
    };

    // The attributes are done: accumulate the field by kind and split off the next one.
    (@attrs [$h:tt [$($idx:tt)*] [$($sf:tt)*] [$($p:tt)*] [$($in:tt)*] [$($r:tt)*] $rest:tt]
        [[$fvis:vis] $fname:ident [$fty:ty]]
        [$($docs:tt)*] [[required] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt] []
    ) => {
        $crate::typst_library::foundations::elem! { @munch $h [$($idx)* + 1]
            [$($sf)* $($docs)* $fvis $fname: $fty,]
            [$($p)* $fname: $fty,]
            [$($in)* $fname,]
            [$($r)* { required [$($idx)*] [$fvis] $fname [$fty] $default $fold $internal
                $parse $arg }]
            $rest
        }
    };
    (@attrs [$h:tt [$($idx:tt)*] [$($sf:tt)*] $p:tt [$($in:tt)*] [$($r:tt)*] $rest:tt]
        [[$fvis:vis] $fname:ident [$fty:ty]]
        [$($docs:tt)*] [[synthesized] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt] []
    ) => {
        $crate::typst_library::foundations::elem! { @munch $h [$($idx)* + 1]
            [$($sf)* $($docs)* $fvis $fname: ::std::option::Option<$fty>,]
            $p
            [$($in)* $fname: ::std::option::Option::None,]
            [$($r)* { synthesized [$($idx)*] [$fvis] $fname [$fty] $default $fold $internal
                $parse $arg }]
            $rest
        }
    };
    (@attrs [$h:tt [$($idx:tt)*] [$($sf:tt)*] $p:tt [$($in:tt)*] [$($r:tt)*] $rest:tt]
        [[$fvis:vis] $fname:ident [$fty:ty]]
        [$($docs:tt)*] [[settable] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt] []
    ) => {
        $crate::typst_library::foundations::elem! { @munch $h [$($idx)* + 1]
            [$($sf)* $($docs)* $fvis $fname:
                $crate::typst_library::foundations::Settable<Self, { $($idx)* }>,]
            $p
            [$($in)* $fname: $crate::typst_library::foundations::Settable::new(),]
            [$($r)* { settable [$($idx)*] [$fvis] $fname [$fty] $default $fold $internal
                $parse $arg }]
            $rest
        }
    };
    (@attrs [$h:tt [$($idx:tt)*] $sf:tt $p:tt $in:tt [$($r:tt)*] $rest:tt]
        [[$fvis:vis] $fname:ident [$fty:ty]]
        $docs:tt [[$kind:ident] $default:tt $fold:tt $internal:tt $parse:tt $arg:tt] []
    ) => {
        $crate::typst_library::foundations::elem! { @munch $h [$($idx)* + 1] $sf $p $in
            [$($r)* { $kind [$($idx)*] [$fvis] $fname [$fty] $default $fold $internal
                $parse $arg }]
            $rest
        }
    };

    // Whether the element implements `Debug`, `PartialEq`, `Repr` and `Construct` itself.
    (@flags $state:tt [$debug:tt $eq:tt $repr:tt $construct:tt] [Debug $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @flags $state
            [[manual] $eq $repr $construct] [$($meta)*] }
    };
    (@flags $state:tt [$debug:tt $eq:tt $repr:tt $construct:tt] [PartialEq $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @flags $state
            [$debug [manual] $repr $construct] [$($meta)*] }
    };
    (@flags $state:tt [$debug:tt $eq:tt $repr:tt $construct:tt] [Repr $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @flags $state
            [$debug $eq [manual] $construct] [$($meta)*] }
    };
    (@flags $state:tt [$debug:tt $eq:tt $repr:tt $construct:tt] [Construct $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @flags $state
            [$debug $eq $repr [manual]] [$($meta)*] }
    };
    (@flags $state:tt $flags:tt [$skip:tt $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @flags $state $flags [$($meta)*] }
    };
    (@flags $state:tt $flags:tt []) => {
        $crate::typst_library::foundations::elem! { @emit $state $flags }
    };

    // Emits the element.
    (@emit
        [[$Name:ident [$vis:vis] [$($docs:tt)*] [$($meta:tt)*] [$($attrs:tt)*]]
            [$($sf:tt)*] [$($p:tt)*] [$($in:tt)*] [$($r:tt)*]]
        [[$debug:ident] [$eq:ident] [$repr:ident] [$construct:ident]]
    ) => {
        $crate::typst_library::foundations::elem! { @struct $debug
            [$($docs)* $($attrs)*] [$vis] $Name [$($sf)*]
        }

        impl $Name {
            /// Create a new instance of the element.
            pub fn new($($p)*) -> Self {
                Self { $($in)* }
            }

            $($crate::typst_library::foundations::elem! { @with $r })*
        }

        #[allow(non_upper_case_globals)]
        impl $Name {
            $($crate::typst_library::foundations::elem! { @const $r })*
        }

        $($crate::typst_library::foundations::elem! { @property $Name $r })*

        $crate::typst_library::foundations::elem! { @eq $eq $Name [$($r)*] }

        $crate::typst_library::foundations::elem! { @construct $construct $Name [$($in)*] [$($r)*] }

        impl $crate::typst_library::foundations::NativeElement for $Name {
            const ELEM: $crate::typst_library::foundations::Element = {
                static DATA: $crate::typst_library::foundations::NativeElementData =
                    $crate::typst_library::foundations::NativeElementData {
                        name: $crate::typst_library::foundations::elem!(@name [$($meta)*]),
                        field_names: &[$($crate::typst_library::foundations::elem!(
                            @record_name $r
                        ),)*],
                        construct:
                            <$Name as $crate::typst_library::foundations::Construct>::construct,
                    };
                $crate::typst_library::foundations::Element::from_data(&DATA)
            };

            #[allow(unused_variables)]
            fn repr(
                packed: &$crate::typst_library::foundations::Packed<Self>,
            ) -> ::ecow::EcoString {
                $crate::typst_library::foundations::elem! { @repr $repr packed [$($meta)*] [$($r)*] }
            }

            $crate::typst_library::foundations::elem! { @caps [$($meta)*] }
        }

        $crate::typst_library::foundations::elem! { @mathy $Name [$($meta)*] }
    };

    // The `Mathy` marker, for elements that list it.
    (@mathy $Name:ident [Mathy $($meta:tt)*]) => {
        impl $crate::typst_library::math::Mathy
            for $crate::typst_library::foundations::Packed<$Name>
        {
        }
    };
    (@mathy $Name:ident [$skip:tt $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @mathy $Name [$($meta)*] }
    };
    (@mathy $Name:ident []) => {};

    (@struct derive [$($attrs:tt)*] [$vis:vis] $Name:ident [$($sf:tt)*]) => {
        $($attrs)*
        #[derive(Debug, Clone)]
        $vis struct $Name { $($sf)* }
    };
    (@struct manual [$($attrs:tt)*] [$vis:vis] $Name:ident [$($sf:tt)*]) => {
        $($attrs)*
        #[derive(Clone)]
        $vis struct $Name { $($sf)* }
    };

    // Builder-style setters, for stored fields that aren't required.
    (@with { settable $idx:tt [$fvis:vis] $fname:ident [$fty:ty] $($rest:tt)* }) => {
        ::paste::paste! {
            #[doc = concat!(
                "Builder-style setter for the [`", stringify!($fname), "`](Self::",
                stringify!($fname), ") field."
            )]
            $fvis fn [<with_ $fname>](mut self, $fname: $fty) -> Self {
                self.$fname.set($fname);
                self
            }
        }
    };
    (@with { synthesized $idx:tt [$fvis:vis] $fname:ident [$fty:ty] $($rest:tt)* }) => {
        ::paste::paste! {
            #[doc = concat!(
                "Builder-style setter for the [`", stringify!($fname), "`](Self::",
                stringify!($fname), ") field."
            )]
            $fvis fn [<with_ $fname>](mut self, $fname: $fty) -> Self {
                self.$fname = ::std::option::Option::Some($fname);
                self
            }
        }
    };
    (@with { $kind:ident $($rest:tt)* }) => {};

    // Field accessor constants, for fields that aren't external.
    (@const { external $($rest:tt)* }) => {};
    (@const { $kind:ident [$($idx:tt)*] [$fvis:vis] $fname:ident $($rest:tt)* }) => {
        $fvis const $fname: $crate::typst_library::foundations::Field<Self, { $($idx)* }> =
            $crate::typst_library::foundations::Field::new();
    };

    (@record_name { $kind:ident $idx:tt $fvis:tt $fname:ident $($rest:tt)* }) => {
        $crate::typst_library::foundations::elem!(@field_name $fname)
    };

    // Settable property impls.
    (@property $Name:ident {
        settable [$($idx:tt)*] [$fvis:vis] $fname:ident [$fty:ty] [$($default:tt)*] [$fold:ident]
        $internal:tt $parse:tt $arg:tt
    }) => {
        impl $crate::typst_library::foundations::SettableField<{ $($idx)* }> for $Name {
            type Type = $fty;
            const FIELD: $crate::typst_library::foundations::SettableFieldData<Self, { $($idx)* }> =
                $crate::typst_library::foundations::elem!(@fold $fold
                    $crate::typst_library::foundations::SettableFieldData::<Self, { $($idx)* }>::new(
                        |elem| &elem.$fname,
                        |elem| &mut elem.$fname,
                        || $crate::typst_library::foundations::elem!(@default [$($default)*]),
                        || {
                            static LOCK: ::std::sync::OnceLock<$fty> = ::std::sync::OnceLock::new();
                            &LOCK
                        },
                    ));
        }
        $crate::typst_library::foundations::elem! { @refable $Name [$($idx)*] [$fold] }
    };
    (@property $Name:ident {
        ghost [$($idx:tt)*] [$fvis:vis] $fname:ident [$fty:ty] [$($default:tt)*] [$fold:ident]
        $internal:tt $parse:tt $arg:tt
    }) => {
        impl $crate::typst_library::foundations::SettableProperty<{ $($idx)* }> for $Name {
            type Type = $fty;
            const FIELD: $crate::typst_library::foundations::SettablePropertyData<Self, { $($idx)* }> =
                $crate::typst_library::foundations::elem!(@fold $fold
                    $crate::typst_library::foundations::SettablePropertyData::<Self, { $($idx)* }>::new(
                        || $crate::typst_library::foundations::elem!(@default [$($default)*]),
                        || {
                            static LOCK: ::std::sync::OnceLock<$fty> = ::std::sync::OnceLock::new();
                            &LOCK
                        },
                    ));
        }
        $crate::typst_library::foundations::elem! { @refable $Name [$($idx)*] [$fold] }
    };
    (@property $Name:ident { $kind:ident $($rest:tt)* }) => {};

    (@refable $Name:ident [$($idx:tt)*] [no]) => {
        impl $crate::typst_library::foundations::RefableProperty<{ $($idx)* }> for $Name {}
    };
    (@refable $Name:ident [$($idx:tt)*] [yes]) => {};

    (@fold yes $data:expr) => { $data.with_fold() };
    (@fold no $data:expr) => { $data };

    (@default []) => { ::std::default::Default::default() };
    (@default [$($value:tt)+]) => { $($value)+ };

    // Equality over the stored fields that aren't internal, unless the element implements it.
    (@eq manual $Name:ident $records:tt) => {};
    (@eq derive $Name:ident [$($r:tt)*]) => {
        impl ::std::cmp::PartialEq for $Name {
            #[allow(unused_variables)]
            fn eq(&self, other: &Self) -> bool {
                true $(&& $crate::typst_library::foundations::elem!(@eq_field self other $r))*
            }
        }
    };
    (@eq_field $a:ident $b:ident {
        $kind:ident $idx:tt $fvis:tt $fname:ident $fty:tt $default:tt $fold:tt [yes] $($rest:tt)*
    }) => {
        true
    };
    (@eq_field $a:ident $b:ident { required $idx:tt $fvis:tt $fname:ident $($rest:tt)* }) => {
        $a.$fname == $b.$fname
    };
    (@eq_field $a:ident $b:ident { synthesized $idx:tt $fvis:tt $fname:ident $($rest:tt)* }) => {
        $a.$fname == $b.$fname
    };
    (@eq_field $a:ident $b:ident { settable $idx:tt $fvis:tt $fname:ident $($rest:tt)* }) => {
        $a.$fname.as_option() == $b.$fname.as_option()
    };
    (@eq_field $a:ident $b:ident { $kind:ident $($rest:tt)* }) => {
        true
    };

    // Upstream's generated constructor, unless the element implements `Construct` itself: the
    // fields that aren't external, synthesized or internal are parsed from the arguments in
    // declaration order. A `#[parse(..)]` field is parsed by the element's `parse_<field>`
    // function, which holds upstream's parse expression with `engine` and `args` in scope.
    (@construct manual $Name:ident $inits:tt $records:tt) => {};
    (@construct generated $Name:ident [$($in:tt)*] [$($r:tt)*]) => {
        impl $crate::typst_library::foundations::Construct for $Name {
            #[allow(unused_variables, unused_mut)]
            fn construct(
                engine: &mut $crate::typst_library::engine::Engine,
                args: &mut $crate::typst_library::foundations::Args,
            ) -> $crate::typst_library::diag::SourceResult<
                $crate::typst_library::foundations::Content,
            > {
                $($crate::typst_library::foundations::elem! { @setup engine args $r })*
                let mut element = Self { $($in)* };
                $($crate::typst_library::foundations::elem! { @apply element $r })*
                ::std::result::Result::Ok($crate::typst_library::foundations::Content::new(
                    element,
                ))
            }
        }
    };

    (@setup $engine:ident $args:ident { external $($rest:tt)* }) => {};
    (@setup $engine:ident $args:ident { ghost $($rest:tt)* }) => {};
    (@setup $engine:ident $args:ident {
        $kind:ident $idx:tt $fvis:tt $fname:ident $fty:tt $default:tt $fold:tt [yes] [no] $arg:tt
    }) => {};
    (@setup $engine:ident $args:ident {
        synthesized $idx:tt $fvis:tt $fname:ident $fty:tt $default:tt $fold:tt $internal:tt [no]
        $arg:tt
    }) => {};
    (@setup $engine:ident $args:ident {
        required $idx:tt $fvis:tt $fname:ident [$fty:ty] $default:tt $fold:tt $internal:tt
        [$parse:ident] [$arg:ident]
    }) => {
        let $fname: $fty = $crate::typst_library::foundations::elem!(
            @parser $engine $args required $parse $arg $fname
        );
    };
    (@setup $engine:ident $args:ident {
        $kind:ident $idx:tt $fvis:tt $fname:ident [$fty:ty] $default:tt $fold:tt $internal:tt
        [$parse:ident] [$arg:ident]
    }) => {
        let $fname: ::std::option::Option<$fty> = $crate::typst_library::foundations::elem!(
            @parser $engine $args settable $parse $arg $fname
        );
    };

    (@parser $engine:ident $args:ident $kind:ident yes $arg:ident $fname:ident) => {
        ::paste::paste! { Self::[<parse_ $fname>]($engine, $args)? }
    };
    (@parser $engine:ident $args:ident required no variadic $fname:ident) => {
        $args.all()?
    };
    (@parser $engine:ident $args:ident required no $arg:ident $fname:ident) => {
        $args.expect($crate::typst_library::foundations::elem!(@field_name $fname))?
    };
    (@parser $engine:ident $args:ident settable no positional $fname:ident) => {
        $args.find()?
    };
    (@parser $engine:ident $args:ident settable no $arg:ident $fname:ident) => {
        $args.named($crate::typst_library::foundations::elem!(@field_name $fname))?
    };

    // Stores the parsed settable fields; required fields are stored by `new`'s initializers.
    (@apply $element:ident {
        settable $idx:tt $fvis:tt $fname:ident $fty:tt $default:tt $fold:tt [yes] [no] $arg:tt
    }) => {};
    (@apply $element:ident { settable $idx:tt $fvis:tt $fname:ident $($rest:tt)* }) => {
        $element.$fname = $crate::typst_library::foundations::Settable::from($fname);
    };
    (@apply $element:ident { $kind:ident $($rest:tt)* }) => {};

    // The element's repr: its own `Repr` impl, or upstream's generic `name(field: value, ..)`
    // over the fields that are present and aren't internal.
    (@repr manual $packed:ident $meta:tt $records:tt) => {
        $crate::typst_library::foundations::Repr::repr(&**$packed)
    };
    (@repr generic $packed:ident $meta:tt [$($r:tt)*]) => {{
        #[allow(unused_mut)]
        let mut fields: ::std::vec::Vec<::ecow::EcoString> = ::std::vec::Vec::new();
        $($crate::typst_library::foundations::elem! { @repr_field $packed fields $r })*
        ::ecow::eco_format!(
            "{}{}",
            $crate::typst_library::foundations::elem!(@name $meta),
            $crate::typst_library::foundations::repr::pretty_array_like(&fields, false),
        )
    }};
    (@repr_field $packed:ident $fields:ident {
        $kind:ident $idx:tt $fvis:tt $fname:ident $fty:tt $default:tt $fold:tt [yes] $($rest:tt)*
    }) => {};
    (@repr_field $packed:ident $fields:ident {
        required $idx:tt $fvis:tt $fname:ident $($rest:tt)*
    }) => {
        $fields.push($crate::typst_library::foundations::elem!(
            @repr_value $fname $packed.$fname.clone()
        ));
    };
    (@repr_field $packed:ident $fields:ident {
        synthesized $idx:tt $fvis:tt $fname:ident $($rest:tt)*
    }) => {
        if let ::std::option::Option::Some(value) = &$packed.$fname {
            $fields.push($crate::typst_library::foundations::elem!(@repr_value $fname value.clone()));
        }
    };
    (@repr_field $packed:ident $fields:ident {
        settable $idx:tt $fvis:tt $fname:ident $($rest:tt)*
    }) => {
        if let ::std::option::Option::Some(value) = $packed.$fname.as_option() {
            $fields.push($crate::typst_library::foundations::elem!(@repr_value $fname value.clone()));
        }
    };
    (@repr_field $packed:ident $fields:ident { $kind:ident $($rest:tt)* }) => {};
    (@repr_value $fname:ident $value:expr) => {
        ::ecow::eco_format!(
            "{}: {}",
            $crate::typst_library::foundations::elem!(@field_name $fname),
            $crate::typst_library::foundations::Repr::repr(
                &$crate::typst_library::foundations::IntoValue::into_value($value),
            ),
        )
    };

    // Upstream exposes fields in kebab case.
    (@field_name $fname:ident) => {{
        const NAME: &str = {
            const BYTES: [u8; stringify!($fname).len()] =
                $crate::typst_library::foundations::kebab_case(stringify!($fname));
            match ::std::str::from_utf8(&BYTES) {
                ::std::result::Result::Ok(name) => name,
                ::std::result::Result::Err(_) => panic!("field names are ASCII"),
            }
        };
        NAME
    }};

    // The element's name from the `#[elem(..)]` metadata.
    (@name [name = $name:literal $($meta:tt)*]) => { $name };
    (@name [$skip:tt $($meta:tt)*]) => { $crate::typst_library::foundations::elem!(@name [$($meta)*]) };
    (@name []) => { compile_error!("`elem!` needs `name = \"..\"` in `#[elem(..)]`") };

    // Capability hooks for the traits the element lists.
    (@caps [ShowSet $($meta:tt)*]) => {
        fn as_show_set(
            packed: &$crate::typst_library::foundations::Packed<Self>,
        ) -> ::std::option::Option<&(dyn $crate::typst_library::foundations::ShowSet + 'static)> {
            ::std::option::Option::Some(packed)
        }
        $crate::typst_library::foundations::elem! { @caps [$($meta)*] }
    };
    (@caps [Mathy $($meta:tt)*]) => {
        fn as_mathy(
            packed: &$crate::typst_library::foundations::Packed<Self>,
        ) -> ::std::option::Option<&(dyn $crate::typst_library::math::Mathy + 'static)> {
            ::std::option::Option::Some(packed)
        }
        $crate::typst_library::foundations::elem! { @caps [$($meta)*] }
    };
    (@caps [Synthesize $($meta:tt)*]) => {
        fn as_synthesize(
            packed: &$crate::typst_library::foundations::Packed<Self>,
        ) -> ::std::option::Option<&(dyn $crate::typst_library::foundations::Synthesize + 'static)> {
            ::std::option::Option::Some(packed)
        }
        fn as_synthesize_mut(
            packed: &mut $crate::typst_library::foundations::Packed<Self>,
        ) -> ::std::option::Option<
            &mut (dyn $crate::typst_library::foundations::Synthesize + 'static),
        > {
            ::std::option::Option::Some(packed)
        }
        $crate::typst_library::foundations::elem! { @caps [$($meta)*] }
    };
    (@caps [$skip:tt $($meta:tt)*]) => {
        $crate::typst_library::foundations::elem! { @caps [$($meta)*] }
    };
    (@caps []) => {};
}

pub(crate) use elem;

/// Converts a snake case field name to upstream's kebab case at compile time.
#[doc(hidden)]
pub const fn kebab_case<const N: usize>(name: &str) -> [u8; N] {
    let bytes = name.as_bytes();
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = if bytes[i] == b'_' { b'-' } else { bytes[i] };
        i += 1;
    }
    out
}
