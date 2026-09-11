use avenger_layout::{Layout, Size, SolveOptions};
use avenger_panels::*;
use std::num::NonZeroUsize;

fn tree() -> PanelTree {
    PanelTree::new(
        "figure".into(),
        [
            PanelNode::group(
                "north",
                [
                    PanelNode::panel("a"),
                    PanelNode::panel("b"),
                    PanelNode::panel("c"),
                ],
            ),
            PanelNode::group(
                "south",
                [
                    PanelNode::panel("d"),
                    PanelNode::panel("e"),
                    PanelNode::panel("f"),
                ],
            ),
        ],
    )
    .unwrap()
}
fn arrangement(columns: usize) -> ArrangementSpec {
    ArrangementSpec::new()
        .group("figure", GroupArrangement::Column)
        .group(
            "north",
            GroupArrangement::Wrap(NonZeroUsize::new(columns).unwrap()),
        )
        .group(
            "south",
            GroupArrangement::Wrap(NonZeroUsize::new(columns).unwrap()),
        )
}
fn frames(tree: &PanelTree, panels: &[(&str, Rect)], holes: &[&str]) -> PanelFrames {
    let mut values: Vec<_> = tree
        .nodes()
        .filter(|id| matches!(id, NodeId::Group(_)))
        .map(|id| (id.clone(), Rect::new(0.0, 0.0, 500.0, 500.0)))
        .collect();
    values.extend(
        panels
            .iter()
            .map(|(id, r)| (NodeId::Panel((*id).into()), *r)),
    );
    PanelFrames::new(tree, values, holes.iter().map(|p| (*p).into())).unwrap()
}
fn simple(ids: &[&str]) -> PanelTree {
    PanelTree::new("root".into(), ids.iter().map(|id| PanelNode::panel(*id))).unwrap()
}
fn contributions(ids: &[(&str, Option<&str>)]) -> Vec<GuideContribution> {
    ids.iter()
        .map(|(id, key)| {
            let c = GuideContribution::new((*id).into());
            if let Some(key) = key {
                c.equivalent((*key).into())
            } else {
                c
            }
        })
        .collect()
}
fn labels(side: Side, content: Vec<GuideContribution>) -> GuideRequest {
    AxisLabels::new("labels".into(), side, Scope::Root, content)
        .visibility(LabelVisibility::Outer)
        .into()
}
fn owner<'a>(plan: &'a GuidePlan, panel: &str) -> Option<&'a PanelId> {
    plan.decision(&"labels".into(), &panel.into())
        .and_then(|d| d.instance())
        .map(|id| plan.instance(id).unwrap().source_panel())
}

#[test]
fn scopes_share_one_contract_and_preserve_order() {
    let t = tree();
    let panels: Vec<_> = t.panels().cloned().rev().collect();
    let parent = t
        .group(panels.clone(), Scope::ancestor(1).unwrap())
        .unwrap();
    assert_eq!(
        parent.iter().map(|g| g.members().len()).collect::<Vec<_>>(),
        [3, 3]
    );
    assert_eq!(
        parent.for_panel(&"a".into()).unwrap().members()[0],
        PanelId::from("a")
    );
    assert_eq!(
        t.group(panels.clone(), Scope::Root).unwrap(),
        t.group(panels.clone(), Scope::ancestor(2).unwrap())
            .unwrap()
    );
    assert_eq!(
        t.group(panels.clone(), Scope::Root).unwrap(),
        t.group(panels.clone(), Scope::Group("figure".into()))
            .unwrap()
    );
    assert_eq!(t.group(panels, Scope::Panel).unwrap().iter().count(), 6);
    assert_eq!(t.group(Vec::new(), Scope::Root).unwrap().iter().count(), 0);
    assert!(Scope::ancestor(0).is_err());
    assert!(matches!(
        t.group(["a".into()], Scope::ancestor(3).unwrap()),
        Err(PanelError::InvalidDepth { .. })
    ));
    assert!(matches!(
        t.group(["a".into()], Scope::Group("south".into())),
        Err(PanelError::NotAncestor { .. })
    ));
    assert!(matches!(
        t.group(["a".into(), "a".into()], Scope::Root),
        Err(PanelError::DuplicateParticipant(_))
    ));
    assert!(matches!(
        t.group(["unknown".into()], Scope::Panel),
        Err(PanelError::UnknownNode(_))
    ));
}

