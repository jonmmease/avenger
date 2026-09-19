mod common;
use avenger_selection::*;
use common::*;
use datafusion::{
    arrow::{array::Float32Array, datatypes::DataType},
    common::ScalarValue,
    logical_expr::col,
};
use std::sync::Arc;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

async fn check(filter: &ConsumerFilter, state: &SelectionSet, focus: &ProducerDefinition) {
    let predicates = filter.predicates(state, focus).unwrap();
    assert_eq!(predicates.full(), &filter.predicate(state).unwrap());
    let split = predicates.split().unwrap();
    assert_eq!(
        selected(flights(), predicates.full().clone()).await,
        selected(
            flights(),
            split.fixed().clone().and(split.changing().clone())
        )
        .await
    );
}

#[tokio::test]
async fn independent_producers_preserve_fixed_predicates_and_correlated_tuples() -> TestResult {
    let focus = producer(
        "brush",
        view("focus"),
        SelectionKind::Interval,
        &["delay", "region"],
    );
    let other = point("airlines", "carrier");
    let filter = cross(view("target"));
    let inactive = state(Resolution::Intersect).set(&other, values("carrier", ["AA".into()]))?;
    let first = filter.predicates(&inactive, &focus)?;
    assert_eq!(
        first.split().unwrap().dimensions(),
        &[col("delay"), col("region")]
    );
    for (low, high) in [(0, 15), (10, 30), (20, 40)] {
        let state = inactive.set(
            &focus,
            SelectionValue::Tuples(vec![
                vec![
                    term("delay", ValueTest::Equal(low.into())),
                    term("region", ValueTest::Equal("East".into())),
                ],
                vec![
                    term("delay", ValueTest::Equal(high.into())),
                    term("region", ValueTest::Equal("West".into())),
                ],
            ]),
        )?;
        check(&filter, &state, &focus).await;
        let next = filter.predicates(&state, &focus)?;
        assert_eq!(
            first.split().unwrap().fixed(),
            next.split().unwrap().fixed()
        );
        assert_eq!(
            first.split().unwrap().dimensions(),
            next.split().unwrap().dimensions()
        );
    }
    Ok(())
}

#[tokio::test]
async fn inactivity_empty_values_and_exclusions_keep_their_distinct_meanings() -> TestResult {
    let focus = interval("focus", "delay");
    let other = point("other", "carrier");
    let inactive = state(Resolution::Intersect);
    for empty in [EmptySelection::MatchAll, EmptySelection::MatchNone] {
        for mode in [SelectionMode::Membership, SelectionMode::CrossFilter] {
            let filter = ConsumerFilter::new(
                other.address().origin.clone(),
                SelectionFilter::Selection {
                    id: id(),
                    usage: SelectionUse { mode, empty },
                },
            );
            for state in [
                inactive.clone(),
                inactive.set(&other, values("carrier", []))?,
                inactive.set(&focus, values("delay", []))?,
                inactive.set(&focus, between("delay", 10, 30))?,
            ] {
                check(&filter, &state, &focus).await;
            }
            let p = filter.predicates(&inactive, &focus)?;
            assert_eq!(p.split().unwrap().dimensions(), &[col("delay")]);
            assert_eq!(
                selected(flights(), p.split().unwrap().fixed().clone())
                    .await
                    .len(),
                7
            );
            assert_eq!(
                selected(flights(), p.full().clone()).await.len(),
                if empty == EmptySelection::MatchAll {
                    7
                } else {
                    0
                }
            );
        }
    }
    assert_eq!(
        cross(focus.address().origin.clone())
            .predicates(&inactive, &focus)?
            .split()
            .unwrap_err(),
        SplitReason::FocusNotUsed
    );
    Ok(())
}

#[tokio::test]
async fn facet_addresses_exclude_only_the_exact_view_instance() -> TestResult {
    let scope = ScopeId::new("region")?;
    let east = ViewAddress {
        view: ViewId::new("hist")?,
        scope: vec![FacetKey::new(scope.clone(), vec!["East".into()])?],
    };
    let west = ViewAddress {
        view: ViewId::new("hist")?,
        scope: vec![FacetKey::new(scope, vec!["West".into()])?],
    };
    let focus = producer("brush", east.clone(), SelectionKind::Interval, &["delay"]);
    let state = state(Resolution::Intersect).set(&focus, between("delay", 10, 30))?;
    assert_eq!(
        cross(east).predicates(&state, &focus)?.split().unwrap_err(),
        SplitReason::FocusNotUsed
    );
    check(&cross(west), &state, &focus).await;
    Ok(())
}

