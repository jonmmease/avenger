//! The errors a layout can have, and the checks that run before solving.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use crate::geometry::{Direction, Side, Size};
use crate::lower::{GridKind, GridNode, Node, NodeKind, SequenceNode, Tree, WrapNode};
use crate::node::{ContentKey, Gap, GridSlot, Overflow, Placement, SolveOptions, Tag, TrackSize};
use crate::solution::{PathStep, path_text};

/// Why a layout could not be solved.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    /// A size, track, gap, fraction or option is NaN or infinite where it must be finite.
    NonFiniteInput {
        path: Vec<PathStep>,
        field: &'static str,
    },
    /// A value is outside its allowed range: a negative size, gap or weight, or a ratio or
    /// pixel ratio that is not positive.
    InvalidValue {
        path: Vec<PathStep>,
        field: &'static str,
    },
    /// A node's or slab's maximum is below its minimum.
    MaxBelowMin {
        path: Vec<PathStep>,
        direction: Direction,
    },
    /// A grid child's slot is empty or reaches past the grid's tracks.
    SlotOutOfBounds {
        path: Vec<PathStep>,
        slot: GridSlot,
        columns: usize,
        rows: usize,
    },
    /// Two nodes have the same id.
    DuplicateId { path: Vec<PathStep> },
    /// Two sequences in one table declare different sizes or tags for one track. `track`
    /// counts the table's tracks, and `first` and `second` are the two sequences.
    SharedTracksDiffer {
        direction: Direction,
        track: usize,
        first: Vec<PathStep>,
        second: Vec<PathStep>,
    },
    /// A Row or Column declares a track that it does not have.
    TrackOutOfRange { path: Vec<PathStep>, track: usize },
    /// A node with a span is not a child of a Row or Column.
    SpanOutsideSequence { path: Vec<PathStep> },
    /// A sequence that joins its parent's table sets what only the table's outermost sequence
    /// can: `distribute`, `Contain` on a side, padding, a maximum or a placement.
    SharedSequenceSetting {
        path: Vec<PathStep>,
        setting: &'static str,
    },
    /// An Aspect track is a column, refers to a column that does not exist, has a ratio that
    /// is not positive, or is declared by a Row, Column or Wrap, which cannot name a column.
    InvalidAspect { path: Vec<PathStep>, row: usize },
    /// Tracks in one match group are declared with different sizes.
    MatchedTracksDiffer {
        path: Vec<PathStep>,
        tag: Tag,
        direction: Direction,
    },
    /// Two slabs that a container collapses or merges, with one tag and content key on one
    /// side, differ in their size limits, max-content size, placement or extent, so one cannot
    /// stand in for the other.
    RepresentativesDiffer {
        tag: Tag,
        content_key: ContentKey,
        first: Vec<PathStep>,
        second: Vec<PathStep>,
    },
    /// A host has two slabs on one side with a tag that a container around it collapses or
    /// merges, so the runs along its lane are ambiguous.
    RepeatedTag {
        path: Vec<PathStep>,
        side: Side,
        tag: Tag,
    },
    /// An overlay's fractions are outside 0 to 1, or its left is not before its right or its
    /// top before its bottom.
    InvalidOverlayRect { path: Vec<PathStep> },
    /// A measurement's size or overhang is NaN, infinite or negative, its range is not a range,
    /// or it gives an overhang for a leaf that is not a slab.
    InvalidMeasurement { path: Vec<PathStep> },
    /// A measurement's range does not contain the value that it was asked at.
    MeasurementOutsideRange { path: Vec<PathStep>, value: f32 },
    /// Finite inputs produced coordinates beyond the range of `f32`.
    CoordinateOverflow,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteInput { path, field } => {
                write!(f, "{}: {field} must be finite", path_text(path))
            }
            Self::InvalidValue { path, field } => {
                write!(f, "{}: {field} is out of range", path_text(path))
            }
            Self::MaxBelowMin { path, direction } => write!(
                f,
                "{}: the {direction:?} maximum is below the minimum",
                path_text(path)
            ),
            Self::SlotOutOfBounds {
                path,
                slot,
                columns,
                rows,
            } => write!(
                f,
                "{}: slot {slot:?} does not fit a grid of {columns} columns and {rows} rows",
                path_text(path)
            ),
            Self::DuplicateId { path } => {
                write!(
                    f,
                    "{}: the id is already used by another node",
                    path_text(path)
                )
            }
            Self::SharedTracksDiffer {
                direction,
                track,
                first,
                second,
            } => write!(
                f,
                "{} and {} declare {direction:?} track {track} of their table differently; \
                 declare it once, or set share(false) on the sequence around them",
                path_text(first),
                path_text(second)
            ),
            Self::TrackOutOfRange { path, track } => write!(
                f,
                "{}: track {track} is past the sequence's tracks",
                path_text(path)
            ),
            Self::SpanOutsideSequence { path } => write!(
                f,
                "{}: only a child of a Row or Column can span; a Grid places its children by \
                 slot",
                path_text(path)
            ),
            Self::SharedSequenceSetting { path, setting } => write!(
                f,
                "{}: a sequence that joins its parent's table cannot set {setting}; set \
                 share(false) on the parent to give it its own table",
                path_text(path)
            ),
            Self::InvalidAspect { path, row } => write!(
                f,
                "{}: Aspect track {row} must be a Grid row with a positive ratio that refers to \
                 an existing column",
                path_text(path)
            ),
            Self::MatchedTracksDiffer {
                path,
                tag,
                direction,
            } => write!(
                f,
                "{}: the {direction:?} tracks matched by tag {tag} have different sizes",
                path_text(path)
            ),
            Self::RepresentativesDiffer {
                tag,
                content_key,
                first,
                second,
            } => write!(
                f,
                "{} and {} show {content_key} with tag {tag} but differ in their size limits, \
                 max-content size, placement or extent",
                path_text(first),
                path_text(second)
            ),
            Self::RepeatedTag { path, side, tag } => write!(
                f,
                "{}: two {side:?} slabs have tag {tag}, which a container around them collapses \
                 or merges",
                path_text(path)
            ),
            Self::InvalidOverlayRect { path } => write!(
                f,
                "{}: an overlay's fractions must lie from 0 to 1, left before right and top \
                 before bottom",
                path_text(path)
            ),
            Self::InvalidMeasurement { path } => write!(
                f,
                "{}: a measurement must have a finite, non-negative size and overhang and a \
                 range from low to high, and only a slab has an overhang",
                path_text(path)
            ),
            Self::MeasurementOutsideRange { path, value } => write!(
                f,
                "{}: the measurement's range does not contain {value}, where it was asked",
                path_text(path)
            ),
            Self::CoordinateOverflow => {
                write!(f, "layout coordinates exceed the finite range of f32")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// Checks the options and every node of `tree`.
pub(crate) fn validate<Id: Eq + Hash>(
    tree: &Tree<Id>,
    options: &SolveOptions,
) -> Result<(), LayoutError> {
    validate_options(options)?;
    if let Some(oversized) = &tree.oversized {
        return Err(oversized.clone());
    }
    let mut ids = HashSet::new();
    for node in &tree.nodes {
        let path = &node.path[..];
        if let Some(id) = &node.id
            && !ids.insert(id)
        {
            return Err(LayoutError::DuplicateId {
                path: path.to_vec(),
            });
        }
        match &node.kind {
            NodeKind::Leaf { aspect } => {
                check_size(path, node.min, node.max)?;
                check_max_content(path, node.max_content)?;
                if let Some(ratio) = aspect {
                    positive(path, "aspect ratio", *ratio)?;
                }
            }
            NodeKind::Slab { .. } => {
                check_size(path, node.min, node.max)?;
                check_max_content(path, node.max_content)?;
            }
            NodeKind::Grid(grid) => {
                check_size(path, node.min, node.max)?;
                match &grid.kind {
                    GridKind::Sequence(sequence) => check_sequence(grid, sequence, node, path)?,
                    GridKind::Wrap(wrap) => check_wrap(grid, wrap, path)?,
                    GridKind::Grid { .. } => check_grid(tree, grid, path)?,
                }
            }
        }
        for padding in node.padding {
            non_negative(path, "padding", padding)?;
        }
        if let Some(span) = node.span {
            if node
                .parent
                .is_none_or(|parent| tree.sequence(parent).is_none())
            {
                return Err(LayoutError::SpanOutsideSequence {
                    path: path.to_vec(),
                });
            }
            if span == 0 {
                return Err(LayoutError::InvalidValue {
                    path: path.to_vec(),
                    field: "span",
                });
            }
        }
    }
    if let Some(conflict) = &tree.conflict {
        return Err(conflict.clone());
    }
    check_shared_slabs(tree)?;
    for overlay in &tree.overlays {
        let [left, top, right, bottom] = overlay.rect;
        let path = &tree.nodes[overlay.root].path;
        if overlay.rect.iter().any(|value| !value.is_finite()) {
            return Err(LayoutError::NonFiniteInput {
                path: path.clone(),
                field: "overlay fraction",
            });
        }
        let in_unit = |value: f32| (0.0..=1.0).contains(&value);
        if !overlay.rect.into_iter().all(in_unit) || left >= right || top >= bottom {
            return Err(LayoutError::InvalidOverlayRect { path: path.clone() });
        }
    }
    Ok(())
}

/// Checks the canvas size and pixel ratio.
pub(crate) fn validate_options(options: &SolveOptions) -> Result<(), LayoutError> {
    let root: &[PathStep] = &[];
    for (field, value) in [
        ("canvas width", options.width),
        ("canvas height", options.height),
    ] {
        if let Some(value) = value {
            non_negative(root, field, value)?;
        }
    }
    for side in Side::ALL {
        non_negative(root, "canvas padding", options.padding.side(side))?;
    }
    if let Some(ratio) = options.pixel_ratio {
        positive(root, "pixel ratio", ratio)?;
    }
    if options.max_solves == 0 {
        return Err(LayoutError::InvalidValue {
            path: root.to_vec(),
            field: "maximum solves",
        });
    }
    Ok(())
}

fn check_grid<Id>(tree: &Tree<Id>, grid: &GridNode, path: &[PathStep]) -> Result<(), LayoutError> {
    for direction in Direction::BOTH {
        let d = direction.index();
        if let Some(gap) = &grid.gaps[d] {
            check_gap(gap, path)?;
        }
        for (index, track) in grid.tracks[d].iter().enumerate() {
            check_track_size(track.size, path)?;
            if let TrackSize::Aspect { column, ratio } = track.size
                && (direction != Direction::Vertical
                    || column >= grid.tracks[0].len()
                    || ratio <= 0.0)
            {
                return Err(LayoutError::InvalidAspect {
                    path: path.to_vec(),
                    row: index,
                });
            }
        }
    }
    check_matches(grid, path)?;
    let columns = grid.tracks[0].len();
    let rows = grid.tracks[1].len();
    for &child in &grid.children {
        let slot = tree.nodes[child].slot.expect("grid children have slots");
        let fits = |start: usize, span: usize, count: usize| {
            span > 0 && start.checked_add(span).is_some_and(|end| end <= count)
        };
        if !fits(slot.column, slot.column_span, columns) || !fits(slot.row, slot.row_span, rows) {
            return Err(LayoutError::SlotOutOfBounds {
                path: tree.nodes[child].path.clone(),
                slot,
                columns,
                rows,
            });
        }
    }
    Ok(())
}

fn check_sequence<Id>(
    grid: &GridNode,
    sequence: &SequenceNode,
    node: &Node<Id>,
    path: &[PathStep],
) -> Result<(), LayoutError> {
    if let Some(gap) = &grid.gaps[sequence.direction.index()] {
        check_gap(gap, path)?;
    }
    let count = grid.tracks[sequence.direction.index()].len();
    let declared = sequence
        .tracks
        .iter()
        .map(|(index, track)| (Some(*index), track))
        .chain(sequence.all.iter().map(|track| (None, track)));
    for (index, track) in declared {
        check_track_size(track.size, path)?;
        if matches!(track.size, TrackSize::Aspect { .. }) {
            return Err(LayoutError::InvalidAspect {
                path: path.to_vec(),
                row: index.unwrap_or(0),
            });
        }
        if let Some(index) = index
            && index >= count
        {
            return Err(LayoutError::TrackOutOfRange {
                path: path.to_vec(),
                track: index,
            });
        }
    }
    if sequence.joins {
        let setting = if sequence.distribute.iter().any(Option::is_some) {
            Some("distribute")
        } else if node.overflow.contains(&Overflow::Contain) {
            Some("Contain")
        } else if node.padding.iter().any(|&padding| padding != 0.0) {
            Some("padding")
        } else if node.max.width.is_finite() || node.max.height.is_finite() {
            Some("a maximum")
        } else if node.place != [Placement::default(); 2] {
            Some("a placement")
        } else {
            None
        };
        if let Some(setting) = setting {
            return Err(LayoutError::SharedSequenceSetting {
                path: path.to_vec(),
                setting,
            });
        }
    }
    check_matches(grid, path)
}

/// The slabs that a container collapses or merges: one slab per tag on each side of a host,
/// and slabs with one tag and key on one side declared alike.
fn check_shared_slabs<Id>(tree: &Tree<Id>) -> Result<(), LayoutError> {
    for (container, node) in tree.nodes.iter().enumerate() {
        let NodeKind::Grid(grid) = &node.kind else {
            continue;
        };
        for tag in grid.rules.collapses.iter().chain(&grid.rules.merges) {
            let mut first_of: Vec<(Side, &ContentKey, usize)> = Vec::new();
            for (host, host_node) in tree.nodes.iter().enumerate() {
                if host == container || !tree.is_within(host, container) {
                    continue;
                }
                for side in Side::ALL {
                    let tagged: Vec<usize> = host_node.slabs[side.index()]
                        .iter()
                        .copied()
                        .filter(|&slab| tree.slab_tag(slab) == Some(tag))
                        .collect();
                    if tagged.len() > 1 {
                        return Err(LayoutError::RepeatedTag {
                            path: host_node.path.clone(),
                            side,
                            tag: tag.clone(),
                        });
                    }
                    let Some(&slab) = tagged.first() else {
                        continue;
                    };
                    let Some(key) = &tree.nodes[slab].content_key else {
                        continue;
                    };
                    match first_of.iter().find(|(s, k, _)| *s == side && *k == key) {
                        Some(&(_, _, first)) if !alike(tree, first, slab) => {
                            return Err(LayoutError::RepresentativesDiffer {
                                tag: tag.clone(),
                                content_key: key.clone(),
                                first: tree.nodes[first].path.clone(),
                                second: tree.nodes[slab].path.clone(),
                            });
                        }
                        Some(_) => {}
                        None => first_of.push((side, key, slab)),
                    }
                }
            }
        }
    }
    Ok(())
}

/// Whether two slabs have the same size limits, max-content size, placement and extent.
fn alike<Id>(tree: &Tree<Id>, a: usize, b: usize) -> bool {
    let (a, b) = (&tree.nodes[a], &tree.nodes[b]);
    let extent = |node: &Node<Id>| match node.kind {
        NodeKind::Slab { extent, .. } => Some(extent),
        _ => None,
    };
    a.min == b.min
        && a.max == b.max
        && a.max_content == b.max_content
        && a.place == b.place
        && extent(a) == extent(b)
}

fn check_wrap(grid: &GridNode, wrap: &WrapNode, path: &[PathStep]) -> Result<(), LayoutError> {
    if let Some(gap) = &grid.gaps[0] {
        check_gap(gap, path)?;
    }
    non_negative(path, "minimum column width", wrap.min_column)?;
    if wrap.max_columns == Some(0) {
        return Err(LayoutError::InvalidValue {
            path: path.to_vec(),
            field: "maximum columns",
        });
    }
    check_track_size(wrap.rows.size, path)?;
    if matches!(wrap.rows.size, TrackSize::Aspect { .. }) {
        return Err(LayoutError::InvalidAspect {
            path: path.to_vec(),
            row: 0,
        });
    }
    check_matches(grid, path)
}

/// Tracks that one grid matches must be declared alike.
fn check_matches(grid: &GridNode, path: &[PathStep]) -> Result<(), LayoutError> {
    for direction in Direction::BOTH {
        for tag in &grid.rules.matches {
            let mut sizes = grid.tracks[direction.index()]
                .iter()
                .filter(|track| track.tag.as_ref() == Some(tag))
                .map(|track| track.size);
            if let Some(first) = sizes.next()
                && sizes.any(|size| size != first)
            {
                return Err(LayoutError::MatchedTracksDiffer {
                    path: path.to_vec(),
                    tag: tag.clone(),
                    direction,
                });
            }
        }
    }
    Ok(())
}

fn check_gap(gap: &Gap, path: &[PathStep]) -> Result<(), LayoutError> {
    non_negative(path, "minimum gap", gap.min)?;
    non_negative(path, "gap clearance", gap.clearance)
}

fn check_track_size(size: TrackSize, path: &[PathStep]) -> Result<(), LayoutError> {
    match size {
        TrackSize::Auto => Ok(()),
        TrackSize::Px(length) => non_negative(path, "px track size", length),
        TrackSize::FitContent(limit) => non_negative(path, "fit-content limit", limit),
        TrackSize::Fr { weight, basis, max } => {
            non_negative(path, "fr weight", weight)?;
            non_negative(path, "fr basis", basis)?;
            match max {
                Some(max) => non_negative(path, "fr maximum", max),
                None => Ok(()),
            }
        }
        TrackSize::Aspect { ratio, .. } if !ratio.is_finite() => Err(LayoutError::NonFiniteInput {
            path: path.to_vec(),
            field: "aspect ratio",
        }),
        TrackSize::Aspect { .. } => Ok(()),
    }
}

fn check_max_content(path: &[PathStep], size: Option<Size>) -> Result<(), LayoutError> {
    if let Some(size) = size {
        non_negative(path, "max-content width", size.width)?;
        non_negative(path, "max-content height", size.height)?;
    }
    Ok(())
}

fn check_size(path: &[PathStep], min: Size, max: Size) -> Result<(), LayoutError> {
    for direction in Direction::BOTH {
        let low = min.along(direction);
        let high = max.along(direction);
        non_negative(path, "minimum size", low)?;
        if high.is_nan() {
            return Err(LayoutError::NonFiniteInput {
                path: path.to_vec(),
                field: "maximum size",
            });
        }
        if high < low {
            return Err(LayoutError::MaxBelowMin {
                path: path.to_vec(),
                direction,
            });
        }
    }
    Ok(())
}

fn non_negative(path: &[PathStep], field: &'static str, value: f32) -> Result<(), LayoutError> {
    if !value.is_finite() {
        return Err(LayoutError::NonFiniteInput {
            path: path.to_vec(),
            field,
        });
    }
    if value < 0.0 {
        return Err(LayoutError::InvalidValue {
            path: path.to_vec(),
            field,
        });
    }
    Ok(())
}

fn positive(path: &[PathStep], field: &'static str, value: f32) -> Result<(), LayoutError> {
    non_negative(path, field, value)?;
    if value <= 0.0 {
        return Err(LayoutError::InvalidValue {
            path: path.to_vec(),
            field,
        });
    }
    Ok(())
}
