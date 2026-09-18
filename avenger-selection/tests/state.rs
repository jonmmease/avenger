mod common;
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::datatypes::{DataType, TimeUnit},
    common::ScalarValue,
    logical_expr::col,
};
use std::ops::Bound;

#[test]
fn producers_share_names_without_losing_identity_and_snapshots_are_immutable() {
    let brush = interval("delay_brush", "delay");
    let points = producer(
        "delay_points",
        brush.address().origin.clone(),
        SelectionKind::Point,
        &["carrier"],
    );
    for resolution in [Resolution::Intersect, Resolution::Union] {
        let old = state(resolution);
        let next = old
            .apply_all([
                (id(), SelectionUpdate::set(&brush, between("delay", 10, 30))),
                (
                    id(),
                    SelectionUpdate::set(&points, values("carrier", ["AA".into(), "DL".into()])),
                ),
            ])
            .unwrap();
        assert_eq!(old.get(&id()).unwrap().contributions().count(), 0);
        assert_eq!(next.get(&id()).unwrap().contributions().count(), 2);
        let cleared = next
            .apply(&id(), SelectionUpdate::clear(brush.address()))
            .unwrap();
        assert_eq!(cleared.get(&id()).unwrap().contributions().count(), 1);
        assert_eq!(next.get(&id()).unwrap().contributions().count(), 2);
        let SelectionValue::Tuples(tuples) = cleared
            .get(&id())
            .unwrap()
            .contributions()
            .next()
            .unwrap()
            .value()
        else {
            panic!()
        };
        assert_eq!(tuples.len(), 2);
    }
}
#[test]
fn global_set_toggle_and_clear_preserve_origins_and_projection_meaning() {
    let a = point("a", "carrier");
    let b = ProducerDefinition::new(
        address("b", view("b")),
        SelectionKind::Point,
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
        s = s.apply(&id(), SelectionUpdate::toggle(p, vec![t])).unwrap();
    }
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 3);
    s = s
        .apply(
            &id(),
            SelectionUpdate::toggle(&b, vec![tuple("renamed", "AA")]),
        )
        .unwrap();
    let origins: Vec<_> = s
        .get(&id())
        .unwrap()
        .contributions()
        .map(|c| c.producer().address().producer.as_str())
        .collect();
    assert_eq!(origins, vec!["b", "c"]);
    s = s.apply(&id(), SelectionUpdate::clear(b.address())).unwrap();
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 1);
    s = s
        .apply(
            &id(),
            SelectionUpdate::set(&a, SelectionValue::Tuples(vec![])),
        )
        .unwrap();
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 1);
    assert_eq!(
        s.get(&id())
            .unwrap()
            .contributions()
            .next()
            .unwrap()
            .value(),
        &SelectionValue::Tuples(vec![])
    );
    s = s.apply(&id(), SelectionUpdate::clear_all()).unwrap();
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 0);
}
#[test]
fn canonical_tuples_sets_nans_and_zero_toggle_consistently() {
    let p = point("p", "x");
    let s = state(Resolution::Union)
        .apply(
            &id(),
            SelectionUpdate::set(
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
            ),
        )
        .unwrap();
    let SelectionValue::Tuples(tuples) = s
        .get(&id())
        .unwrap()
        .contributions()
        .next()
        .unwrap()
        .value()
    else {
        panic!()
    };
    assert_eq!(tuples.len(), 2);
    let empty = s
        .apply(
            &id(),
            SelectionUpdate::toggle(&p, vec![tuple("x", -0.0), tuple("x", f64::NAN)]),
        )
        .unwrap();
    assert_eq!(empty.get(&id()).unwrap().contributions().count(), 0);
    let explicit = empty
        .apply(
            &id(),
            SelectionUpdate::set(&p, SelectionValue::Tuples(vec![])),
        )
        .unwrap();
    assert_eq!(explicit.get(&id()).unwrap().contributions().count(), 1);
}
#[test]
fn invalid_updates_are_atomic_and_definitions_are_checked() {
    let p = point("p", "x");
    let s = state(Resolution::Intersect);
    let good = SelectionUpdate::set(&p, values("x", [1_i64.into()]));
    let invalid = SelectionUpdate::set(&p, values("wrong", [1_i64.into()]));
    assert!(s.apply_all([(id(), good), (id(), invalid)]).is_err());
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 0);
    let snapshot = s.get(&id()).unwrap().clone();
    assert!(SelectionSet::new([snapshot.clone(), snapshot]).is_err());
    let missing = SelectionId::new("missing").unwrap();
    assert!(s.apply(&missing, SelectionUpdate::clear_all()).is_err());
    let mut foreign = p.address().clone();
    foreign.selection = missing;
    assert!(s.apply(&id(), SelectionUpdate::clear(&foreign)).is_err());
    for terms in [
        vec![],
        vec![
            term("x", ValueTest::Equal(1_i64.into())),
            term("x", ValueTest::Equal(2_i64.into())),
        ],
    ] {
        assert!(s
            .apply(
                &id(),
                SelectionUpdate::set(&p, SelectionValue::Tuples(vec![SelectionTuple { terms }]))
            )
            .is_err());
    }
    assert!(ProducerDefinition::new(p.address().clone(), SelectionKind::Point, vec![]).is_err());
    let proj = Projection::new(ProjectionId::new("x").unwrap(), col("x")).unwrap();
    assert!(ProducerDefinition::new(
        p.address().clone(),
        SelectionKind::Point,
        vec![proj.clone(), proj]
    )
    .is_err());
    assert!(SelectionId::new(" ").is_err());
    assert!(s
        .apply(
            &id(),
            SelectionUpdate::toggle(&interval("i", "x"), vec![tuple("x", 1_i64)])
        )
        .is_err());
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
            .apply(
                &id(),
                SelectionUpdate::set(&p, range("x", Bound::Included(a), Bound::Included(b)))
            )
            .is_err());
    }
    let mixed = SelectionValue::Tuples(vec![SelectionTuple {
        terms: vec![term(
            "x",
            ValueTest::OneOf(vec![1_i64.into(), 1_i32.into()]),
        )],
    }]);
    assert!(s.apply(&id(), SelectionUpdate::set(&p, mixed)).is_err());
    let value = ScalarValue::TimestampNanosecond(Some(123), Some("UTC".into()));
    let next = s
        .apply(
            &id(),
            SelectionUpdate::set(&p, values("x", [value.clone()])),
        )
        .unwrap();
    assert_eq!(
        next.get(&id())
            .unwrap()
            .contributions()
            .next()
            .unwrap()
            .value(),
        &values("x", [value])
    );
    let changed = ProducerDefinition::new(
        p.address().clone(),
        SelectionKind::Point,
        vec![Projection::new(ProjectionId::new("x").unwrap(), col("other")).unwrap()],
    )
    .unwrap();
    assert!(next
        .apply(
            &id(),
            SelectionUpdate::toggle(&changed, vec![tuple("x", 2_i64)])
        )
        .is_err());
}
#[test]
fn nested_composite_addresses_keep_types_nulls_zones_and_order() {
    let scope = ScopeId::new("facets").unwrap();
    let key = |v| FacetKey::new(scope.clone(), v).unwrap();
    let a = key(vec!["East".into(), ScalarValue::Int64(None)]);
    assert_eq!(a, key(vec!["East".into(), ScalarValue::Int64(None)]));
    assert_ne!(a, key(vec!["East".into(), ScalarValue::Int32(None)]));
    assert_ne!(a, key(vec![ScalarValue::Int64(None), "East".into()]));
    let utc = key(vec![ScalarValue::TimestampSecond(
        Some(0),
        Some("UTC".into()),
    )]);
    let other = key(vec![ScalarValue::TimestampSecond(
        Some(0),
        Some("America/New_York".into()),
    )]);
    assert_ne!(utc, other);
    assert!(!utc.cmp(&other).is_eq());
    assert!(FacetKey::new(scope, vec![1.0.into()]).is_err());
    let mut nested = view("hist");
    nested.scope = vec![a, utc];
    let parent = ViewAddress {
        view: nested.view.clone(),
        scope: nested.scope[..1].to_vec(),
    };
    assert_ne!(nested, parent);
    let ids = RowIdentity::new(DataType::Timestamp(
        TimeUnit::Nanosecond,
        Some("UTC".into()),
    ))
    .unwrap();
    let unrelated = RowIdentity::new(ids.data_type().clone()).unwrap();
    assert_ne!(ids, unrelated);
    assert_eq!(ids, ids.clone());
}
#[test]
fn row_ids_are_typed_and_require_the_same_lineage() {
    let identity = RowIdentity::new(DataType::UInt64).unwrap();
    let p = ProducerDefinition::row_ids(address("ids", view("rows")), identity.clone());
    let ids =
        RowIdSelection::new(&identity, vec![2_u64.into(), 1_u64.into(), 2_u64.into()]).unwrap();
    assert_eq!(ids.values(), &[1_u64.into(), 2_u64.into()]);
    assert!(RowIdSelection::new(&identity, vec![1_i64.into()]).is_err());
    let s = state(Resolution::Union)
        .apply(&id(), SelectionUpdate::set(&p, SelectionValue::RowIds(ids)))
        .unwrap();
    let foreign = RowIdentity::new(DataType::UInt64).unwrap();
    assert!(s
        .apply(
            &id(),
            SelectionUpdate::set(
                &p,
                SelectionValue::RowIds(RowIdSelection::new(&foreign, vec![]).unwrap())
            )
        )
        .is_err());
    assert!(s
        .apply(
            &id(),
            SelectionUpdate::set(&p, SelectionValue::Tuples(vec![]))
        )
        .is_err());
}

