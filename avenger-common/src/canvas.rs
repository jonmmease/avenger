#[derive(Debug, Copy, Clone)]
pub struct CanvasDimensions {
    pub size: [f32; 2],
    pub scale: f32,
}

impl CanvasDimensions {
    pub fn to_physical_width(&self) -> u32 {
        (self.size[0] * self.scale) as u32
    }

    pub fn to_physical_height(&self) -> u32 {
        (self.size[1] * self.scale) as u32
    }
}
