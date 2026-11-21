//! Physical bounds and occlusion reported by the desktop window manager.
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoxPx {
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

    pub(crate) fn intersect(self, other: Self) -> Option<Self> {
        let overlap = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (overlap.width() > 0 && overlap.height() > 0).then_some(overlap)
    }

    fn valid(self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .all(|n| (-32768..=32768).contains(&n))
            && self.width() > 0
            && self.height() > 0
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Viewport {
    pub epoch: u64,
    pub area: Option<BoxPx>,
    pub frame: Option<BoxPx>,
    pub clip: Vec<BoxPx>,
    pub command_center: Option<BoxPx>,
    pub menu_slot: Option<BoxPx>,
    pub title_colors: Option<[String; 2]>,
}

impl Viewport {
    pub(crate) fn valid(&self) -> bool {
        self.area.is_none_or(BoxPx::valid)
            && self.frame.is_none_or(BoxPx::valid)
            && self.command_center.is_none_or(BoxPx::valid)
            && self.menu_slot.is_none_or(BoxPx::valid)
            && self.clip.len() <= 32
            && self.clip.iter().all(|area| area.valid())
    }
}
