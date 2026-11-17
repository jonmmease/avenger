//! The surface of the constructors of the `functions` feature: every constructor's parameters and
//! the inputs that each accepts match their snapshot, and a value means the same bare and wrapped
//! in its constructor. The tests of their SQL functions are in avenger-sql.

use std::path::Path;

use avenger_chart::functions::{
    constructor, constructors, surface, Accept, Arg, Class, Constructor, Shape,
};

#[test]
fn the_sql_surface_matches_its_snapshot() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = tests.join("output").join("constructors.txt");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let surface = surface();
    std::fs::write(&output, &surface).unwrap();
    let baseline = tests.join("baselines").join("constructors.txt");
    let expected = std::fs::read_to_string(&baseline).unwrap_or_default();
    assert!(
        expected == surface,
        "the constructors changed; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}

/// A value of the class and the shape of `accept`, or `None` for a value of another type, which
/// has no literal here.
fn sample(class: Class, shape: &Shape) -> Option<Arg> {
    Some(match (class, shape) {
        (Class::Array, Shape::Array(item)) => {
            let item_class = match item {
                Shape::Bool => Class::Bool,
                Shape::Int => Class::Int,
                Shape::Float => Class::Float,
                Shape::Array(_) => Class::Array,
                Shape::Options => Class::Struct,
                Shape::Bytes => Class::Bytes,
                _ => Class::Str,
            };
            let item = sample(item_class, item)?;
            Arg::List(vec![Some(item); 4])
        }
        (_, Shape::Enum { values, .. } | Shape::Names(values)) => {
            Arg::Str(values.first()?.name.to_string())
        }
        (Class::Bool, _) => Arg::Bool(true),
        (Class::Int, _) => Arg::Int(3),
        (Class::Float, _) => Arg::Float(2.5),
        (Class::Str, _) => Arg::Str("a".to_string()),
        (Class::Array, _) => Arg::List(vec![Some(Arg::Int(1)), Some(Arg::Int(2))]),
        (Class::Struct, _) => Arg::Struct(vec![("a".to_string(), Some(Arg::Int(1)))]),
        (Class::Bytes, _) => Arg::Bytes(Vec::new().into()),
        (Class::Other | Class::Fragment, _) => return None,
    })
}

/// The arguments of `owner` with `arg` at the parameter `index` and a value of each other required
/// parameter.
fn args(owner: &Constructor, index: usize, arg: Arg) -> Vec<Option<Arg>> {
    let mut args: Vec<Option<Arg>> = vec![None; owner.params.len()];
    for (at, param) in owner.params.iter().enumerate() {
        if param.required && at != index {
            let first = param.accepts.first().expect("an input");
            args[at] = sample(first.class, &first.shape);
        }
    }
    args[index] = Some(arg);
    args
}

/// Whether `accepts` takes a value of `class` itself, not through a fragment.
fn takes(accepts: &[Accept], class: Class, array: bool) -> bool {
    accepts.iter().any(|accept| {
        accept.class == class
            && match array {
                true => matches!(accept.shape, Shape::Array(_)) && !accept.chain.is_empty(),
                false => true,
            }
    })
}

#[test]
fn a_value_means_the_same_bare_and_wrapped_in_its_constructor() {
    let mut checked = 0;
    for owner in constructors() {
        for (index, param) in owner.params.iter().enumerate() {
            // The constructor of the parameter's own form, and whether it builds each element.
            let own = param.accepts.iter().find_map(|accept| match accept.shape {
                Shape::Message { constructor, .. } if accept.class == Class::Fragment => {
                    Some((constructor, false))
                }
                Shape::Array(Shape::Message { constructor, .. }) => Some((*constructor, true)),
                _ => None,
            });
            let Some((name, array)) = own else {
                continue;
            };
            let Some(own) = constructor(name) else {
                continue;
            };
            let Some(first) = own.params.first().filter(|first| first.positional) else {
                continue;
            };
            for accept in first.accepts {
                if !takes(
                    param.accepts,
                    if array { Class::Array } else { accept.class },
                    array,
                ) {
                    continue;
                }
                let Some(value) = sample(accept.class, &accept.shape) else {
                    continue;
                };
                let mut wrapped_args = vec![None; own.params.len()];
                wrapped_args[0] = Some(value.clone());
                let Ok(fragment) = own.build(&wrapped_args) else {
                    continue;
                };
                let (bare, wrapped) = match array {
                    true => (
                        Arg::List(vec![Some(value)]),
                        Arg::List(vec![Some(Arg::Fragment(fragment))]),
                    ),
                    false => (value, Arg::Fragment(fragment)),
                };
                let bare = owner.build(&args(owner, index, bare));
                let wrapped = owner.build(&args(owner, index, wrapped));
                match (&bare, &wrapped) {
                    (Ok(bare), Ok(wrapped)) => assert_eq!(
                        bare, wrapped,
                        "`{} => v` and `{0} => {0}(v)` differ in `{}`",
                        param.name, owner.name
                    ),
                    (Err(bare), Err(wrapped)) => {
                        assert_eq!(bare.to_string(), wrapped.to_string())
                    }
                    _ => panic!(
                        "`{} => v` of `{}` gives {bare:?}, and wrapped {wrapped:?}",
                        param.name, owner.name
                    ),
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 40, "{checked} checks");
}

#[test]
fn every_shorthand_means_the_same_bare_and_as_the_first_argument() {
    let mut unchecked = Vec::new();
    for constructor in constructors() {
        let Some(first) = constructor.params.first().filter(|first| first.positional) else {
            continue;
        };
        let mut checked = 0;
        for accept in first.accepts {
            let Some(value) = sample(accept.class, &accept.shape) else {
                continue;
            };
            let mut args = vec![None; constructor.params.len()];
            args[0] = Some(value.clone());
            let wrapped = constructor.build(&args);
            match (constructor.convert(&value), wrapped) {
                (Ok(bare), Ok(wrapped)) => assert_eq!(
                    bare, wrapped,
                    "`{}`: a value of the class {:?} bare and as its first argument",
                    constructor.name, accept.class
                ),
                // A shorthand that is not the message's only required content converts to the
                // message only with the other content, as a pin's value does.
                (Err(_), Err(_)) => {}
                (bare, wrapped) => panic!(
                    "`{}`: a value of the class {:?} gives {bare:?} bare and {wrapped:?} as its \
                     first argument",
                    constructor.name, accept.class
                ),
            }
            checked += 1;
        }
        if checked == 0 {
            unchecked.push(constructor.name);
        }
    }
    assert!(unchecked.is_empty(), "unchecked shorthands: {unchecked:?}");
}