#[test]
fn typed_namespaces_allow_equal_text_but_reject_duplicates() {
    assert!(PanelTree::new("a".into(), [PanelNode::panel("a")]).is_ok());
    assert!(matches!(
        PanelTree::new("a".into(), [PanelNode::group("a", [])]),
        Err(PanelError::DuplicateNode(_))
    ));
    assert!(
        PanelTree::new(
            "r".into(),
            [
                PanelNode::panel("a"),
                PanelNode::group("g", [PanelNode::panel("a")])
            ]
        )
        .is_err()
    );
}

#[test]
fn rewrap_and_holes_preserve_logical_groups() {
    let t = tree();
    let a = t.arrange(&arrangement(2)).unwrap();
    let b = t
        .arrange(&arrangement(3).display("c", PanelDisplay::Hole))
        .unwrap();
    assert_eq!(a.tree(), b.tree());
    assert_eq!(
        a.grid(&"north".into()).unwrap().shape(),
        GridShape {
            rows: 2,
            columns: 2
        }
    );
    assert_eq!(
        b.grid(&"north".into()).unwrap().shape(),
        GridShape {
            rows: 1,
            columns: 3
        }
    );
    assert_eq!(b.display(&"c".into()), Some(PanelDisplay::Hole));
    assert_eq!(
        a.tree()
            .group(t.panels().cloned(), Scope::ancestor(1).unwrap())
            .unwrap(),
        b.tree()
            .group(t.panels().cloned(), Scope::ancestor(1).unwrap())
            .unwrap()
    );
}

#[test]
fn explicit_grids_validate_spans_completeness_and_direct_children() {
    let t = simple(&["a", "b"]);
    let a = GridSlot {
        row: 0,
        column: 0,
        row_span: 1,
        column_span: 2,
    };
    let b = GridSlot {
        row: 1,
        column: 0,
        row_span: 1,
        column_span: 1,
    };
    let spec = |slots| {
        ArrangementSpec::new().group(
            "root",
            GroupArrangement::Grid {
                shape: GridShape {
                    rows: 2,
                    columns: 2,
                },
                slots,
            },
        )
    };
    assert!(
        t.arrange(&spec(vec![
            (PanelId::from("b").into(), b),
            (PanelId::from("a").into(), a)
        ]))
        .is_ok()
    );
    for bad in [
        a,
        GridSlot {
            column_span: 0,
            ..b
        },
        GridSlot {
            row: usize::MAX,
            ..b
        },
        GridSlot { column: 2, ..b },
    ] {
        assert!(
            t.arrange(&spec(vec![
                (PanelId::from("a").into(), a),
                (PanelId::from("b").into(), bad)
            ]))
            .is_err()
        );
    }
    assert!(
        t.arrange(&spec(vec![(PanelId::from("a").into(), a)]))
            .is_err()
    );
    assert!(
        t.arrange(&spec(vec![
            (PanelId::from("a").into(), a),
            (PanelId::from("a").into(), b)
        ]))
        .is_err()
    );
    assert!(t.arrange(&ArrangementSpec::new()).is_err());
    assert!(
        tree()
            .arrange(&arrangement(2).group("missing", GroupArrangement::Row))
            .is_err()
    );
    assert!(
        tree()
            .arrange(&arrangement(2).display("missing", PanelDisplay::Hole))
            .is_err()
    );
    let empty = PanelTree::new("r".into(), []).unwrap();
    assert_eq!(
        empty
            .arrange(&ArrangementSpec::new().group("r", GroupArrangement::Row))
            .unwrap()
            .grid(&"r".into())
            .unwrap()
            .shape(),
        GridShape {
            rows: 0,
            columns: 0
        }
    );
}

