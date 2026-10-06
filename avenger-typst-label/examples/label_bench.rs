//! Times label compilation, with and without system fonts, and a few font primitives.
//!
//! ```sh
//! cargo run --release -p avenger-typst-label --example label_bench
//! cargo run --release -p avenger-typst-label --example label_bench -- loop <case|source> [secs] [nosys]
//! ```
//!
//! The first form prints the median and mean time per case. The second compiles one case in a
//! loop for a profiler such as `sample <pid>` or `perf record -p <pid>`. Timings depend on the
//! machine and its load; compare runs made on the same machine.

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use avenger_typst_label::{EngineOptions, LabelEngine, LabelOptions, RegisteredFont};

/// The cases, by name and source.
const CASES: &[(&str, &str)] = &[
    ("plain", "Revenue 2024"),
    ("plain-long", "Quarterly revenue by product line, 2019-2024"),
    ("markup", "*Bold* and _italic_ #underline[text]"),
    ("math-simple", "$x^2 + y^2$"),
    ("math-moderate", "$sum_(i=0)^n x_i / sqrt(n)$"),
    ("mixed", "Growth $R^2 = 0.94$ in 2024"),
    ("emoji", "Revenue 🚀"),
    ("cjk", "Hello 温度"),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("loop") {
        let case = args.get(2).expect("a case name or source");
        let secs = args.get(3).and_then(|secs| secs.parse().ok()).unwrap_or(10);
        let system_fonts = args.get(4).map(String::as_str) != Some("nosys");
        profile(case, secs, system_fonts);
        return;
    }

    let budget = Duration::from_millis(400);
    let options = LabelOptions::default();
    for system_fonts in [false, true] {
        let start = Instant::now();
        let engine = LabelEngine::new(engine_options(system_fonts));
        let creation = start.elapsed().as_secs_f64() * 1e3;
        println!("\n== system fonts: {system_fonts}; LabelEngine::new {creation:.1} ms");
        println!(
            "{:<14} {:>12} {:>12} {:>12} {:>7}",
            "case", "compile med", "compile mean", "measure med", "iters"
        );
        for (name, source) in CASES {
            if let Err(err) = engine.compile(source, &options) {
                println!("{name:<14} (error: {err:?})");
                continue;
            }
            let (median, mean, iters) =
                time(budget, || engine.compile(source, &options).unwrap());
            let (measure, _, _) =
                time(budget, || engine.measure(source, &options).unwrap());
            println!(
                "{name:<14} {median:>10.1}us {mean:>10.1}us {measure:>10.1}us {iters:>7}"
            );
        }

        // A math family that isn't installed.
        let mut missing = LabelOptions::default();
        missing.math.font_family = "New Computer Modern Math".into();
        let source = "$x^2 + y^2$";
        match engine.compile(source, &missing) {
            Ok(_) => {
                let (median, mean, iters) =
                    time(budget, || engine.compile(source, &missing).unwrap());
                println!(
                    "{:<14} {median:>10.1}us {mean:>10.1}us {:>12} {iters:>7}",
                    "math-missing", ""
                );
            }
            Err(err) => println!("math-missing   (error: {err:?})"),
        }

        let (median, _, _) =
            time(budget, || engine.compile_text("Revenue 2024", &options).unwrap());
        println!("{:<14} {median:>10.1}us   (compile_text)", "plain-text");
    }

    primitives();
}

/// Compiles one case in a loop, for a profiler.
fn profile(case: &str, secs: u64, system_fonts: bool) {
    let engine = LabelEngine::new(engine_options(system_fonts));
    let options = LabelOptions::default();
    let source = CASES
        .iter()
        .find(|(name, _)| *name == case)
        .map_or(case, |(_, source)| *source);
    let end = Instant::now() + Duration::from_secs(secs);
    let mut iterations = 0u64;
    eprintln!("pid {} compiling {source:?} in a loop", std::process::id());
    while Instant::now() < end {
        std::hint::black_box(engine.compile(source, &options).unwrap());
        iterations += 1;
    }
    eprintln!("{iterations} iterations");
}

