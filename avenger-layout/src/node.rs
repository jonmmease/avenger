//! The layout tree: leaves, rows, columns, grids, slabs and overlays, and the options for
//! solving it.

use std::fmt;
use std::sync::Arc;

use crate::geometry::{Direction, Edges, Side, Size};

/// A string name that is cheap to clone and compare.
macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(Arc<str>);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(name: &str) -> Self {
                Self(Arc::from(name))
            }
        }

        impl From<String> for $name {
            fn from(name: String) -> Self {
                Self(Arc::from(name))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(
    /// A role label on a track, a gap or a slab, such as `x-labels` or `panel-gap`.
    ///
    /// A tag only names a part. It takes effect where a container declares a rule for it:
    /// [`match_tag`](Grid::match_tag) makes sizes equal, and [`collapse_tag`](Grid::collapse_tag)
    /// and [`merge_tag`](Grid::merge_tag) let one slab stand in for others.
    Tag
);

name_type!(
    /// What a leaf or slab draws, such as a scale domain with its format and font, or a title's
    /// text.
    ///
    /// The layout only compares content keys for equality. Equal keys must mean the same drawing
    /// at the same length, so the caller builds each key from everything that changes how the
    /// node draws, and leaves out the length, which [`Measure`](crate::Measure) receives
    /// separately.
    ///
    /// Nodes with equal keys share their measurement answers. Slabs with a tag and a key also
    /// take part in the [`collapse_tag`](Grid::collapse_tag) and [`merge_tag`](Grid::merge_tag)
    /// rules of the containers around them.
    ContentKey
);

/// What a node does, on one side, with its slabs and its children's overflow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Overflow {
    /// They hang outside the node into the parent's gaps, so the parent reserves room for them.
    #[default]
    Propagate,
    /// They stay inside the node's slot, which grows to hold them.
    Contain,
    /// They are drawn but reserve no space anywhere.
    Flush,
}

/// How a node fills its slot in one direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sizing {
    /// The node takes its fit-content size, as CSS `fit-content` sizes a box: its max-content
    /// size, no smaller than its minimum and no larger than its slot or its maximum. A node
    /// without a max-content size keeps its minimum.
    Fit,
    /// The node grows to its slot, up to its maximum size, as CSS `stretch` does.
    #[default]
    Stretch,
}

/// Where a node sits in its slot when it does not fill it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Position {
    #[default]
    Start,
    Center,
    End,
}

/// A node's size and position in its slot, in one direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Placement {
    pub size: Sizing,
    pub position: Position,
}

impl Placement {
    /// Take the fit-content size, at `position` in the slot. See [`Sizing::Fit`].
    pub fn fit(position: Position) -> Self {
        Self {
            size: Sizing::Fit,
            position,
        }
    }

    /// Stretch to the slot up to the maximum size, at `position` when the maximum stops it
    /// short.
    pub fn stretch(position: Position) -> Self {
        Self {
            size: Sizing::Stretch,
            position,
        }
    }
}

/// How far a slab runs along its side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Extent {
    /// Along the host's content edge, as a facet header runs along its panel.
    #[default]
    Content,
    /// Across the corners too, over the slabs on the neighboring sides, as a title runs over a
    /// chart's axes. Top and bottom slabs take the corners: a full left or right slab stops at
    /// the inner edge of the full top and bottom slabs.
    Full,
}

/// The size of a grid track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackSize {
    /// As large as the largest item in the track, and growing toward the largest max-content
    /// size of its items when there is room, before Fr tracks share what is left, as CSS `auto`
    /// tracks do. Auto tracks also share leftover space when the grid has no Fr tracks and
    /// distributes by stretching.
    Auto,
    /// A length in layout units. Items larger than the track overflow it.
    Px(f32),
    /// An Auto track that grows toward its items' max-content size no further than the limit,
    /// as CSS `fit-content(limit)` does.
    FitContent(f32),
    /// `basis` plus a share, by weight, of the space that the other tracks, the gaps and the Fr
    /// bases leave, as CSS flex items with a `flex-basis` grow: no smaller than its basis or its
    /// largest item, and no larger than `max`. With a zero basis, the track is a CSS `fr` track.
    /// Without a canvas size, the share is large enough for every Fr track's largest
    /// max-content item, so that a container in an Fr column, such as a Wrap, takes its
    /// max-content size.
    Fr {
        weight: f32,
        basis: f32,
        max: Option<f32>,
    },
    /// A row as tall as `column`'s final width divided by `ratio`, where `ratio` is width over
    /// height. Valid only for rows. Items larger than the row overflow it.
    Aspect { column: usize, ratio: f32 },
}

