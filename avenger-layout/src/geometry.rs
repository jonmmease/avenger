//! Sizes, rectangles, sides and directions.

/// A width and a height.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// A zero size.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// The size along `direction`: the width for horizontal, the height for vertical.
    pub fn along(self, direction: Direction) -> f32 {
        match direction {
            Direction::Horizontal => self.width,
            Direction::Vertical => self.height,
        }
    }
}

/// An axis-aligned rectangle, from its top-left corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The x coordinate of the right edge.
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The y coordinate of the bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether `point` lies inside the rectangle or on its edge.
    pub fn contains(&self, point: [f32; 2]) -> bool {
        point[0] >= self.x
            && point[0] <= self.right()
            && point[1] >= self.y
            && point[1] <= self.bottom()
    }

    /// The rectangle with the horizontal range `x` and the vertical range `y`.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn from_ranges(x: (f32, f32), y: (f32, f32)) -> Self {
        Self::new(x.0, y.0, x.1 - x.0, y.1 - y.0)
    }

    /// The rectangle with the horizontal range `x` and the vertical range `y` of the solver,
    /// which works in `f64`. Its ends keep its length exactly, so a length that the solver chose,
    /// such as a max-content length, comes back exactly, and the edges of neighbors can differ
    /// in the last bit instead.
    pub(crate) fn from_solver_ranges(x: (f64, f64), y: (f64, f64)) -> Self {
        Self::new(
            x.0 as f32,
            y.0 as f32,
            (x.1 - x.0) as f32,
            (y.1 - y.0) as f32,
        )
    }
}

/// One side of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Top,
    Right,
    Bottom,
}

impl Side {
    /// The four sides, in the order that [`Edges`] lists them.
    pub const ALL: [Side; 4] = [Side::Left, Side::Top, Side::Right, Side::Bottom];

    /// The direction a slab on this side is thick in: horizontal for the left and right sides,
    /// vertical for the top and bottom.
    pub fn direction(self) -> Direction {
        match self {
            Side::Left | Side::Right => Direction::Horizontal,
            Side::Top | Side::Bottom => Direction::Vertical,
        }
    }

    /// Whether the side comes first in its direction: the left or the top.
    pub fn is_leading(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Top => 1,
            Side::Right => 2,
            Side::Bottom => 3,
        }
    }
}

/// A length for each side of a rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Edges {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// The same length on every side.
    pub fn all(length: f32) -> Self {
        Self::new(length, length, length, length)
    }

    pub fn side(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.left,
            Side::Top => self.top,
            Side::Right => self.right,
            Side::Bottom => self.bottom,
        }
    }
}

/// A direction of the layout. Widths are solved in the horizontal direction and heights in the
/// vertical one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Horizontal,
    Vertical,
}

impl Direction {
    /// Both directions, widths first.
    pub const BOTH: [Direction; 2] = [Direction::Horizontal, Direction::Vertical];

    /// The other direction.
    pub fn cross(self) -> Direction {
        match self {
            Direction::Horizontal => Direction::Vertical,
            Direction::Vertical => Direction::Horizontal,
        }
    }

    /// The leading and trailing sides: left and right, or top and bottom.
    pub fn sides(self) -> [Side; 2] {
        match self {
            Direction::Horizontal => [Side::Left, Side::Right],
            Direction::Vertical => [Side::Top, Side::Bottom],
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Direction::Horizontal => 0,
            Direction::Vertical => 1,
        }
    }
}
