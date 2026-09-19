mod common;
use avenger_selection::*;
use common::*;
use datafusion::{common::ScalarValue, logical_expr::col};
use std::ops::Bound;

#[test]
fn updates_route_across_names_atomically_and_clear_only_their_destination() {
    let first = point("first", "x");
    let second_id = SelectionId::new("second").unwrap();
    let second = ProducerDefinition::new(
        second_id.clone(),
        ProducerId::new("second").unwrap(),
        view("second"),
        vec![Projection::new(ProjectionId::new("x").unwrap(), col("x")).unwrap()],
    )
    .unwrap();
    let old = SelectionSet::new([
        (id(), Resolution::Intersect),
        (second_id.clone(), Resolution::Global),
    ])
    .unwrap();
    let value = SelectionValue::tuple([(ProjectionId::new("x").unwrap(), ValueTest::equal(1_i64))]);
    let next = old
        .apply_all([
            SelectionUpdate::set(&first, value.clone()),
            SelectionUpdate::set(&second, value.clone()),
        ])
        .unwrap();
    assert_eq!(old.contributions(&id()).unwrap().count(), 0);
    assert_eq!(old.contributions(&second_id).unwrap().count(), 0);
    assert_eq!(next.resolution(&second_id).unwrap(), Resolution::Global);
    let contribution = next.contributions(&second_id).unwrap().next().unwrap();
    assert_eq!(contribution.producer(), &second);
    assert_eq!(contribution.value(), &value);

    // A later invalid update cannot publish an earlier clear in the same batch.
    assert!(next
        .apply_all([
            SelectionUpdate::clear_all(&id()),
            SelectionUpdate::set(&second, values("wrong", [2_i64.into()])),
        ])
        .is_err());
    assert_eq!(next.contributions(&id()).unwrap().count(), 1);
    assert_eq!(next.contributions(&second_id).unwrap().count(), 1);
    let cleared = next.clear_all(&second_id).unwrap();
    assert_eq!(cleared.contributions(&id()).unwrap().count(), 1);
    assert_eq!(cleared.contributions(&second_id).unwrap().count(), 0);
    let toggled = cleared.toggle(&second, value.clone()).unwrap();
    assert_eq!(toggled.contributions(&second_id).unwrap().count(), 1);
    assert_eq!(
        toggled
            .toggle(&second, value)
            .unwrap()
            .contributions(&second_id)
            .unwrap()
            .count(),
        0
    );

    let missing = SelectionId::new("missing").unwrap();
    assert!(matches!(
        next.contributions(&missing),
        Err(Error::MissingSelection(_))
    ));
    assert!(matches!(
        next.resolution(&missing),
        Err(Error::MissingSelection(_))
    ));
}

#[tokio::test]
async fn value_constructors_preserve_range_endpoints_and_tuple_correlation() {
    let brush = interval("delay_brush", "delay");
    for (test, expected) in [
        (ValueTest::range(10_i64..20_i64), vec![1, 6]),
        (ValueTest::range(10_i64..=20_i64), vec![1, 2, 4, 5, 6]),
        (ValueTest::range(..10_i64), vec![0]),
        (ValueTest::range(..=10_i64), vec![0, 1]),
        (ValueTest::range(20_i64..), vec![2, 3, 4, 5]),
        (ValueTest::range::<i64>(..), vec![0, 1, 2, 3, 4, 5, 6]),
        (
            ValueTest::range((Bound::Excluded(10_i64), Bound::Included(20_i64))),
            vec![2, 4, 5, 6],
        ),
    ] {
        let state = state(Resolution::Intersect)
            .set(
                &brush,
                SelectionValue::tuple([(ProjectionId::new("delay").unwrap(), test)]),
            )
            .unwrap();
        assert_eq!(
            selected(flights(), membership().predicate(&state).unwrap()).await,
            expected
        );
    }

    let brush = producer("correlated", view("brush"), &["delay", "carrier"]);
    let delay = ProjectionId::new("delay").unwrap();
    let carrier = ProjectionId::new("carrier").unwrap();
    let aa = [
        (delay.clone(), ValueTest::range(10_i64..20_i64)),
        (carrier.clone(), ValueTest::equal("AA")),
    ];
    let others = [
        (carrier, ValueTest::one_of(["UA", "DL", "UA"])),
        (delay, ValueTest::range(20_i64..30_i64)),
    ];
    let state = state(Resolution::Intersect)
        .set(&brush, SelectionValue::tuples([aa.clone(), others, aa]))
        .unwrap();
    assert_eq!(
        selected(flights(), membership().predicate(&state).unwrap()).await,
        vec![1, 2, 5]
    );
    let tuples = state
        .contributions(&id())
        .unwrap()
        .next()
        .unwrap()
        .value()
        .as_tuples();
    assert_eq!(tuples.len(), 2);
    assert!(tuples
        .iter()
        .all(|tuple| tuple[0].0.as_str() == "carrier" && tuple[1].0.as_str() == "delay"));
}