/// A grid column or row: its size and an optional tag.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub size: TrackSize,
    pub tag: Option<Tag>,
}

impl Track {
    pub fn auto() -> Self {
        Self::sized(TrackSize::Auto)
    }

    pub fn px(length: f32) -> Self {
        Self::sized(TrackSize::Px(length))
    }

    pub fn fr(weight: f32) -> Self {
        Self::sized(TrackSize::Fr {
            weight,
            basis: 0.0,
            max: None,
        })
    }

    /// An Auto track whose growth toward its items' max-content size stops at `limit`.
    pub fn fit_content(limit: f32) -> Self {
        Self::sized(TrackSize::FitContent(limit))
    }

    /// An Fr track that grows no larger than `max`.
    pub fn fr_max(weight: f32, max: f32) -> Self {
        Self::sized(TrackSize::Fr {
            weight,
            basis: 0.0,
            max: Some(max),
        })
    }

    /// An Fr track of `basis` plus its share by `weight`. For example, band columns whose
    /// widths are a fixed padding plus a step per band have equal steps when each column is
    /// `fr_basis(bands, padding)`.
    pub fn fr_basis(weight: f32, basis: f32) -> Self {
        Self::sized(TrackSize::Fr {
            weight,
            basis,
            max: None,
        })
    }

    /// A row as tall as `column`'s width divided by `ratio`, where `ratio` is width over height.
    pub fn aspect(column: usize, ratio: f32) -> Self {
        Self::sized(TrackSize::Aspect { column, ratio })
    }

    pub fn sized(size: TrackSize) -> Self {
        Self { size, tag: None }
    }

    pub fn tag(mut self, tag: impl Into<Tag>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

/// The gaps between a grid's tracks in one direction, as CSS `column-gap` and `row-gap` set.
///
/// Each gap is `max(min, trailing + leading + clearance)`, where `trailing` and `leading` are
/// the overflow of the items on either side of it in the same column or row. A grid's gap
/// applies to the boundaries that it is the innermost grid around.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gap {
    pub min: f32,
    /// The room between a neighbor's overflow and the content beside it.
    pub clearance: f32,
    pub tag: Option<Tag>,
}

impl Gap {
    pub fn new(min: f32) -> Self {
        Self {
            min,
            ..Self::default()
        }
    }

    pub fn clearance(mut self, clearance: f32) -> Self {
        self.clearance = clearance;
        self
    }

    pub fn tag(mut self, tag: impl Into<Tag>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

/// Where a grid puts the space left over after its tracks are sized, as CSS
/// `justify-content` and `align-content` do. Fr tracks take the leftover space first under every
/// setting, so only a grid without Fr tracks, or with Fr tracks at their maximums, has space to
/// place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Distribute {
    /// Grow the Auto tracks equally, when the grid has no Fr tracks.
    #[default]
    Stretch,
    Start,
    Center,
    End,
    /// Add the same amount to every gap.
    SpaceBetween,
    /// Add the same amount on both sides of every track, so the space before the first track
    /// and after the last is half of what each gap gains.
    SpaceAround,
    /// Add the same amount to every gap and before the first track and after the last.
    SpaceEvenly,
}

/// The tracks that a grid child occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridSlot {
    pub column: usize,
    pub row: usize,
    pub column_span: usize,
    pub row_span: usize,
}

impl GridSlot {
    /// The cell at `column` and `row`.
    pub fn new(column: usize, row: usize) -> Self {
        Self {
            column,
            row,
            column_span: 1,
            row_span: 1,
        }
    }

    /// The slot spanning `columns` columns and `rows` rows from its first cell.
    pub fn span(mut self, columns: usize, rows: usize) -> Self {
        self.column_span = columns;
        self.row_span = rows;
        self
    }

