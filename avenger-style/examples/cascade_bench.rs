//! Times the cascade on a chart-sized style tree: six facet plots with three layers and two
//! labeled axes each, and a legend, under the base stylesheet. The argument is the number of
//! rounds:
//!
//! ```text
//! cargo run --profile release-perf -p avenger-style --example cascade_bench -- 300
//! ```

use std::time::Instant;

use avenger_style::*;

fn tree() -> StyleTree {
    let mut tree = StyleTree::new(Element::Chart);
    let root = tree.root();
    let facet = tree.add(root, Element::Plot);
    tree.set_attribute(facet, "type", "facet_wrap");
    for _ in 0..6 {
        let chart = tree.add(facet, Element::Chart);
        let plot = tree.add(chart, Element::Plot);
        tree.set_attribute(plot, "type", "cartesian");
        for kind in ["bar", "line", "symbol"] {
            let mark = tree.add(plot, Element::Mark);
            tree.set_attribute_forms(mark, "type", kind, kind);
            tree.add(mark, Element::Label);
        }
        for channel in ["x", "y"] {
            let axis = tree.add(plot, Element::Axis);
            tree.set_attribute(axis, "channel", channel);
            tree.set_attribute(axis, "scale", "linear");
            tree.set_attribute(axis, "kind", "sequential");
            tree.add(axis, Element::Title);
            for _ in 0..10 {
                tree.add(axis, Element::Label);
                tree.add(axis, Element::Tick);
            }
            tree.add(axis, Element::Rule);
            tree.add(axis, Element::Grid);
        }
        let scale = tree.add(plot, Element::Scale);
        tree.set_attribute(scale, "channel", "color");
        tree.set_attribute(scale, "kind", "categorical");
    }
    let legend = tree.add(facet, Element::Legend);
    for _ in 0..5 {
        tree.add(legend, Element::Swatch);
        tree.add(legend, Element::Label);
    }
    tree
}

fn main() {
    let rounds: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(50);
    let tree = tree();
    let mut none = |url: &str, _: Option<&str>| Err(url.to_string());
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let css = "@import 'std:base.css'; @container (width < 400px) { axis { label-angle: 45; } }";
    let (sheet, diagnostics) = Sheet::load(css, None, SourceId(1), options, &mut none);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let table = StyleTable::new().with_author(sheet, Env::new());
    let cascade = Cascade::new(&table);
    let host = HostInputs::sized(800.0, 600.0);
    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let start = Instant::now();
        std::hint::black_box(cascade.compute(&tree, &host));
        times.push(start.elapsed().as_secs_f64() * 1e3);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{} nodes, compute median {:.3} ms, min {:.3} ms",
        tree.len(),
        times[rounds / 2],
        times[0]
    );
}
