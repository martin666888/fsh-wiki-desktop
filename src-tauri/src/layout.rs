//! Pure workspace state. A document belongs to at most one visible pane.
use serde::{Deserialize, Serialize};

pub const TAB_BAR_HEIGHT: f64 = 42.0;
pub const PANE_HEADER_HEIGHT: f64 = 34.0;
pub const DIVIDER_WIDTH: f64 = 6.0;
pub const PANE_BORDER: f64 = 0.0;
pub const MIN_PANE_WIDTH: f64 = 480.0;
pub const MIN_SPLIT_WIDTH: f64 = MIN_PANE_WIDTH * 2.0 + DIVIDER_WIDTH;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pane {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Default)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Layout {
    #[default]
    Empty,
    Single {
        tab: String,
    },
    Picking {
        left: String,
        focused: Pane,
        ratio: f64,
    },
    Split {
        left: String,
        right: String,
        focused: Pane,
        ratio: f64,
    },
}

impl Layout {
    pub fn is_split(&self) -> bool {
        matches!(self, Self::Picking { .. } | Self::Split { .. })
    }

    pub fn pane(&self, label: &str) -> Option<Pane> {
        match self {
            Self::Single { tab } if tab == label => Some(Pane::Left),
            Self::Picking { left, .. } | Self::Split { left, .. } if left == label => {
                Some(Pane::Left)
            }
            Self::Split { right, .. } if right == label => Some(Pane::Right),
            _ => None,
        }
    }

    pub fn focused(&self) -> Pane {
        match self {
            Self::Picking { focused, .. } | Self::Split { focused, .. } => *focused,
            _ => Pane::Left,
        }
    }

    pub fn document(&self, pane: Pane) -> Option<&str> {
        match (self, pane) {
            (Self::Single { tab }, Pane::Left) => Some(tab),
            (Self::Picking { left, .. } | Self::Split { left, .. }, Pane::Left) => Some(left),
            (Self::Split { right, .. }, Pane::Right) => Some(right),
            _ => None,
        }
    }

    pub fn active(&self) -> Option<&str> {
        self.document(self.focused())
    }

    pub fn retained(&self) -> Option<&str> {
        self.active().or_else(|| self.document(Pane::Left))
    }

    pub fn focus(&mut self, pane: Pane) {
        match self {
            Self::Picking { focused, .. } | Self::Split { focused, .. } => *focused = pane,
            _ => {}
        }
    }

    pub fn select(&mut self, tab: String, target: Pane) {
        if let Some(pane) = self.pane(&tab) {
            self.focus(pane);
            return;
        }
        match self {
            Self::Empty | Self::Single { .. } => *self = Self::Single { tab },
            Self::Picking {
                left,
                focused,
                ratio,
            } => {
                if target == Pane::Left {
                    *left = tab;
                    *focused = Pane::Left;
                } else {
                    *self = Self::Split {
                        left: left.clone(),
                        right: tab,
                        focused: Pane::Right,
                        ratio: *ratio,
                    };
                }
            }
            Self::Split {
                left,
                right,
                focused,
                ..
            } => {
                match target {
                    Pane::Left => *left = tab,
                    Pane::Right => *right = tab,
                }
                *focused = target;
            }
        }
    }

    pub fn enter(&mut self) -> Result<(), String> {
        let Self::Single { tab } = self else {
            return Err("请先打开一篇文章".into());
        };
        *self = Self::Picking {
            left: tab.clone(),
            focused: Pane::Right,
            ratio: 0.5,
        };
        Ok(())
    }

    pub fn exit(&mut self) {
        if let Some(tab) = self.retained().map(str::to_owned) {
            *self = Self::Single { tab };
        }
    }

    pub fn ratio(&self) -> f64 {
        match self {
            Self::Picking { ratio, .. } | Self::Split { ratio, .. } => *ratio,
            _ => 0.5,
        }
    }

    pub fn set_ratio(&mut self, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("分屏比例无效".into());
        }
        match self {
            Self::Picking { ratio, .. } | Self::Split { ratio, .. } => {
                *ratio = value.clamp(0.0, 1.0)
            }
            _ => return Err("请先进入分屏".into()),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
    pub left_width: f64,
    pub right_x: f64,
    pub can_split: bool,
    pub suspended: bool,
    pub min_ratio: f64,
}

impl Viewport {
    pub fn calculate(width: f64, height: f64, scale: f64, layout: &Layout) -> Self {
        let can_split = width >= MIN_SPLIT_WIDTH && height >= PANE_HEADER_HEIGHT + 120.0;
        let available = (width - DIVIDER_WIDTH).max(1.0);
        let min_ratio = (MIN_PANE_WIDTH / available).min(0.5);
        let ratio = layout.ratio().clamp(min_ratio, 1.0 - min_ratio);
        let left_width = (available * ratio * scale).round() / scale;
        Self {
            width,
            height,
            left_width,
            right_x: left_width + DIVIDER_WIDTH,
            can_split,
            suspended: layout.is_split() && !can_split,
            min_ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pair_retains_both_pages_and_focused_exit() {
        let mut layout = Layout::Single { tab: "a".into() };
        layout.enter().unwrap();
        layout.select("b".into(), Pane::Right);
        assert_eq!(layout.document(Pane::Left), Some("a"));
        assert_eq!(layout.document(Pane::Right), Some("b"));
        layout.exit();
        assert_eq!(layout, Layout::Single { tab: "b".into() });
    }
    #[test]
    fn resize_preserves_desired_ratio() {
        let mut layout = Layout::Single { tab: "a".into() };
        layout.enter().unwrap();
        layout.set_ratio(0.7).unwrap();
        assert!(Viewport::calculate(800.0, 700.0, 1.25, &layout).suspended);
        let large = Viewport::calculate(2000.0, 700.0, 1.25, &layout);
        assert!((large.left_width / (2000.0 - DIVIDER_WIDTH) - 0.7).abs() < 0.001);
        assert!(layout.set_ratio(f64::NAN).is_err());
    }
}