    /// The first track and the span in `direction`.
    pub(crate) fn along(self, direction: Direction) -> (usize, usize) {
        match direction {
            Direction::Horizontal => (self.column, self.column_span),
            Direction::Vertical => (self.row, self.row_span),
        }
    }
}

/// The settings that every node shares.
#[derive(Clone, Debug)]
pub(crate) struct Common<Id> {
    pub id: Option<Id>,
    /// Slabs per side, by [`Side::index`], innermost first.
    pub slabs: [Vec<Slab<Id>>; 4],
    pub overlays: Vec<Overlay<Id>>,
    /// By [`Side::index`].
    pub overflow: [Overflow; 4],
    /// By [`Direction::index`].
    pub place: [Placement; 2],
    /// The tracks that the node spans along its parent [`Row`] or [`Column`].
    pub span: Option<usize>,
    /// By [`Side::index`].
    pub padding: [f32; 4],
    pub min: Size,
    pub max: Size,
}

impl<Id> Default for Common<Id> {
    fn default() -> Self {
        Self {
            id: None,
            slabs: Default::default(),
            overlays: Vec::new(),
            overflow: [Overflow::Propagate; 4],
            place: [Placement::default(); 2],
            span: None,
            padding: [0.0; 4],
            min: Size::ZERO,
            max: UNBOUNDED,
        }
    }
}

macro_rules! common_methods {
    ($what:literal, $($common:ident).+) => {
        #[doc = concat!(
            "The smallest size of the ", $what, ", as CSS `min-width` and `min-height` set."
        )]
        pub fn min_size(mut self, size: Size) -> Self {
            self.$($common).+.min = size;
            self
        }

        pub fn min_width(mut self, width: f32) -> Self {
            self.$($common).+.min.width = width;
            self
        }

        pub fn min_height(mut self, height: f32) -> Self {
            self.$($common).+.min.height = height;
            self
        }

        #[doc = concat!(
            "The largest size of the ", $what, ", as CSS `max-width` and `max-height` set."
        )]
        pub fn max_size(mut self, size: Size) -> Self {
            self.$($common).+.max = size;
            self
        }

        pub fn max_width(mut self, width: f32) -> Self {
            self.$($common).+.max.width = width;
            self
        }

        pub fn max_height(mut self, height: f32) -> Self {
            self.$($common).+.max.height = height;
            self
        }

        /// The id that the node's region and measure calls carry.
        pub fn id(mut self, id: Id) -> Self {
            self.$($common).+.id = Some(id);
            self
        }

        /// Attaches `slab` to `side`, outside the slabs already there.
        pub fn slab(mut self, side: Side, slab: Slab<Id>) -> Self {
            self.$($common).+.slabs[side.index()].push(slab);
            self
        }

        /// Places `overlay` over the node. Overlays never change sizes.
        pub fn overlay(mut self, overlay: Overlay<Id>) -> Self {
            self.$($common).+.overlays.push(overlay);
            self
        }

        /// What happens to the slabs and children's overflow on `side`.
        pub fn overflow(mut self, side: Side, overflow: Overflow) -> Self {
            self.$($common).+.overflow[side.index()] = overflow;
            self
        }

        /// What happens to the slabs and children's overflow on both sides of `direction`.
        pub fn overflow_direction(self, direction: Direction, overflow: Overflow) -> Self {
            let [leading, trailing] = direction.sides();
            self.overflow(leading, overflow)
                .overflow(trailing, overflow)
        }

        /// How the node fills its slot in `direction`.
        pub fn place(mut self, direction: Direction, placement: Placement) -> Self {
            self.$($common).+.place[direction.index()] = placement;
            self
        }

        /// Space inside the node's slot on each side, between the slot's edge and what the node
        /// keeps inside: its content, and on a contained side its slabs and its children's
        /// overflow. It is reserved whatever the side's overflow policy, and slabs that a side
        /// propagates or flushes hang outside it.
        pub fn padding(mut self, padding: Edges) -> Self {
            self.$($common).+.padding = Side::ALL.map(|side| padding.side(side));
            self
        }

        /// Spans `tracks` tracks along the parent: columns in a [`Row`], rows in a [`Column`].
        /// Only a child of a `Row` or `Column` can span, since a [`Grid`] places its children
        /// by [`GridSlot`].
        pub fn span(mut self, tracks: usize) -> Self {
            self.$($common).+.span = Some(tracks);
            self
        }
    };
}

/// No limit on a size.
const UNBOUNDED: Size = Size::new(f32::INFINITY, f32::INFINITY);

/// A node that the layout does not look inside: a plot, a legend or a block of text.
#[derive(Clone, Debug)]
pub struct Leaf<Id = usize> {
    pub(crate) common: Common<Id>,
    pub(crate) content_key: Option<ContentKey>,
    pub(crate) max_content: Option<Size>,
    pub(crate) aspect: Option<f32>,
}