#[tokio::test]
async fn mapped_pixel_dimensions_cover_inactive_brushes_and_reject_changed_definitions(
) -> TestResult {
    let original = interval("focus", "x");
    let grid = |size| {
        PixelGrid::new(
            avenger_scales_datafusion::BuiltinScale::Linear,
            Arc::new(Float32Array::from(vec![0., 40.])),
            Arc::new(Float32Array::from(vec![40., 0.])),
            Default::default(),
            0.,
            size,
        )
    };
    let pixel = original.with_pixel_grids([(ProjectionId::new("x")?, grid(10.)?)])?;
    let filter = ConsumerFilter::new(view("target"), SelectionFilter::cross_filter([&id()]))
        .with_projection(original.address(), &ProjectionId::new("x")?, col("delay"))?;

    let inactive = state(Resolution::Intersect);
    let initial = filter.predicates(&inactive, &pixel)?;
    assert_eq!(
        initial.split().unwrap().dimensions(),
        &[pixel
            .pixel_grid(&ProjectionId::new("x")?)
            .unwrap()
            .cell_expr(col("delay"))]
    );
    let selected_state = inactive.set(&pixel, between("x", 0, 30))?;
    check(&filter, &selected_state, &pixel).await;
    assert_eq!(
        initial.split().unwrap().dimensions(),
        filter
            .predicates(&selected_state, &pixel)?
            .split()
            .unwrap()
            .dimensions()
    );
    let changed = pixel.with_pixel_grids([(ProjectionId::new("x")?, grid(5.)?)])?;
    let changed_state = selected_state.set(&changed, between("x", 0, 30))?;
    let stale = filter.predicates(&changed_state, &pixel)?;
    assert_eq!(stale.split().unwrap_err(), SplitReason::IncompatibleFocus);
    assert_eq!(stale.full(), &filter.predicate(&changed_state)?);
    check(&filter, &changed_state, &changed).await;
    Ok(())
}

#[tokio::test]
async fn unsupported_composition_retains_full_membership_and_invalid_names_are_errors() -> TestResult
{
    let focus = interval("focus", "delay");
    for resolution in [Resolution::Union, Resolution::Global] {
        let state = state(resolution).set(&focus, between("delay", 10, 30))?;
        let p = membership().predicates(&state, &focus)?;
        assert_eq!(p.split().unwrap_err(), SplitReason::UnsupportedComposition);
        assert_eq!(
            selected(flights(), p.full().clone()).await,
            vec![1, 2, 4, 5, 6]
        );
    }
    let leaf = SelectionFilter::membership(&id(), EmptySelection::MatchAll);
    for tree in [
        SelectionFilter::Any(vec![leaf.clone()]),
        SelectionFilter::Not(Box::new(leaf)),
    ] {
        let filter = ConsumerFilter::new(view("target"), tree);
        assert_eq!(
            filter
                .predicates(&state(Resolution::Intersect), &focus)?
                .split()
                .unwrap_err(),
            SplitReason::UnsupportedComposition
        );
    }
    let missing =
        SelectionFilter::membership(&SelectionId::new("missing")?, EmptySelection::MatchAll);
    let filter = ConsumerFilter::new(
        view("target"),
        SelectionFilter::Any(vec![SelectionFilter::All(vec![]), missing]),
    );
    assert!(matches!(
        filter.predicates(&state(Resolution::Intersect), &focus),
        Err(Error::MissingSelection(_))
    ));
    let foreign = producer("foreign", view("foreign"), SelectionKind::Point, &["id"]);
    assert!(membership()
        .predicates(&SelectionSet::new([])?, &foreign)
        .is_err());
    Ok(())
}

#[test]
fn row_identity_uses_direct_predicates() -> TestResult {
    let identity = RowIdentity::new(DataType::Int64)?;
    let focus = ProducerDefinition::row_ids(address("ids", view("source")), identity.clone());
    let state = state(Resolution::Intersect).set(
        &focus,
        SelectionValue::RowIds(RowIdSelection::new(
            &identity,
            vec![ScalarValue::Int64(Some(1))],
        )?),
    )?;
    let filter = ConsumerFilter::new(view("target"), SelectionFilter::cross_filter([&id()]))
        .with_row_identity(&identity, col("id"))?;

    assert_eq!(
        filter.predicates(&state, &focus)?.split().unwrap_err(),
        SplitReason::UnsupportedInteraction
    );
    Ok(())
}

#[tokio::test]
async fn nonfocused_boolean_branches_remain_whole_fixed_predicates() -> TestResult {
    let focus = interval("focus", "delay");
    let other_id = SelectionId::new("categories")?;
    let other = ProducerDefinition::new(
        ProducerAddress {
            selection: other_id.clone(),
            producer: ProducerId::new("airline")?,
            origin: view("other"),
        },
        SelectionKind::Point,
        vec![Projection::new(
            ProjectionId::new("carrier")?,
            col("carrier"),
        )?],
    )?;
    let state = SelectionSet::new([
        (id(), Resolution::Intersect),
        (other_id.clone(), Resolution::Union),
    ])?
    .apply_all([
        SelectionUpdate::set(&focus, between("delay", 10, 30)),
        SelectionUpdate::set(&other, values("carrier", ["AA".into()])),
    ])?;
    let filter = ConsumerFilter::new(
        view("target"),
        SelectionFilter::All(vec![
            SelectionFilter::membership(&id(), EmptySelection::MatchAll),
            SelectionFilter::Not(Box::new(SelectionFilter::Any(vec![
                SelectionFilter::membership(&other_id, EmptySelection::MatchNone),
            ]))),
        ]),
    );
    check(&filter, &state, &focus).await;
    assert_eq!(
        selected(flights(), filter.predicates(&state, &focus)?.full().clone()).await,
        vec![2, 5, 6]
    );
    Ok(())
}
