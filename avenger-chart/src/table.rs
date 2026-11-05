//! The tables of tracks of a chart. avenger-layout places nested rows and columns in one table
//! of tracks, so the plots of a facet grid line up. A concat inside a concat of its direction adds
//! its children to the tracks of its parent, a concat of the other direction spans as many tracks
//! as its widest or tallest child, and every other child spans the tracks of its parent across
//! the parent's direction. Each child of a wrap starts a table of its own.

use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

use crate::{
    proto::{self, node::Node, track, Channel},
    resolve::{ProjectionGroup, ScaleGroup, Tree},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The direction of the tracks of a concat: its columns for a row, and its rows for a column.
pub fn direction(concat: &proto::Concat) -> usize {
    match proto::Direction::try_from(concat.direction) {
        Ok(proto::Direction::Column) => 1,
        _ => 0,
    }
}

/// The name of a direction of tracks in errors.
fn tracks_name(direction: usize) -> &'static str {
    if direction == 0 {
        "column"
    } else {
        "row"
    }
}

/// The count of column tracks and of row tracks that `node` spans in its table.
pub fn extent(node: &proto::Node) -> [usize; 2] {
    match &node.node {
        Some(Node::Concat(concat)) => {
            let along = direction(concat);
            let mut extent = [0, 0];
            for child in &concat.children {
                let child = self::extent(child);
                extent[along] += child[along];
                extent[1 - along] = extent[1 - along].max(child[1 - along]);
            }
            extent
        }
        _ => [1, 1],
    }
}

/// The place of a node in its table: the column tracks and the row tracks that it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The index of its table, in tree order.
    pub table: usize,
    /// The column tracks and the row tracks that the node spans.
    pub tracks: [Range<usize>; 2],
}

/// The tables of a chart: the place of each node, and the size of each track that a concat
/// declares.
pub struct Tables {
    /// The place of each node that is not empty, by its path.
    pub places: HashMap<String, Place>,
    /// The size of each declared track, by its table, its direction, and its index, with the path
    /// of the first concat that declares it.
    declared: HashMap<(usize, usize, usize), (track::Size, String)>,
}

impl Tables {
    /// The size of a track that a concat declares, or `None` for a share of weight 1.
    pub fn declared(&self, table: usize, direction: usize, index: usize) -> Option<&track::Size> {
        self.declared
            .get(&(table, direction, index))
            .map(|(size, _)| size)
    }
}

/// The tables of `chart`. Two concats that declare different sizes for one track fail, with an
/// error that names both concats and the track.
pub fn tables(chart: &Chart) -> Result<Tables> {
    let mut tables = Tables {
        places: HashMap::new(),
        declared: HashMap::new(),
    };
    let root = chart.root();
    let mut count = 0;
    place(
        root,
        "root",
        0,
        [0, 0],
        extent(root),
        &mut count,
        &mut tables,
    )?;
    Ok(tables)
}