impl<Id> Default for Leaf<Id> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Id> Leaf<Id> {
    /// A leaf with no minimum and no maximum. A leaf whose size depends on the space it gets is
    /// measured through [`Measure`](crate::Measure), given an id.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            content_key: None,
            max_content: None,
            aspect: None,
        }
    }

    /// What the leaf draws. Leaves with equal keys share their measurement answers. See
    /// [`ContentKey`].
    pub fn content_key(mut self, key: impl Into<ContentKey>) -> Self {
        self.content_key = Some(key.into());
        self
    }

    /// The size the leaf's content takes with no constraint, such as the width of unwrapped
    /// text, as CSS max-content sizes a box. Auto tracks grow toward it when there is room, and
    /// [`Sizing::Fit`] takes it within the leaf's bounds. A measured leaf can answer it through
    /// [`Measure::max_content`](crate::Measure::max_content) instead.
    pub fn max_content(mut self, size: Size) -> Self {
        self.max_content = Some(size);
        self
    }

    /// Keeps the leaf's content at `ratio`, width over height, inside the space it gets.
    pub fn aspect(mut self, ratio: f32) -> Self {
        self.aspect = Some(ratio);
        self
    }

    common_methods!("leaf's content", common);
}

/// A leaf attached to one side of a node, outside its content, such as an axis, a header or a
/// legend.
///
/// Its thickness is its size across the side: its width on the left or right, its height on
/// the top or bottom. Along the side it spans the host's content edge, or the whole box with
/// [`Extent::Full`], and its content sits in that length by its placement, up to its maximum
/// length.
#[derive(Clone, Debug)]
pub struct Slab<Id = usize> {
    pub(crate) id: Option<Id>,
    pub(crate) min: Size,
    pub(crate) max_length: f32,
    pub(crate) place: Placement,
    pub(crate) extent: Extent,
    pub(crate) tag: Option<Tag>,
    pub(crate) content_key: Option<ContentKey>,
    pub(crate) max_content: Option<f32>,
}