#[test]
fn timezone_metadata_survives_tuple_deduplication_and_projection_identity() {
    let utc = ScalarValue::TimestampSecond(Some(0), Some("UTC".into()));
    let local = ScalarValue::TimestampSecond(Some(0), Some("America/New_York".into()));
    let p = point("points", "time");
    let s = state(Resolution::Union)
        .apply(
            &id(),
            SelectionUpdate::set(&p, values("time", [utc.clone(), local.clone()])),
        )
        .unwrap();
    let SelectionValue::Tuples(tuples) = s
        .get(&id())
        .unwrap()
        .contributions()
        .next()
        .unwrap()
        .value()
    else {
        panic!()
    };
    assert_eq!(tuples.len(), 2);

    let make = |name, value| {
        ProducerDefinition::new(
            address(name, view(name)),
            SelectionKind::Point,
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
        .apply(
            &id(),
            SelectionUpdate::toggle(&a, vec![tuple("time", utc.clone())]),
        )
        .unwrap()
        .apply(
            &id(),
            SelectionUpdate::toggle(&b, vec![tuple("time", utc.clone())]),
        )
        .unwrap();
    assert_eq!(s.get(&id()).unwrap().contributions().count(), 2);
    let changed = make("a", local);
    assert!(s
        .apply(
            &id(),
            SelectionUpdate::toggle(&changed, vec![tuple("time", utc)])
        )
        .is_err());
}
