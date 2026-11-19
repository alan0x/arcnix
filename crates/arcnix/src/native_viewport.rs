//! Physical bounds and occlusion reported by the desktop window manager.
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct BoxPx {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl BoxPx {
    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub(super) fn valid(self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .all(|n| (0..=32768).contains(&n))
            && self.width() > 0
            && self.height() > 0
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Viewport {
    pub epoch: u64,
    pub area: Option<BoxPx>,
    pub clip: Vec<BoxPx>,
    pub command_center: Option<BoxPx>,
}