impl<Id> Default for Slab<Id> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Id> Slab<Id> {
    /// A slab with no minimum and no maximum. Its minimum is its thickness across its side and
    /// its length along it, and a measured slab's thickness comes from
    /// [`Measure`](crate::Measure), given an id.
    pub fn new() -> Self {
        Self {
            id: None,
            min: Size::ZERO,
            max_length: f32::INFINITY,
            place: Placement::default(),
            extent: Extent::Content,
            tag: None,
            content_key: None,
            max_content: None,
        }
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    /// The slab's smallest thickness across its side and smallest length along it, as a width
    /// and a height. A minimum length raises the host's minimum along the side.
    pub fn min_size(mut self, size: Size) -> Self {
        self.min = size;
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min.width = width;
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.min.height = height;
        self
    }

    /// The largest length of the slab's content along its side. A slab is as thick as its
    /// minimum and its measurement need, so it has no maximum thickness.
    pub fn max_length(mut self, length: f32) -> Self {
        self.max_length = length;
        self
    }

    /// The length the slab's content takes along its side with no constraint, such as a
    /// title's unwrapped text width. With [`Placement::fit`], the slab's content takes it when
    /// there is room. Unlike its minimum length, a max-content length does not widen the host.
    /// See [`Leaf::max_content`].
    pub fn max_content(mut self, length: f32) -> Self {
        self.max_content = Some(length);
        self
    }

    /// How the slab's content fills the slab along its side.
    pub fn place(mut self, placement: Placement) -> Self {
        self.place = placement;
        self
    }

    /// Runs the slab across the corners. See [`Extent::Full`].
    pub fn full(mut self) -> Self {
        self.extent = Extent::Full;
        self
    }

    pub fn tag(mut self, tag: impl Into<Tag>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// What the slab draws, such as a scale's tick labels or a title's text. With a tag, the
    /// slab can stand in for or be represented by others through the containers'
    /// [`collapse_tag`](Grid::collapse_tag) and [`merge_tag`](Grid::merge_tag) rules. See
    /// [`ContentKey`].
    pub fn content_key(mut self, key: impl Into<ContentKey>) -> Self {
        self.content_key = Some(key.into());
        self
    }
}

/// A node placed over another node, at fractions of the host's content.
///
/// Overlays are solved after their host and never change any size.
#[derive(Clone, Debug)]
pub struct Overlay<Id = usize> {
    pub(crate) node: Box<Layout<Id>>,
    /// Left, top, right and bottom, as fractions of the host's content.
    pub(crate) rect: [f32; 4],
}

impl<Id> Overlay<Id> {
    /// An overlay that covers the host's content.
    pub fn new(node: impl Into<Layout<Id>>) -> Self {
        Self {
            node: Box::new(node.into()),
            rect: [0.0, 0.0, 1.0, 1.0],
        }
    }

    /// The part of the host's content that the overlay covers, as fractions from 0 to 1 of its
    /// width and height.
    pub fn rect(mut self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        self.rect = [left, top, right, bottom];
        self
    }
}

macro_rules! rule_methods {
    ($($rules:ident).+) => {
        /// Makes everything with `tag` take the largest size: the tracks with the tag that the
        /// container spans, and the gaps and slabs with the tag anywhere inside it, each in its
        /// direction.
        pub fn match_tag(mut self, tag: impl Into<Tag>) -> Self {
            self.$($rules).+.matches.push(tag.into());
            self
        }

        /// Lets slabs with `tag` inside the container stand in for each other along each lane:
        /// on each side, the outermost slab of a run that shows one content key is drawn, and
        /// the slabs inward of it take no space.
        ///
        /// A lane is the same tracks of one table across the side, such as one column for
        /// bottom slabs, so the lanes of separate tables never meet. An empty cell or a host
        /// without a slab with the tag is passed over. A slab that shows another key, or none,
        /// ends the run, and so does a host that overlaps the lane without matching it, such as
        /// a plot that spans more columns. When containers nest, the outermost one that declares
        /// a rule for a tag sets the runs. Only slabs with a content key take part.
        pub fn collapse_tag(mut self, tag: impl Into<Tag>) -> Self {
            self.$($rules).+.collapses.push(tag.into());
            self
        }

        /// Collapses slabs with `tag` as [`collapse_tag`](Self::collapse_tag) does, and then
        /// replaces the slabs left on a side with one slab on the container, when they all show
        /// one content key and their hosts cover every track of the container along that side.
        ///
        /// The merged slab is innermost on the container's side, runs along the container's
        /// content edge, takes its members' placement, extent, size limits and tag, and takes
        /// the id of the first slab in tree order that it stands for. Merges run from the
        /// innermost declaring container outward, and a merged slab counts as one of the slabs
        /// left for the next container out.
        pub fn merge_tag(mut self, tag: impl Into<Tag>) -> Self {
            self.$($rules).+.merges.push(tag.into());
            self
        }
    };
}

/// A node that places its children in slots on declared column and row tracks.
///
/// A grid's children are separate cells: a [`Row`] or [`Column`] in a grid cell starts its own
/// table.
///
/// A grid's minimum and maximum size bound its content, its tracks and gaps, as a leaf's bound
/// its content, so padding and contained slabs add to them. A minimum that is larger than the
/// tracks need grows them as a larger canvas would, and a grid larger than its maximum keeps its
/// maximum and is placed in its slot by its placement.
#[derive(Clone, Debug)]
pub struct Grid<Id = usize> {
    pub(crate) common: Common<Id>,
    /// Columns, then rows.
    pub(crate) tracks: [Vec<Track>; 2],
    pub(crate) children: Vec<(GridSlot, Layout<Id>)>,
    pub(crate) gaps: [Option<Gap>; 2],
    pub(crate) distribute: [Distribute; 2],
    pub(crate) rules: Rules,
    pub(crate) fit_aspect: bool,
}

/// The tags that a container declares rules for.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rules {
    pub matches: Vec<Tag>,
    pub collapses: Vec<Tag>,
    pub merges: Vec<Tag>,
}

impl<Id> Grid<Id> {
    pub fn new(
        columns: impl IntoIterator<Item = Track>,
        rows: impl IntoIterator<Item = Track>,
    ) -> Self {
        Self {
            common: Common::default(),
            tracks: [columns.into_iter().collect(), rows.into_iter().collect()],
            children: Vec::new(),
            gaps: [None, None],
            distribute: [Distribute::Stretch; 2],
            rules: Rules::default(),
            fit_aspect: false,
        }
    }

    pub fn child(mut self, slot: GridSlot, child: impl Into<Layout<Id>>) -> Self {
        self.children.push((slot, child.into()));
        self
    }

    /// The gaps between the grid's columns.
    pub fn column_gap(mut self, gap: Gap) -> Self {
        self.gaps[Direction::Horizontal.index()] = Some(gap);
        self
    }

    /// The gaps between the grid's rows.
    pub fn row_gap(mut self, gap: Gap) -> Self {
        self.gaps[Direction::Vertical.index()] = Some(gap);
        self
    }

    /// Where the grid puts its leftover space in `direction`.
    pub fn distribute(mut self, direction: Direction, distribute: Distribute) -> Self {
        self.distribute[direction.index()] = distribute;
        self
    }

    rule_methods!(rules);

    /// When the canvas height is fixed and the grid's Aspect rows make the layout too tall, runs
    /// one more width pass with the grid's Fr columns capped so that the rows fit.
    ///
    /// Each grid that fits takes the whole amount by which the layout is too tall, which is exact
    /// for one such grid or for grids side by side. Grids that fit and are stacked vertically
    /// shrink more than they need to.
    pub fn fit_aspect(mut self) -> Self {
        self.fit_aspect = true;
        self
    }

