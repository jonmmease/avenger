//! Generates upstream Typst reference fixtures for `avenger-typst-label`.
//!
//! Each fixture directory holds a `cases.toml` manifest. For every case it wraps the label
//! source in a one-box page, compiles it with upstream Typst and the fixture fonts only, and
//! writes `ref/{id}.json`: the box's frame, the resolved math IR of every inline equation, and
//! upstream's diagnostics. The Avenger tests compare against these files offline.
//!
//! ```sh
//! cargo run --release --locked --manifest-path tools/typst-upstream/references/Cargo.toml -- \
//!     avenger-typst-label/tests/fixtures/upstream_frames \
//!     avenger-typst-label/tests/fixtures/upstream_math
//! ```
//!
//! `--check` regenerates into memory and fails if any checked-in reference differs.
//! `--only <id>` restricts a run to one case.

mod cases;
mod frame;
mod math;
mod pin;
mod world;

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value as Json, json};

use crate::{cases::Manifest, world::ReferenceWorld};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

fn main() -> Result<()> {
    let mut check = false;
    let mut only = None;
    let mut dirs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--only" => only = Some(args.next().ok_or("--only needs a case id")?),
            _ if arg.starts_with("--") => return Err(format!("unknown option {arg}").into()),
            _ => dirs.push(PathBuf::from(arg)),
        }
    }
    if dirs.is_empty() {
        return Err(
            "usage: typst-upstream-references [--check] [--only <id>] <fixture-dir>...".into(),
        );
    }

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let pin = pin::Pin::load(&repo_root)?;
    pin.verify_checkout(&repo_root.join("../typst"))?;

    let fonts = world::load_fixture_fonts(&repo_root)?;
    let library = world::reference_library();

    let mut stale = Vec::new();
    let mut written = 0;
    for dir in &dirs {
        let manifest = Manifest::load(&dir.join("cases.toml"))?;
        let ref_dir = dir.join("ref");
        if !check {
            fs::create_dir_all(&ref_dir)?;
        }
        for case in &manifest.cases {
            if only.as_ref().is_some_and(|id| id != &case.id) {
                continue;
            }
            let wrapped = manifest.wrap(case);
            let reference = generate(&library, &fonts, &wrapped, &case.source)
                .map_err(|err| format!("{}: {err}", case.id))?;
            let text = pretty(&round_floats(reference), 0, 0) + "\n";
            let path = ref_dir.join(format!("{}.json", case.id));
            if check {
                if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                    stale.push(path);
                }
            } else {
                fs::write(&path, text)?;
                written += 1;
            }
        }
        if !check && only.is_none() {
            remove_orphans(&ref_dir, &manifest)?;
        }
    }

    if check {
        if !stale.is_empty() {
            let list = stale.iter().map(|path| format!("  {}", path.display()));
            let list = list.collect::<Vec<_>>().join("\n");
            return Err(format!(
                "references differ from upstream Typst {}:\n{list}",
                pin.version
            )
            .into());
        }
        println!("all references match upstream Typst {}", pin.version);
    } else {
        println!(
            "wrote {written} references with upstream Typst {}",
            pin.version
        );
    }
    Ok(())
}

/// Compiles one wrapped case and returns its reference JSON.
fn generate(
    library: &typst::utils::LazyHash<typst::Library>,
    fonts: &world::Fonts,
    wrapped: &cases::Wrapped,
    label_source: &str,
) -> Result<Json> {
    // Equation IR is captured by a show rule, so memoized realization from an earlier case must
    // not short-circuit it.
    comemo_evict();
    math::begin_capture();
    let world = ReferenceWorld::new(library.clone(), fonts, &wrapped.text)?;
    let warned = typst::compile::<typst_layout::PagedDocument>(&world);
    let equations = math::end_capture();

    let mapper = world::SpanMapper::new(&world, wrapped.offset, label_source.len());
    let warnings = world::diagnostics(&warned.warnings, &mapper);
    let mut reference = json!({
        "source": label_source,
        "repr": content_repr(library, &world, label_source),
        "warnings": warnings,
    });
    match warned.output {
        Ok(document) => {
            let page = document
                .pages()
                .first()
                .ok_or("compiled document has no pages")?;
            let label = frame::find_label_box(&page.frame).ok_or("no box frame in the page")?;
            let mut fonts = Vec::new();
            reference["frame"] = frame::dump_frame(label, &mapper, &mut fonts);
            reference["fonts"] = Json::Array(fonts);
            reference["equations"] = Json::Array(math::finish(equations, &mapper));
        }
        Err(errors) => {
            reference["errors"] = world::diagnostics(&errors, &mapper);
        }
    }
    Ok(reference)
}