#[test]
fn producers_share_names_without_losing_identity_and_snapshots_are_immutable() {
    let brush = interval("delay_brush", "delay");
    let points = producer("delay_points", brush.view().clone(), &["carrier"]);
    for resolution in [Resolution::Intersect, Resolution::Union] {
        let old = state(resolution);
        let next = old
            .apply_all([
                SelectionUpdate::set(&brush, between("delay", 10, 30)),
                SelectionUpdate::set(&points, values("carrier", ["AA".into(), "DL".into()])),
            ])
            .unwrap();
        assert_eq!(old.contributions(&id()).unwrap().count(), 0);
        assert_eq!(next.contributions(&id()).unwrap().count(), 2);
        let cleared = next.clear(&brush).unwrap();
        assert_eq!(cleared.contributions(&id()).unwrap().count(), 1);
        assert_eq!(next.contributions(&id()).unwrap().count(), 2);
        let tuples = cleared
            .contributions(&id())
            .unwrap()
            .next()
            .unwrap()
            .value()
            .as_tuples();
        assert_eq!(tuples.len(), 2);
    }
}
#[test]
fn global_set_toggle_and_clear_preserve_origins_and_projection_meaning() {
    let a = point("a", "carrier");
    let b = ProducerDefinition::new(
        id(),
        ProducerId::new("b").unwrap(),
        view("b"),
        vec![Projection::new(
            ProjectionId::new("renamed").unwrap(),
            col("carrier").alias("alias"),
        )
        .unwrap()],
    )
    .unwrap();
    let unrelated = point("c", "region");
    let mut s = state(Resolution::Global);
    for (p, t) in [
        (&a, tuple("carrier", "AA")),
        (&b, tuple("renamed", "DL")),
        (&unrelated, tuple("region", "AA")),
    ] {
        s = s.toggle(p, SelectionValue::tuple(t)).unwrap();
    }
    assert_eq!(s.contributions(&id()).unwrap().count(), 3);
    s = s
        .toggle(&b, SelectionValue::tuple(tuple("renamed", "AA")))
        .unwrap();
    let origins: Vec<_> = s
        .contributions(&id())
        .unwrap()
        .map(|c| c.producer().id().as_str())
        .collect();
    assert_eq!(origins, vec!["b", "c"]);
    s = s.clear(&b).unwrap();
    assert_eq!(s.contributions(&id()).unwrap().count(), 1);
    s = s.set(&a, SelectionValue::default()).unwrap();
    assert_eq!(s.contributions(&id()).unwrap().count(), 1);
    assert_eq!(
        s.contributions(&id()).unwrap().next().unwrap().value(),
        &SelectionValue::default()
    );
    s = s.clear_all(&id()).unwrap();
    assert_eq!(s.contributions(&id()).unwrap().count(), 0);
}
#[test]
fn canonical_tuples_sets_nans_and_zero_toggle_consistently() {
    let p = point("p", "x");
    let s = state(Resolution::Union)
        .set(
            &p,
            values(
                "x",
                [
                    ScalarValue::Float64(Some(f64::NAN)),
                    ScalarValue::Float64(Some(f64::from_bits(0xfff8000000000042))),
                    0.0.into(),
                    (-0.0).into(),
                    0.0.into(),
                ],
            ),
        )
        .unwrap();
    let tuples = s
        .contributions(&id())
        .unwrap()
        .next()
        .unwrap()
        .value()
        .as_tuples();
    assert_eq!(tuples.len(), 2);
    let empty = s
        .toggle(
            &p,
            SelectionValue::tuples(vec![tuple("x", -0.0), tuple("x", f64::NAN)]),
        )
        .unwrap();
    assert_eq!(empty.contributions(&id()).unwrap().count(), 0);
    let explicit = empty.set(&p, SelectionValue::default()).unwrap();
    assert_eq!(explicit.contributions(&id()).unwrap().count(), 1);
}
#[test]
fn invalid_updates_are_atomic_and_definitions_are_checked() {
    let p = point("p", "x");
    let s = state(Resolution::Intersect);
    let good = SelectionUpdate::set(&p, values("x", [1_i64.into()]));
    let invalid = SelectionUpdate::set(&p, values("wrong", [1_i64.into()]));
    assert!(s.apply_all([good, invalid]).is_err());
    assert_eq!(s.contributions(&id()).unwrap().count(), 0);
    assert!(SelectionSet::new([(id(), Resolution::Intersect), (id(), Resolution::Union)]).is_err());
    let missing = SelectionId::new("missing").unwrap();
    assert!(s.clear_all(&missing).is_err());
    let foreign = ProducerDefinition::new(
        missing,
        p.id().clone(),
        p.view().clone(),
        p.projections().to_vec(),
    )
    .unwrap();
    assert!(s.apply(SelectionUpdate::clear(&foreign)).is_err());
    for terms in [
        vec![],
        vec![
            term("x", ValueTest::Equal(1_i64.into())),
            term("x", ValueTest::Equal(2_i64.into())),
        ],
    ] {
        assert!(s.set(&p, SelectionValue::tuple(terms)).is_err());
    }
    assert!(ProducerDefinition::new(
        p.selection().clone(),
        p.id().clone(),
        p.view().clone(),
        vec![]
    )
    .is_err());
    let proj = Projection::new(ProjectionId::new("x").unwrap(), col("x")).unwrap();
    assert!(ProducerDefinition::new(
        p.selection().clone(),
        p.id().clone(),
        p.view().clone(),
        vec![proj.clone(), proj]
    )
    .is_err());
    assert!(SelectionId::new(" ").is_err());
}
#[test]
fn ranges_and_sets_validate_types_without_converting_values() {
    let p = point("bin", "x");
    let s = state(Resolution::Union);
    for (a, b) in [
        (2_i64.into(), 1_i64.into()),
        (1_i64.into(), 2_i32.into()),
        (ScalarValue::Int64(None), 2_i64.into()),
        (f64::NAN.into(), 2.0.into()),
        (0.0.into(), f64::NAN.into()),
    ] {
        assert!(s
            .set(&p, range("x", Bound::Included(a), Bound::Included(b)))
            .is_err());
    }
    let mixed = SelectionValue::tuple(vec![term(
        "x",
        ValueTest::OneOf(vec![1_i64.into(), 1_i32.into()]),
    )]);
    assert!(s.set(&p, mixed).is_err());
    let value = ScalarValue::TimestampNanosecond(Some(123), Some("UTC".into()));
    let next = s.set(&p, values("x", [value.clone()])).unwrap();
    assert_eq!(
        next.contributions(&id()).unwrap().next().unwrap().value(),
        &values("x", [value])
    );
    let changed = ProducerDefinition::new(
        p.selection().clone(),
        p.id().clone(),
        p.view().clone(),
        vec![Projection::new(ProjectionId::new("x").unwrap(), col("other")).unwrap()],
    )
    .unwrap();
    assert!(next
        .toggle(&changed, SelectionValue::tuple(tuple("x", 2_i64)))
        .is_err());
}
#[test]
fn producer_identity_keeps_selection_name_and_view_instance_separate() {
    let first = producer("brush", view("first-instance"), &["x"]);
    let sibling = producer("brush", view("second-instance"), &["x"]);
    let other_name = SelectionId::new("other-selection").unwrap();
    let other = ProducerDefinition::new(
        other_name.clone(),
        first.id().clone(),
        first.view().clone(),
        first.projections().to_vec(),
    )
    .unwrap();
    let state = SelectionSet::new([
        (id(), Resolution::Union),
        (other_name.clone(), Resolution::Union),
    ])
    .unwrap()
    .apply_all([
        SelectionUpdate::set(&first, values("x", [1_i64.into()])),
        SelectionUpdate::set(&sibling, values("x", [2_i64.into()])),
        SelectionUpdate::set(&other, values("x", [3_i64.into()])),
    ])
    .unwrap();
    assert_eq!(state.contributions(&id()).unwrap().count(), 2);
    assert_eq!(state.contributions(&other_name).unwrap().count(), 1);
    let cleared = state.apply(SelectionUpdate::clear(&first)).unwrap();
    assert_eq!(
        cleared
            .contributions(&id())
            .unwrap()
            .next()
            .unwrap()
            .producer()
            .view(),
        sibling.view()
    );
    assert_eq!(cleared.contributions(&other_name).unwrap().count(), 1);
}