    common_methods!("grid's content, its tracks and gaps", common);
}

/// The settings that [`Row`] and [`Column`] share.
#[derive(Clone, Debug)]
pub(crate) struct Sequence<Id> {
    pub common: Common<Id>,
    pub children: Vec<Layout<Id>>,
    /// Tracks declared one at a time, by index in the sequence's direction.
    pub tracks: Vec<(usize, Track)>,
    /// The track for every index that `tracks` does not declare.
    pub all: Option<Track>,
    pub gap: Option<Gap>,
    /// By [`Direction::index`], `None` where the sequence leaves the default.
    pub distribute: [Option<Distribute>; 2],
    pub share: bool,
    pub rules: Rules,
}

impl<Id> Sequence<Id> {
    fn new(children: impl IntoIterator<Item = impl Into<Layout<Id>>>) -> Self {
        Self {
            common: Common::default(),
            children: children.into_iter().map(Into::into).collect(),
            tracks: Vec::new(),
            all: None,
            gap: None,
            distribute: [None; 2],
            share: true,
            rules: Rules::default(),
        }
    }
}

macro_rules! sequence {
    ($name:ident, $along:literal, $across:literal, $order:literal, $other:literal) => {
        #[doc = concat!("A node that places its children ", $order, ", one or more ", $along, "s each.")]
        ///
        #[doc = concat!("A `", stringify!($name), "` shares its tracks with the [`Row`]s and [`Column`]s around it")]
        /// and inside it by default, so that nested sequences form one table, as the rows of a
        /// facet line up their columns. A table ends at a [`Grid`], [`Wrap`] or [`Flow`] cell,
        #[doc = concat!("at a sequence with [`share(false)`](", stringify!($name), "::share), or at the root.")]
        ///
        #[doc = concat!("- A child `", $other, "` lines up with the `", stringify!($name), "`'s other children, and a child `", stringify!($name), "`")]
        #[doc = concat!("  adds its children to the `", stringify!($name), "`'s ", $along, "s.")]
        /// - A leaf, a [`Grid`], a [`Wrap`], a [`Flow`] or a sequence that does not share is one
        ///   cell, which spans
        #[doc = concat!("  every ", $across, " of the `", stringify!($name), "`. A child `", $other, "` with fewer ", $across, "s than its")]
        ///   neighbors leaves its last cells empty.
        #[doc = concat!("- The `", stringify!($name), "` declares the sizes of its ", $along, "s with [`track`](", stringify!($name), "::track) and")]
        #[doc = concat!("  [`tracks`](", stringify!($name), "::tracks). A ", $along, " that it leaves undeclared takes the size that another")]
        #[doc = concat!("  sequence in the table declares for it, and a ", $along, " that no sequence declares is")]
        ///   `Fr(1)`. Two sequences that declare different sizes for one track fail validation.
        ///
        /// Its minimum and maximum size bound its content, its tracks and gaps, as a [`Grid`]'s
        /// do. In a sequence that joins its parent's table, a minimum spreads over the tracks
        /// that it spans, as a spanning child's does. Only the outermost sequence of a table
        /// can take a maximum, padding, a placement, `distribute` or a contained side.
        #[derive(Clone, Debug)]
        pub struct $name<Id = usize> {
            pub(crate) sequence: Sequence<Id>,
        }

        impl<Id> $name<Id> {
            pub fn new(children: impl IntoIterator<Item = impl Into<Layout<Id>>>) -> Self {
                Self {
                    sequence: Sequence::new(children),
                }
            }

            #[doc = concat!("Declares the size and tag of ", $along, " `index`, counted in tracks rather than children: a")]
            /// spanning child or a nested sequence in the same direction covers several indices.
            pub fn track(mut self, index: usize, track: Track) -> Self {
                self.sequence.tracks.push((index, track));
                self
            }

            #[doc = concat!("Declares the size and tag of every ", $along, " that [`track`](Self::track) does not declare.")]
            pub fn tracks(mut self, track: Track) -> Self {
                self.sequence.all = Some(track);
                self
            }

            #[doc = concat!("The gaps between the `", stringify!($name), "`'s children. A gap uses the settings of the")]
            /// innermost sequence or grid around it.
            pub fn gap(mut self, gap: Gap) -> Self {
                self.sequence.gap = Some(gap);
                self
            }

            /// Where the table puts its leftover space in `direction`. Only the outermost
            /// sequence of a table sizes it, so a sequence that joins its parent's table cannot
            /// set this.
            pub fn distribute(mut self, direction: Direction, distribute: Distribute) -> Self {
                self.sequence.distribute[direction.index()] = Some(distribute);
                self
            }

            /// With `false`, makes each child a separate table in one cell, so the children's
            /// tracks do not line up. Sequences inside a child still share with that child, and
            /// tags that this sequence or one around it matches still reach the children.
            pub fn share(mut self, share: bool) -> Self {
                self.sequence.share = share;
                self
            }

            rule_methods!(sequence.rules);

            common_methods!("sequence's content, its tracks and gaps", sequence.common);
        }

        impl<Id> From<$name<Id>> for Layout<Id> {
            fn from(node: $name<Id>) -> Self {
                Self::$name(node)
            }
        }
    };
}

sequence!(Row, "column", "row", "left to right", "Column");
sequence!(Column, "row", "column", "top to bottom", "Row");

/// A node that places its children row by row in as many equal columns as fit its width, so a
/// facet wrap or a card gallery reflows as the canvas resizes.
///
/// A `Wrap` that chooses `k` columns solves as a [`Column`] of [`Row`]s with `k` children per
/// row would, with equal `Fr(1)` columns as CSS `repeat(auto-fit, minmax(min, 1fr))` gives,
/// except that each column gap takes the largest gap that any two children could need, so that
/// the widths do not depend on the count. Its children are its table's cells, so lanes,
/// collapse and merge work on its columns. A `Wrap` always starts its own table, like a
/// [`Grid`], and its children cannot span.
///
/// - The count is the largest that fits the width the `Wrap` gets, at least 1, and at most the
///   number of children and [`max_columns`](Wrap::max_columns). Each column is at least the
///   larger of [`min_column`](Wrap::min_column) and the widest child's minimum.
/// - A short last row leaves its last cells empty. A kept slab above an empty cell hangs into
///   it: the `Wrap` flushes the bottom of a child whose cell below is empty.
/// - Left and right slabs collapse only when every child shows the same content key, since only
///   then is the first child of each row the same at every count. Every child's slabs are
///   measured, so a resize that changes the count needs no measuring.
/// - A solve without a canvas size puts every child on one row.
///
/// The `Wrap` is row-major only: a wrap that fills columns first would take its count from its
/// height, which makes its width depend on its height.
#[derive(Clone, Debug)]
pub struct Wrap<Id = usize> {
    pub(crate) common: Common<Id>,
    pub(crate) children: Vec<Layout<Id>>,
    pub(crate) min_column: f32,
    pub(crate) max_columns: Option<usize>,
    pub(crate) rows: Track,
    pub(crate) gap: Option<Gap>,
    pub(crate) distribute: Distribute,
    pub(crate) rules: Rules,
}

impl<Id> Wrap<Id> {
    pub fn new(children: impl IntoIterator<Item = impl Into<Layout<Id>>>) -> Self {
        Self {
            common: Common::default(),
            children: children.into_iter().map(Into::into).collect(),
            min_column: 0.0,
            max_columns: None,
            rows: Track::fr(1.0),
            gap: None,
            distribute: Distribute::Stretch,
            rules: Rules::default(),
        }
    }

