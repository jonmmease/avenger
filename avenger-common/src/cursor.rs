#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CursorStyle {
    #[default]
    Default,
    Pointer,
    Text,
    Crosshair,
    Grab,
    Grabbing,
    ResizeHorizontal,
    ResizeVertical,
    ResizeNwSe,
    ResizeNeSw,
}

impl CursorStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Pointer => "pointer",
            Self::Text => "text",
            Self::Crosshair => "crosshair",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::ResizeHorizontal => "resize_horizontal",
            Self::ResizeVertical => "resize_vertical",
            Self::ResizeNwSe => "resize_nw_se",
            Self::ResizeNeSw => "resize_ne_sw",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "default" => Some(Self::Default),
            "pointer" => Some(Self::Pointer),
            "text" => Some(Self::Text),
            "crosshair" => Some(Self::Crosshair),
            "grab" => Some(Self::Grab),
            "grabbing" => Some(Self::Grabbing),
            "resize_horizontal" => Some(Self::ResizeHorizontal),
            "resize_vertical" => Some(Self::ResizeVertical),
            "resize_nw_se" => Some(Self::ResizeNwSe),
            "resize_ne_sw" => Some(Self::ResizeNeSw),
            _ => None,
        }
    }
}
