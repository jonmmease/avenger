//! Plain geometric value types.

/// Two-dimensional size.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Axis-aligned rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// One side of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// Per-side values around a rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Edges<T = f32> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T> Edges<T> {
    pub fn new(top: T, right: T, bottom: T, left: T) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn side(&self, side: Side) -> &T {
        match side {
            Side::Top => &self.top,
            Side::Right => &self.right,
            Side::Bottom => &self.bottom,
            Side::Left => &self.left,
        }
    }

    pub fn set_side(&mut self, side: Side, value: T) {
        match side {
            Side::Top => self.top = value,
            Side::Right => self.right = value,
            Side::Bottom => self.bottom = value,
            Side::Left => self.left = value,
        }
    }
}

impl Edges<f32> {
    /// Component-wise maximum.
    pub fn max(self, other: Self) -> Self {
        Self {
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
            left: self.left.max(other.left),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_side_accessors_follow_side_enum() {
        let mut edges = Edges::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(*edges.side(Side::Left), 4.0);
        edges.set_side(Side::Left, 5.0);
        assert_eq!(edges.left, 5.0);
        assert_eq!(
            edges.max(Edges::new(0.0, 9.0, 0.0, 0.0)),
            Edges::new(1.0, 9.0, 3.0, 5.0)
        );
    }
}