    /// The smallest column width that the count allows.
    pub fn min_column(mut self, width: f32) -> Self {
        self.min_column = width;
        self
    }

    /// The most columns, however wide the `Wrap` is.
    pub fn max_columns(mut self, columns: usize) -> Self {
        self.max_columns = Some(columns);
        self
    }

    /// The size and tag of every row. Rows are `Fr(1)` unless declared otherwise.
    pub fn rows(mut self, track: Track) -> Self {
        self.rows = track;
        self
    }

    /// The gaps between the `Wrap`'s columns and between its rows.
    pub fn gap(mut self, gap: Gap) -> Self {
        self.gap = Some(gap);
        self
    }

    /// Where the `Wrap` puts the leftover space of its rows, as CSS `align-content` does. Its
    /// columns fill its width, so they have none.
    pub fn distribute(mut self, distribute: Distribute) -> Self {
        self.distribute = distribute;
        self
    }

    rule_methods!(rules);

    common_methods!("wrap's content, its tracks and gaps", common);
}

impl<Id> From<Wrap<Id>> for Layout<Id> {
    fn from(wrap: Wrap<Id>) -> Self {
        Self::Wrap(wrap)
    }
}

/// A node that breaks its children into lines by their max-content widths, or their minimums
/// when they have none, as widget rows, filter chips and horizontal legend entries do.
///
/// Each line is a separate row with its own columns, so nothing aligns across lines, and the
/// children's slabs never collapse or merge. A `Flow` starts its own table, like a [`Grid`],
/// and its children cannot span. A solve without a canvas size puts every child on one line.
#[derive(Clone, Debug)]
pub struct Flow<Id = usize> {
    pub(crate) common: Common<Id>,
    pub(crate) children: Vec<Layout<Id>>,
    pub(crate) rows: Track,
    pub(crate) gap: Option<Gap>,
    pub(crate) distribute: [Distribute; 2],
    pub(crate) matches: Vec<Tag>,
}

impl<Id> Flow<Id> {
    pub fn new(children: impl IntoIterator<Item = impl Into<Layout<Id>>>) -> Self {
        Self {
            common: Common::default(),
            children: children.into_iter().map(Into::into).collect(),
            rows: Track::fr(1.0),
            gap: None,
            distribute: [Distribute::Start, Distribute::Stretch],
            matches: Vec::new(),
        }
    }