#[test]
fn layout_adapter_reads_content_and_retains_holes() {
    let t = simple(&["a", "b"]);
    let a = t
        .arrange(
            &ArrangementSpec::new()
                .group("root", GroupArrangement::Row)
                .display("b", PanelDisplay::Hole),
        )
        .unwrap();
    let layout: Layout<NodeId> = Layout::row([
        Layout::leaf(Size::new(100.0, 80.0)).id(PanelId::from("a").into()),
        Layout::leaf(Size::new(100.0, 80.0)).id(PanelId::from("b").into()),
    ])
    .id(GroupId::from("root").into());
    let result = layout.solve(&SolveOptions::default()).unwrap();
    let f = PanelFrames::from_layout(&a, &result).unwrap();
    assert_eq!(f.rect(&PanelId::from("b").into()).unwrap().x, 100.0);
    assert_eq!(f.display(&"b".into()), Some(PanelDisplay::Hole));
    assert_eq!(f.rect(&GroupId::from("root").into()).unwrap().width, 200.0);
}

#[test]
fn frame_validation_rejects_missing_duplicate_and_nonfinite_values() {
    let t = simple(&["a"]);
    let root = NodeId::Group("root".into());
    let p = NodeId::Panel("a".into());
    let good = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(matches!(
        PanelFrames::new(&t, [(root.clone(), good)], []),
        Err(PanelError::MissingFrame(_))
    ));
    assert!(matches!(
        PanelFrames::new(
            &t,
            [(root.clone(), good), (p.clone(), good), (p.clone(), good)],
            []
        ),
        Err(PanelError::DuplicateNode(_))
    ));
    for bad in [
        Rect::new(f32::NAN, 0.0, 1.0, 1.0),
        Rect::new(0.0, 0.0, -1.0, 1.0),
        Rect::new(f32::MAX, 0.0, f32::MAX, 1.0),
    ] {
        assert!(matches!(
            PanelFrames::new(&t, [(root.clone(), good), (p.clone(), bad)], []),
            Err(PanelError::InvalidFrame { .. })
        ));
    }
    assert!(PanelFrames::new(&t, [(root, good), (p, good)], ["unknown".into()]).is_err());
}

#[test]
fn every_sparse_grid_has_one_outer_owner_per_occupied_strip_on_each_side() {
    let ids = ["a", "b", "c", "d", "e", "f"];
    let t = simple(&ids);
    let rects: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            (
                *id,
                Rect::new((i % 3) as f32 * 120.0, (i / 3) as f32 * 120.0, 100.0, 100.0),
            )
        })
        .collect();
    for mask in 0..64 {
        let holes: Vec<_> = ids
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, id)| *id)
            .collect();
        let f = frames(&t, &rects, &holes);
        for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
            let plan = t
                .plan_guides(
                    &f,
                    [labels(
                        side,
                        contributions(&ids.map(|id| (id, Some("same")))),
                    )],
                    GuideOptions::default(),
                )
                .unwrap();
            for (i, id) in ids.iter().enumerate() {
                if holes.contains(id) {
                    assert!(owner(&plan, id).is_none());
                    continue;
                }
                let candidates: Vec<_> = (0..6)
                    .filter(|j| {
                        !holes.contains(&ids[*j])
                            && if matches!(side, Side::Top | Side::Bottom) {
                                j % 3 == i % 3
                            } else {
                                j / 3 == i / 3
                            }
                    })
                    .collect();
                let expected = if matches!(side, Side::Top | Side::Left) {
                    *candidates.first().unwrap()
                } else {
                    *candidates.last().unwrap()
                };
                assert_eq!(
                    owner(&plan, id).unwrap().as_str(),
                    ids[expected],
                    "mask {mask}, {side:?}, {id}"
                );
            }
        }
    }
}