fn place(
    node: &proto::Node,
    path: &str,
    table: usize,
    start: [usize; 2],
    span: [usize; 2],
    count: &mut usize,
    tables: &mut Tables,
) -> Result<()> {
    if !matches!(node.node, Some(Node::Empty(_))) {
        tables.places.insert(
            path.to_string(),
            Place {
                table,
                tracks: [start[0]..start[0] + span[0], start[1]..start[1] + span[1]],
            },
        );
    }
    match &node.node {
        Some(Node::Concat(concat)) => {
            let along = direction(concat);
            let mut offset = start[along];
            for (index, child) in concat.children.iter().enumerate() {
                let child_extent = extent(child);
                let mut child_start = start;
                child_start[along] = offset;
                let mut child_span = span;
                child_span[along] = child_extent[along];
                let size = concat
                    .layout
                    .as_ref()
                    .and_then(|layout| layout.tracks.get(index))
                    .and_then(|track| track.size);
                if let Some(size) = size {
                    let key = (table, along, offset);
                    match tables.declared.get(&key) {
                        Some((declared, first)) if *declared != size => {
                            return Err(ChartError::invalid(
                                format!("{path}/layout/tracks[{index}]"),
                                format!(
                                    "the concats `{first}` and `{path}` set different sizes for \
                                     {} {offset} of their table",
                                    tracks_name(along)
                                ),
                            ));
                        }
                        Some(_) => {}
                        None => {
                            tables.declared.insert(key, (size, path.to_string()));
                        }
                    }
                }
                place(
                    child,
                    &format!("{path}/children[{index}]"),
                    table,
                    child_start,
                    child_span,
                    count,
                    tables,
                )?;
                offset += child_extent[along];
            }
        }
        Some(Node::Wrap(wrap)) => {
            for (index, child) in wrap.children.iter().enumerate() {
                *count += 1;
                place(
                    child,
                    &format!("{path}/children[{index}]"),
                    *count,
                    [0, 0],
                    extent(child),
                    count,
                    tables,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The tracks whose lengths the plots of shared position scales match, and the nodes that
/// declare each match.
pub struct Matches {
    /// The matched tracks, by their table, their direction, and their index.
    pub tracks: BTreeSet<(usize, usize, usize)>,
    /// The owners of the scales whose plots match their lengths, with the direction of each.
    pub owners: BTreeSet<(String, usize)>,
}

/// The matches of the plots of each shared `x`, `y`, and projection whose plots stand in more
/// than one track: along the columns for `x`, along the rows for `y`, and along both for a
/// projection. A shared position scale has one length in every plot, and the map plots of a
/// shared projection fit one view, so those plots must stand in one track each, and their tracks
/// must have equal sizes.
pub fn matches(
    tree: &Tree,
    groups: &[ScaleGroup],
    projections: &[ProjectionGroup],
    tables: &Tables,
) -> Result<Matches> {
    let mut matches = Matches {
        tracks: BTreeSet::new(),
        owners: BTreeSet::new(),
    };
    for group in groups {
        let along = match group.channel {
            Channel::X => 0,
            Channel::Y => 1,
            _ => continue,
        };
        let plots: BTreeSet<usize> = group
            .members
            .iter()
            .map(|member| tree.units[member.unit].plot)
            .collect();
        let shared = format!("`{}`", group.channel.as_str_name());
        matches.plots(tree, tables, &plots, &group.owner, along, &shared)?;
    }
    for projection in projections {
        let plots: BTreeSet<usize> = projection.plots.iter().copied().collect();
        for along in [0, 1] {
            matches.plots(
                tree,
                tables,
                &plots,
                &projection.owner,
                along,
                "a projection",
            )?;
        }
    }
    Ok(matches)
}

impl Matches {
    /// Matches the tracks along `along` of `plots`, which share what `shared` names and whose
    /// owner is `owner`, when they stand in more than one track.
    fn plots(
        &mut self,
        tree: &Tree,
        tables: &Tables,
        plots: &BTreeSet<usize>,
        owner: &str,
        along: usize,
        shared: &str,
    ) -> Result<()> {
        let places: Vec<(&str, &Place)> = plots
            .iter()
            .filter_map(|plot| {
                let path = tree.plots[*plot].path.as_str();
                Some((path, tables.places.get(path)?))
            })
            .collect();
        let Some((_, first)) = places.first() else {
            return Ok(());
        };
        if places.iter().all(|(_, place)| {
            place.table == first.table && place.tracks[along] == first.tracks[along]
        }) {
            return Ok(());
        }
        let name = tracks_name(along);
        let mut size: Option<(Option<&track::Size>, String)> = None;
        for (path, place) in &places {
            let tracks = &place.tracks[along];
            if tracks.len() != 1 {
                return Err(ChartError::invalid(
                    owner,
                    format!(
                        "the plot `{path}` spans {} {name}s, and the plots that share {shared} \
                         across {name}s stand in one {name} each, so they can take one length",
                        tracks.len()
                    ),
                ));
            }
            let declared = tables.declared(place.table, along, tracks.start);
            let label = format!("{name} {} of the table of `{path}`", tracks.start);
            match &size {
                Some((first, first_label)) if *first != declared => {
                    return Err(ChartError::invalid(
                        owner,
                        format!(
                            "the plots that share {shared} stand in {first_label} and {label}, \
                             whose sizes differ, so they cannot take one length"
                        ),
                    ));
                }
                Some(_) => {}
                None => size = Some((declared, label)),
            }
            self.tracks.insert((place.table, along, tracks.start));
        }
        self.owners.insert((owner.to_string(), along));
        Ok(())
    }
}