    /// The size and tag of every line's row. Rows are `Fr(1)` unless declared otherwise.
    pub fn rows(mut self, track: Track) -> Self {
        self.rows = track;
        self
    }

    /// The gaps between the children of a line and between the lines.
    pub fn gap(mut self, gap: Gap) -> Self {
        self.gap = Some(gap);
        self
    }

    /// Where the `Flow` puts leftover space in `direction`: each line's leftover space for
    /// horizontal, at the start unless set otherwise, and the rows' leftover space for
    /// vertical.
    pub fn distribute(mut self, direction: Direction, distribute: Distribute) -> Self {
        self.distribute[direction.index()] = distribute;
        self
    }

    /// Makes the rows and gaps with `tag` inside the `Flow` take the largest size, as
    /// [`Grid::match_tag`] does.
    pub fn match_tag(mut self, tag: impl Into<Tag>) -> Self {
        self.matches.push(tag.into());
        self
    }

    common_methods!("flow's content, its tracks and gaps", common);
}

impl<Id> From<Flow<Id>> for Layout<Id> {
    fn from(flow: Flow<Id>) -> Self {
        Self::Flow(flow)
    }
}

/// One node of a layout.
#[derive(Clone, Debug)]
pub enum Layout<Id = usize> {
    Leaf(Leaf<Id>),
    Row(Row<Id>),
    Column(Column<Id>),
    Grid(Grid<Id>),
    Wrap(Wrap<Id>),
    Flow(Flow<Id>),
}

impl<Id> From<Leaf<Id>> for Layout<Id> {
    fn from(leaf: Leaf<Id>) -> Self {
        Self::Leaf(leaf)
    }
}

impl<Id> From<Grid<Id>> for Layout<Id> {
    fn from(grid: Grid<Id>) -> Self {
        Self::Grid(grid)
    }
}

impl<Id> Layout<Id> {
    pub(crate) fn common(&self) -> &Common<Id> {
        match self {
            Layout::Leaf(leaf) => &leaf.common,
            Layout::Row(row) => &row.sequence.common,
            Layout::Column(column) => &column.sequence.common,
            Layout::Grid(grid) => &grid.common,
            Layout::Wrap(wrap) => &wrap.common,
            Layout::Flow(flow) => &flow.common,
        }
    }
}

/// The canvas and settings for a solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolveOptions {
    /// The canvas width, or `None` to take the layout's max-content width.
    pub width: Option<f32>,
    /// The canvas height, or `None` to take the layout's max-content height.
    pub height: Option<f32>,
    /// Space between the canvas edges and everything the layout draws, reserved on each side
    /// whatever the root's overflow policies are. The canvas size includes it.
    pub padding: Edges,
    /// Device pixels per layout unit to snap to, or `None` to leave coordinates unsnapped.
    pub pixel_ratio: Option<f32>,
    /// Whether to record why each track and gap has its size, for
    /// [`Solution::report`](crate::Solution::report) and the diagnostic image. Recording changes
    /// no rectangle of the solution.
    pub explain: bool,
    /// The most solves that growing measured widths can start. Must be at least 1.
    pub max_solves: u32,
}

impl SolveOptions {
    /// Options for a canvas of `width` by `height`.
    pub fn sized(width: f32, height: f32) -> Self {
        Self {
            width: Some(width),
            height: Some(height),
            ..Self::default()
        }
    }
}

impl Default for SolveOptions {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            padding: Edges::all(0.0),
            pixel_ratio: None,
            explain: false,
            max_solves: 4,
        }
    }
}