/// Upstream's repr of the label's evaluated markup, or `None` when evaluation fails. The label
/// is evaluated on its own, outside the wrapper, which evaluates it the same way.
fn content_repr(
    library: &typst::utils::LazyHash<typst::Library>,
    world: &ReferenceWorld,
    label_source: &str,
) -> Option<String> {
    use comemo::{Track, TrackedMut};
    use typst::World;
    use typst::engine::Sink;
    use typst::foundations::{Context, Repr, Scope};
    use typst::introspection::{EmptyIntrospector, Introspector};
    use typst::routines::SpanMode;
    use typst::syntax::{Span, SyntaxMode};

    let mut sink = Sink::new();
    let context = Context::none();
    let world: &dyn World = world;
    let introspector: &dyn Introspector = &EmptyIntrospector;
    let value = (library.routines.eval_string)(
        world.track(),
        library,
        TrackedMut::reborrow_mut(&mut sink.track_mut()),
        introspector.track(),
        context.track(),
        label_source,
        SpanMode::Uniform(Span::detached()),
        SyntaxMode::Markup,
        Scope::new(),
    )
    .ok()?;
    Some(value.repr().to_string())
}

/// Rounds floats to nine decimals, which removes f64 noise such as `10.223999999999998` and is
/// far below every comparison tolerance.
fn round_floats(value: Json) -> Json {
    match value {
        Json::Number(number) if number.is_f64() => {
            let value = number.as_f64().unwrap();
            let rounded = (value * 1e9).round() / 1e9;
            serde_json::Number::from_f64(if rounded == 0.0 { 0.0 } else { rounded })
                .map_or(Json::Null, Json::Number)
        }
        Json::Array(items) => Json::Array(items.into_iter().map(round_floats).collect()),
        Json::Object(map) => {
            Json::Object(map.into_iter().map(|(k, v)| (k, round_floats(v))).collect())
        }
        other => other,
    }
}

/// Pretty-prints JSON with two-space indents, keeping any value whose line fits in 160 columns
/// on that line. `used` is the width already taken on the current line.
fn pretty(value: &Json, indent: usize, used: usize) -> String {
    let compact = value.to_string();
    if used + compact.chars().count() <= 160 {
        return compact;
    }
    let pad = " ".repeat(indent + 2);
    let close = " ".repeat(indent);
    match value {
        Json::Array(items) => {
            let items = items
                .iter()
                .map(|item| format!("{pad}{}", pretty(item, indent + 2, indent + 3)));
            format!("[\n{}\n{close}]", items.collect::<Vec<_>>().join(",\n"))
        }
        Json::Object(map) => {
            let entries = map.iter().map(|(key, item)| {
                let key = Json::String(key.clone()).to_string();
                let used = indent + 2 + key.len() + 3;
                format!("{pad}{key}: {}", pretty(item, indent + 2, used))
            });
            format!("{{\n{}\n{close}}}", entries.collect::<Vec<_>>().join(",\n"))
        }
        _ => compact,
    }
}

fn comemo_evict() {
    comemo::evict(0);
}

/// Deletes references whose case was removed from the manifest.
fn remove_orphans(ref_dir: &Path, manifest: &Manifest) -> Result<()> {
    for entry in fs::read_dir(ref_dir)? {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().is_some_and(|ext| ext == "json")
            && !manifest.cases.iter().any(|case| case.id == stem)
        {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}
