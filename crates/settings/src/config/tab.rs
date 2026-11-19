use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{view::ViewMode, workspace::SavedPath};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    Directory { path: SavedPath },
    Tag { id: Uuid },
}

impl Location {
    pub fn directory(path: PathBuf) -> Self {
        Self::Directory {
            path: SavedPath(path),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabItem {
    pub id: Uuid,
    pub location: Location,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<TabItem>,
    pub active: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_mode: Option<ViewMode>,
    pub zoom_step: i8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<SplitSession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSession>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitLayout {
    #[default]
    SideBySide,
    Stacked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitSession {
    pub tabs: Vec<TabItem>,
    pub active: Option<Uuid>,
    pub right_active: bool,
    pub ratio: f32,
    #[serde(default)]
    pub layout: SplitLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub third: Option<ThirdPaneSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPaneSession {
    pub tabs: Vec<TabItem>,
    pub active: Option<Uuid>,
    pub active_index: usize,
    pub middle_ratio: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridSession {
    pub columns: Vec<PaneColumn>,
    // 首个窗格沿用 Session 中的标签页，其余窗格按列从上到下排列。
    pub panes: Vec<PaneSession>,
    pub active_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSession {
    pub tabs: Vec<TabItem>,
    pub active: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_mode: Option<ViewMode>,
    #[serde(default)]
    pub zoom_step: i8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PaneColumn {
    pub rows: usize,
    pub width_ratio: f32,
    pub top_ratio: f32,
    #[serde(default = "single_pane_rows")]
    pub row_panes: [usize; 2],
    #[serde(default)]
    pub row_ratios: [[f32; 3]; 2],
}

fn single_pane_rows() -> [usize; 2] {
    [1; 2]
}

impl PaneColumn {
    pub fn pane_count(&self) -> usize {
        self.row_panes[..self.rows].iter().sum()
    }
}

impl Default for PaneColumn {
    fn default() -> Self {
        Self {
            rows: 1,
            width_ratio: 1.,
            top_ratio: 0.5,
            row_panes: single_pane_rows(),
            row_ratios: [[0.; 3]; 2],
        }
    }
}

impl From<SplitSession> for GridSession {
    fn from(split: SplitSession) -> Self {
        let mut panes = vec![PaneSession {
            tabs: split.tabs,
            active: split.active,
            view_mode: None,
            zoom_step: 0,
        }];
        let mut active_index = usize::from(split.right_active);
        let mut widths = vec![split.ratio, 1. - split.ratio];
        if let Some(third) = split.third {
            panes.push(PaneSession {
                tabs: third.tabs,
                active: third.active,
                view_mode: None,
                zoom_step: 0,
            });
            active_index = third.active_index.min(2);
            widths[1] = third.middle_ratio;
            widths.push(1. - split.ratio - third.middle_ratio);
        }
        let columns = match split.layout {
            SplitLayout::SideBySide => widths
                .into_iter()
                .map(|width_ratio| PaneColumn {
                    width_ratio,
                    ..PaneColumn::default()
                })
                .collect(),
            SplitLayout::Stacked => {
                let top_ratio = if panes.len() == 2 {
                    split.ratio / (split.ratio + widths[1])
                } else {
                    split.ratio
                };
                let mut columns = vec![PaneColumn {
                    rows: 2,
                    top_ratio,
                    ..PaneColumn::default()
                }];
                if panes.len() == 2 {
                    columns.push(PaneColumn::default());
                    columns
                        .iter_mut()
                        .for_each(|column| column.width_ratio = 0.5);
                }
                columns
            }
        };
        Self {
            columns,
            panes,
            active_index,
        }
    }
}