#[test]
fn incompatible_or_missing_contributions_break_runs() {
    let t = simple(&["a", "b", "c"]);
    let f = frames(
        &t,
        &[
            ("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
            ("b", Rect::new(0.0, 120.0, 100.0, 100.0)),
            ("c", Rect::new(0.0, 240.0, 100.0, 100.0)),
        ],
        &[],
    );
    for content in [
        contributions(&[("a", Some("A")), ("b", Some("B")), ("c", Some("A"))]),
        contributions(&[("a", Some("A")), ("c", Some("A"))]),
        contributions(&[("a", Some("A")), ("b", None), ("c", Some("A"))]),
    ] {
        let p = t
            .plan_guides(&f, [labels(Side::Bottom, content)], GuideOptions::default())
            .unwrap();
        assert_eq!(owner(&p, "a").unwrap().as_str(), "a");
        assert_eq!(owner(&p, "c").unwrap().as_str(), "c");
    }
}

#[test]
fn geometry_and_scope_block_unsafe_compaction() {
    let t = PanelTree::new(
        "root".into(),
        [
            PanelNode::group("g", [PanelNode::panel("a"), PanelNode::panel("c")]),
            PanelNode::panel("b"),
        ],
    )
    .unwrap();
    for b in [
        Rect::new(0.0, 120.0, 100.0, 100.0),
        Rect::new(30.0, 120.0, 50.0, 100.0),
        Rect::new(0.0, 80.0, 100.0, 100.0),
    ] {
        let f = frames(
            &t,
            &[
                ("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
                ("b", b),
                ("c", Rect::new(0.0, 240.0, 100.0, 100.0)),
            ],
            &[],
        );
        let req = AxisLabels::new(
            "labels".into(),
            Side::Bottom,
            Scope::Group("g".into()),
            contributions(&[("a", Some("A")), ("c", Some("A"))]),
        )
        .visibility(LabelVisibility::Outer);
        let p = t
            .plan_guides(&f, [req.into()], GuideOptions::default())
            .unwrap();
        assert_eq!(owner(&p, "a").unwrap().as_str(), "a");
    }
}

#[test]
fn alignment_tolerance_is_direct_not_transitive() {
    let t = simple(&["a", "b", "c"]);
    let f = frames(
        &t,
        &[
            ("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
            ("b", Rect::new(0.4, 120.0, 100.0, 100.0)),
            ("c", Rect::new(0.8, 240.0, 100.0, 100.0)),
        ],
        &[],
    );
    let p = t
        .plan_guides(
            &f,
            [labels(
                Side::Bottom,
                contributions(&[("a", Some("A")), ("b", Some("A")), ("c", Some("A"))]),
            )],
            GuideOptions {
                alignment_tolerance: 0.5,
            },
        )
        .unwrap();
    assert_eq!(owner(&p, "a").unwrap().as_str(), "a");
    assert_eq!(owner(&p, "b").unwrap().as_str(), "c");
    for tol in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(
            t.plan_guides(
                &f,
                [],
                GuideOptions {
                    alignment_tolerance: tol
                }
            )
            .is_err()
        );
    }
}

#[test]
fn shared_guide_anchor_and_source_are_independent_and_stable() {
    let t = tree();
    let rects: Vec<_> = t
        .panels()
        .enumerate()
        .map(|(i, p)| (p.as_str(), Rect::new(i as f32 * 110.0, 0.0, 100.0, 100.0)))
        .collect();
    let f = frames(&t, &rects, &[]);
    let c: Vec<_> = t
        .panels()
        .rev()
        .map(|p| GuideContribution::new(p.clone()).equivalent("Revenue".into()))
        .collect();
    let request = SharedGuide::new(
        "title".into(),
        SharedGuideKind::AxisTitle,
        Side::Left,
        Scope::Root,
        c.clone(),
    );
    let p = t
        .plan_guides(&f, [request.into()], GuideOptions::default())
        .unwrap();
    let i = p.instances().next().unwrap();
    assert_eq!(i.source_panel().as_str(), "a");
    assert_eq!(i.anchor(), &NodeId::Group("figure".into()));
    assert_eq!(i.members().len(), 6);
    let changed = frames(&t, &rects, &["a"]);
    let q = t
        .plan_guides(
            &changed,
            [SharedGuide::new(
                "title".into(),
                SharedGuideKind::AxisTitle,
                Side::Right,
                Scope::Root,
                c,
            )
            .into()],
            GuideOptions::default(),
        )
        .unwrap();
    assert_eq!(q.instances().next().unwrap().id(), i.id());
    assert_eq!(q.instances().next().unwrap().source_panel().as_str(), "b");
}

#[test]
fn guide_requests_are_deterministic_and_validate_equivalence() {
    let t = simple(&["a", "b"]);
    let f = frames(
        &t,
        &[
            ("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
            ("b", Rect::new(0.0, 120.0, 100.0, 100.0)),
        ],
        &[],
    );
    let c = contributions(&[("a", Some("A")), ("b", Some("A"))]);
    let a = labels(Side::Bottom, c.clone());
    let b: GuideRequest = SharedGuide::new(
        "legend".into(),
        SharedGuideKind::Legend,
        Side::Right,
        Scope::Root,
        c,
    )
    .into();
    assert_eq!(
        t.plan_guides(&f, [a.clone(), b.clone()], GuideOptions::default())
            .unwrap(),
        t.plan_guides(&f, [b, a.clone()], GuideOptions::default())
            .unwrap()
    );
    assert!(matches!(
        t.plan_guides(&f, [a.clone(), a], GuideOptions::default()),
        Err(PanelError::DuplicateGuide(_))
    ));
    for c in [
        contributions(&[("a", Some("A")), ("b", Some("B"))]),
        contributions(&[("a", None), ("b", None)]),
    ] {
        assert!(matches!(
            t.plan_guides(
                &f,
                [SharedGuide::new(
                    "legend".into(),
                    SharedGuideKind::Legend,
                    Side::Right,
                    Scope::Root,
                    c
                )
                .into()],
                GuideOptions::default()
            ),
            Err(PanelError::IncompatibleGuide { .. })
        ));
    }
    let zero = frames(
        &t,
        &[
            ("a", Rect::new(0.0, 0.0, 0.0, 100.0)),
            ("b", Rect::new(0.0, 120.0, 100.0, 100.0)),
        ],
        &["b"],
    );
    let p = t
        .plan_guides(
            &zero,
            [labels(
                Side::Bottom,
                contributions(&[("a", None), ("b", None)]),
            )],
            GuideOptions::default(),
        )
        .unwrap();
    assert_eq!(p.instances().count(), 0);
    assert_eq!(
        p.decision(&"labels".into(), &"a".into()).unwrap().reason(),
        DecisionReason::ZeroArea
    );
    assert_eq!(
        p.decision(&"labels".into(), &"b".into()).unwrap().reason(),
        DecisionReason::Hole
    );
    let other = simple(&["a"]);
    assert!(matches!(
        other.plan_guides(&f, [], GuideOptions::default()),
        Err(PanelError::MismatchedTree)
    ));
}

#[test]
fn all_none_and_unknown_equivalence_have_explicit_decisions() {
    let t = simple(&["a", "b"]);
    let f = frames(
        &t,
        &[
            ("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
            ("b", Rect::new(0.0, 120.0, 100.0, 100.0)),
        ],
        &[],
    );
    for (visibility, count) in [
        (LabelVisibility::All, 2),
        (LabelVisibility::Outer, 2),
        (LabelVisibility::None, 0),
    ] {
        let p = t
            .plan_guides(
                &f,
                [AxisLabels::new(
                    "labels".into(),
                    Side::Bottom,
                    Scope::Root,
                    contributions(&[("a", None), ("b", None)]),
                )
                .visibility(visibility)
                .into()],
                GuideOptions::default(),
            )
            .unwrap();
        assert_eq!(p.instances().count(), count);
        for id in ["a", "b"] {
            assert!(p.decision(&"labels".into(), &id.into()).is_some());
        }
        assert!(p.decision(&"absent".into(), &"a".into()).is_none());
    }
}

#[test]
fn empty_participation_still_validates_an_explicit_group() {
    let tree = PanelTree::new("root".into(), []).unwrap();
    assert!(matches!(
        tree.group([], Scope::Group("unknown".into())),
        Err(PanelError::UnknownNode(_))
    ));
    assert_eq!(tree.group([], Scope::Root).unwrap().iter().count(), 0);
}
