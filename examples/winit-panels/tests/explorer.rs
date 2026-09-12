use avenger_panels::{GuideKind, NodeId, PanelDisplay, PanelId};
use winit_panels::{scene, state::State};

#[test]
fn outer_x_labels_remain_shared_with_independent_y_domains() {
    let mut state = State::new(avenger_text::default_text_engine());
    state.y_scope = 0;
    state.title_scope = 1;
    for preset in [1, 2] {
        state.preset = preset;
        for (size, columns) in [
            ([1280.0, 900.0], 3),
            ([940.0, 1100.0], 2),
            ([720.0, 1600.0], 1),
        ] {
            state.size = size;
            for outer in [false, true] {
                state.outer = outer;
                let out = scene::build(&state).unwrap();
                assert!(
                    !out.fallback,
                    "preset {preset}, size {size:?}, outer {outer}"
                );
                for product in ["a", "b", "c"] {
                    let north = out
                        .frames
                        .rect(&NodeId::Panel(format!("north-{product}").into()))
                        .unwrap();
                    let south = out
                        .frames
                        .rect(&NodeId::Panel(format!("south-{product}").into()))
                        .unwrap();
                    assert!(
                        (north.x - south.x).abs() < 0.01
                            && (north.width - south.width).abs() < 0.01,
                        "unaligned product {product}: {north:?}, {south:?}, preset {preset}, outer {outer}"
                    );
                }
                assert_eq!(
                    out.plan
                        .instances()
                        .filter(|i| i.key().as_str() == "x-labels")
                        .count(),
                    if outer { columns } else { 6 },
                    "preset {preset}, size {size:?}, outer {outer}"
                );
                assert_eq!(
                    out.plan
                        .instances()
                        .filter(|i| i.key().as_str() == "y-labels")
                        .count(),
                    6
                );
            }
        }
    }
}

#[test]
fn explorer_coordinates_domains_titles_legends_and_reflow_through_public_plans() {
    let mut state = State::new(avenger_text::default_text_engine());
    let base = scene::build(&state).unwrap();
    assert_eq!(base.columns, 3);
    assert!(!base.fallback);
    assert_eq!(
        base.plan
            .instances()
            .filter(|i| i.kind() == GuideKind::Legend)
            .count(),
        2
    );
    assert_eq!(
        base.plan
            .instances()
            .filter(|i| i.key().as_str() == "y-title")
            .count(),
        1
    );
    assert_eq!(
        base.domains[&PanelId::from("north-a")],
        base.domains[&PanelId::from("north-b")]
    );
    assert_ne!(
        base.domains[&PanelId::from("north-a")],
        base.domains[&PanelId::from("south-a")]
    );

    state.size = [940.0, 1000.0];
    state.missing = 2;
    state.overlay = true;
    state.legend_bottom = true;
    let wrapped = scene::build(&state).unwrap();
    assert_eq!(wrapped.columns, 2);
    assert_eq!(wrapped.domains, base.domains);
    assert!(!wrapped.fallback);
    assert_eq!(
        wrapped.frames.display(&"south-c".into()),
        Some(PanelDisplay::Hole)
    );
    assert!(
        wrapped
            .plan
            .instances()
            .all(|i| i.source_panel().as_str() != "south-c")
    );
    let source = wrapped
        .plan
        .instances()
        .find(|i| i.key().as_str() == "legend")
        .unwrap();
    assert!(matches!(source.anchor(), NodeId::Group(_)));

    state.y_scope = 0;
    state.title_scope = 2;
    state.missing = 1;
    let independent = scene::build(&state).unwrap();
    assert_ne!(
        independent.domains[&PanelId::from("north-a")],
        independent.domains[&PanelId::from("north-b")]
    );
    assert_eq!(
        independent
            .plan
            .instances()
            .filter(|i| i.key().as_str() == "y-title")
            .count(),
        1
    );
    assert_eq!(
        independent.frames.display(&"south-c".into()),
        Some(PanelDisplay::Shown)
    );
}

#[test]
fn all_controls_and_unit_format_changes_produce_measured_scenes() {
    let mut state = State::new(avenger_text::default_text_engine());
    for index in [0, 0, 1, 1, 2, 2, 2, 3, 3, 3, 4, 5, 5, 5, 6, 7, 7] {
        state.activate(index);
        let out = scene::build(&state).unwrap();
        assert!(!out.fallback, "control {index}");
        let p = out.frames.rect(&NodeId::Group("figure".into())).unwrap();
        assert!(p.width > 0.0 && p.height > 0.0);
        for instance in out.plan.instances() {
            assert!(out.frames.rect(instance.anchor()).is_some());
            assert!(!instance.members().is_empty());
        }
    }
    state.y_scope = 2;
    let mixed = scene::build(&state).unwrap();
    assert_eq!(
        mixed
            .plan
            .instances()
            .filter(|i| i.key().as_str() == "y-labels")
            .count(),
        6
    );
}

#[test]
fn measured_guides_do_not_overlap_each_other_or_leave_the_canvas() {
    use avenger_geometry::marks::MarkGeometryUtils;
    use avenger_scenegraph::marks::mark::SceneMark;

    let mut state = State::new(avenger_text::default_text_engine());
    for (size, local) in [
        ([1280.0, 900.0], false),
        ([940.0, 1100.0], false),
        ([720.0, 1600.0], true),
        ([720.0, 780.0], true),
    ] {
        state.size = size;
        state.legend_scope = if local { 0 } else { 1 };
        state.title_scope = if local { 0 } else { 2 };
        state.legend_bottom = true;
        state.outer = false;
        let out = scene::build(&state).unwrap();
        let SceneMark::Group(root) = &out.scene.marks[0] else {
            panic!("scene root")
        };
        let bounds = root.bounding_box_with_text_engine(&state.engine);
        assert!(bounds.lower().iter().all(|x| *x >= 0.0));
        assert!(
            bounds.upper()[0] <= size[0] && bounds.upper()[1] <= size[1],
            "scene bounds: {size:?} {bounds:?}"
        );
        let chart = root
            .marks
            .iter()
            .find_map(|m| match m {
                SceneMark::Group(g) if g.name == "panels" || g.name == "needs-room" => Some(g),
                _ => None,
            })
            .unwrap();
        if size[1] == 780.0 {
            assert_eq!(chart.name, "needs-room");
            continue;
        }
        assert_eq!(chart.name, "panels", "{size:?}");
        let guides: Vec<_> = chart
            .marks
            .iter()
            .filter_map(|m| match m {
                SceneMark::Group(g) if g.name.starts_with("guide/") => Some(g),
                _ => None,
            })
            .collect();
        for (index, a) in guides.iter().enumerate() {
            let a_bounds = a.bounding_box_with_text_engine(&state.engine);
            for b in &guides[index + 1..] {
                let b_bounds = b.bounding_box_with_text_engine(&state.engine);
                let separated = (0..2).any(|axis| {
                    a_bounds.upper()[axis] <= b_bounds.lower()[axis]
                        || b_bounds.upper()[axis] <= a_bounds.lower()[axis]
                });
                assert!(
                    separated,
                    "overlapping guides: {} and {} at {size:?}",
                    a.name, b.name
                );
            }
        }
    }
}