/// The fixture fonts, with Lato as the default family.
fn engine_options(system_fonts: bool) -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = system_fonts;
    options.fonts.default_sans_serif_family = Some("Lato".into());
    options.fonts.default_monospace_family = Some("DejaVu Sans Mono".into());
    options.fonts.default_math_family = Some("Lete Sans Math".into());
    options.fonts.registered_fonts = [
        avenger_fonts::LATO_LIGHT,
        avenger_fonts::LATO_ITALIC,
        avenger_fonts::LATO_MEDIUM,
        avenger_fonts::LATO_BOLD,
        avenger_fonts::DEJAVU_SANS_MONO,
        avenger_fonts::LETE_SANS_MATH,
        avenger_fonts::LETE_SANS_MATH_BOLD,
    ]
    .iter()
    .map(|compressed| RegisteredFont::new(decompress(compressed)))
    .collect();
    options
}

fn decompress(compressed: &[u8]) -> Arc<[u8]> {
    let mut data = Vec::new();
    brotli::Decompressor::new(compressed, 4096)
        .read_to_end(&mut data)
        .unwrap();
    data.into()
}

/// The median and mean wall time of `f` in microseconds, and the number of samples.
fn time<T>(budget: Duration, mut f: impl FnMut() -> T) -> (f64, f64, usize) {
    for _ in 0..3 {
        std::hint::black_box(f());
    }
    let mut samples = Vec::new();
    let start = Instant::now();
    while samples.len() < 5 || (start.elapsed() < budget && samples.len() < 2000) {
        let sample = Instant::now();
        std::hint::black_box(f());
        samples.push(sample.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    (samples[samples.len() / 2], mean, samples.len())
}

/// The costs that font caching avoids: parsing faces, building shaping faces and plans, and
/// copying or comparing font data.
fn primitives() {
    println!("\n== font primitives (median us)");
    let budget = Duration::from_millis(300);
    let mut fonts = vec![
        ("Lato-Light", decompress(avenger_fonts::LATO_LIGHT)),
        ("LeteSansMath", decompress(avenger_fonts::LETE_SANS_MATH)),
    ];
    for (name, path) in [
        ("SFNS", "/System/Library/Fonts/SFNS.ttf"),
        ("HiraginoSansGB", "/System/Library/Fonts/Hiragino Sans GB.ttc"),
        ("AppleColorEmoji", "/System/Library/Fonts/Apple Color Emoji.ttc"),
    ] {
        if let Ok(data) = std::fs::read(path) {
            fonts.push((name, data.into()));
        }
    }
    println!(
        "{:<16} {:>10} {:>10} {:>10} {:>11} {:>10}",
        "font", "bytes", "ttf parse", "rb face", "Arc copy", "slice eq"
    );
    for (name, data) in &fonts {
        let (parse, _, _) = time(budget, || ttf_parser::Face::parse(data, 0).is_ok());
        let (face, _, _) =
            time(budget, || rustybuzz::Face::from_slice(data, 0).is_some());
        let (copy, _, _) = time(budget, || Arc::<[u8]>::from(&data[..]));
        let other = data.clone();
        let (eq, _, _) = time(budget, || {
            std::hint::black_box(data.as_ref()) == std::hint::black_box(other.as_ref())
        });
        println!(
            "{name:<16} {:>10} {parse:>10.2} {face:>10.2} {copy:>11.1} {eq:>10.1}",
            data.len()
        );
    }

    let face = rustybuzz::Face::from_slice(&fonts[0].1, 0).unwrap();
    let text = "Quarterly revenue by product line";
    let plan = || {
        rustybuzz::ShapePlan::new(
            &face,
            rustybuzz::Direction::LeftToRight,
            Some(rustybuzz::script::LATIN),
            None,
            &[],
        )
    };
    let (plan_new, _, _) = time(budget, plan);
    let plan = plan();
    let buffer = || {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer
    };
    let (shape, _, _) = time(budget, || rustybuzz::shape(&face, &[], buffer()).len());
    let (shape_with_plan, _, _) =
        time(budget, || rustybuzz::shape_with_plan(&face, &plan, buffer()).len());
    println!(
        "shaping {} chars (Lato): plan {plan_new:.1}, shape {shape:.1}, with a plan \
         {shape_with_plan:.1}",
        text.len()
    );
}
