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
    /// Visible, disjoint rectangles in the host's physical coordinates.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn visible_rects(&self) -> Vec<BoxPx> {
        let mut visible: Vec<_> = self
            .area
            .zip(self.frame)
            .and_then(|(area, frame)| area.intersect(frame))
            .into_iter()
            .collect();
        for &cover in &self.clip {
            let mut next = Vec::new();
            for rect in visible {
                if let Some(cut) = rect.intersect(cover) {
                    next.extend(
                        [
                            BoxPx {
                                bottom: cut.top,
                                ..rect
                            },
                            BoxPx {
                                top: cut.bottom,
                                ..rect
                            },
                            BoxPx {
                                right: cut.left,
                                top: cut.top,
                                bottom: cut.bottom,
                                ..rect
                            },
                            BoxPx {
                                left: cut.right,
                                top: cut.top,
                                bottom: cut.bottom,
                                ..rect
                            },
                        ]
                        .into_iter()
                        .filter(|r| r.width() > 0 && r.height() > 0),
                    );
                } else {
                    next.push(rect);
                }
            }
            visible = next;
        }
        visible
    }

    pub(crate) fn valid(&self) -> bool {
        self.area.is_none_or(BoxPx::valid)
            && self.frame.is_none_or(BoxPx::valid)
            && self.command_center.is_none_or(BoxPx::valid)
            && self.menu_slot.is_none_or(BoxPx::valid)
            && self.clip.len() <= 32
            && self.clip.iter().all(|area| area.valid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_surface_matches_frame_minus_overlapping_windows_and_titlebar_gaps() {
        let rect = |left, top, right, bottom| BoxPx {
            left,
            top,
            right,
            bottom,
        };
        let area = rect(-2, 1, 17, 14);
        let frame = rect(0, 0, 15, 12);
        let viewport = Viewport {
            epoch: 0,
            area: Some(area),
            frame: Some(frame),
            clip: vec![
                rect(4, 1, 7, 4),
                rect(10, 1, 17, 4),
                rect(5, 3, 9, 7),
                rect(5, 3, 9, 7),
            ],
            command_center: None,
            menu_slot: None,
            title_colors: None,
        };
        let visible = viewport.visible_rects();
        for y in 0..15 {
            for x in -2..17 {
                let pixel = rect(x, y, x + 1, y + 1);
                let contains = |r: &BoxPx| r.intersect(pixel).is_some();
                let expected =
                    contains(&area) && contains(&frame) && !viewport.clip.iter().any(contains);
                assert_eq!(
                    visible.iter().filter(|r| contains(r)).count(),
                    usize::from(expected),
                    "at ({x}, {y})"
                );
            }
        }
        assert!(
            Viewport {
                frame: None,
                ..viewport
            }
            .visible_rects()
            .is_empty()
        );
    }
}