#[test]
fn row_ids_are_ordinary_typed_projection_values_and_support_toggle() {
    let p = point("ids", "id");
    let state = state(Resolution::Union)
        .set(&p, values("id", [2_u64.into(), 1_u64.into(), 2_u64.into()]))
        .unwrap();
    let value = state.contributions(&id()).unwrap().next().unwrap().value();
    assert_eq!(value.as_tuples().len(), 2);
    assert_eq!(value.as_tuples()[0][0].1, ValueTest::equal(1_u64));
    assert_eq!(value.as_tuples()[1][0].1, ValueTest::equal(2_u64));
    let toggled = state
        .toggle(&p, values("id", [2_u64.into(), 3_u64.into()]))
        .unwrap();
    assert_eq!(
        toggled
            .contributions(&id())
            .unwrap()
            .next()
            .unwrap()
            .value(),
        &values("id", [1_u64.into(), 3_u64.into()])
    );
    assert!(state
        .set(
            &p,
            SelectionValue::tuple([(
                ProjectionId::new("id").unwrap(),
                ValueTest::one_of([ScalarValue::UInt64(Some(1)), ScalarValue::Int64(Some(1))])
            )])
        )
        .is_err());
}

#[test]
fn timezone_metadata_survives_tuple_deduplication_and_projection_identity() {
    let utc = ScalarValue::TimestampSecond(Some(0), Some("UTC".into()));
    let local = ScalarValue::TimestampSecond(Some(0), Some("America/New_York".into()));
    let p = point("points", "time");
    let s = state(Resolution::Union)
        .set(&p, values("time", [utc.clone(), local.clone()]))
        .unwrap();
    let tuples = s
        .contributions(&id())
        .unwrap()
        .next()
        .unwrap()
        .value()
        .as_tuples();
    assert_eq!(tuples.len(), 2);

    let make = |name, value| {
        ProducerDefinition::new(
            id(),
            ProducerId::new(name).unwrap(),
            view(name),
            vec![Projection::new(
                ProjectionId::new("time").unwrap(),
                datafusion::logical_expr::lit(value),
            )
            .unwrap()],
        )
        .unwrap()
    };
    let a = make("a", utc.clone());
    let b = make("b", local.clone());
    let s = state(Resolution::Global)
        .toggle(&a, SelectionValue::tuple(tuple("time", utc.clone())))
        .unwrap()
        .toggle(&b, SelectionValue::tuple(tuple("time", utc.clone())))
        .unwrap();
    assert_eq!(s.contributions(&id()).unwrap().count(), 2);
    let changed = make("a", local);
    assert!(s
        .toggle(&changed, SelectionValue::tuple(tuple("time", utc)))
        .is_err());
}
