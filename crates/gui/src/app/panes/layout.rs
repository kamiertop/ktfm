use settings::config::tab::{PaneColumn, SplitLayout};

use super::{MAX_COLUMNS, MAX_PANES};

pub(in crate::app) struct PaneLayout {
    pub(in crate::app) columns: Vec<PaneColumn>,
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self {
            columns: vec![PaneColumn::default()],
        }
    }
}

impl PaneLayout {
    pub(super) fn restore(columns: Vec<PaneColumn>, count: usize) -> Option<Self> {
        if columns.is_empty()
            || columns.len() > MAX_COLUMNS
            || count > MAX_PANES
            || columns.iter().any(|column| {
                !(1..=2).contains(&column.rows)
                    || column
                        .row_panes
                        .iter()
                        .any(|count| !(1..=MAX_COLUMNS).contains(count))
            })
        {
            return None;
        }
        let mut layout = Self { columns };
        if layout.pane_count() != count {
            return None;
        }
        let width_sum: f32 = layout.columns.iter().map(|column| column.width_ratio).sum();
        if !width_sum.is_finite()
            || width_sum <= 0.
            || layout.columns.iter().any(|column| column.width_ratio < 0.)
        {
            layout.equalize_columns();
        } else {
            for column in &mut layout.columns {
                column.width_ratio /= width_sum;
            }
        }
        for column in &mut layout.columns {
            column.top_ratio = if column.top_ratio.is_finite() {
                column.top_ratio.clamp(0., 1.)
            } else {
                0.5
            };
        }
        Some(layout)
    }

    fn pane_count(&self) -> usize {
        self.columns.iter().map(PaneColumn::pane_count).sum()
    }

    pub(super) fn pane_column(&self, pane: usize) -> (usize, usize) {
        let mut start = 0;
        for (index, column) in self.columns.iter().enumerate() {
            if pane < start + column.pane_count() {
                return (index, start);
            }
            start += column.pane_count();
        }
        unreachable!("窗格属于某一列")
    }

    // 返回列、行以及该行在窗格列表中的起始位置。
    pub(super) fn pane_row(&self, pane: usize) -> (usize, usize, usize) {
        let (column, start) = self.pane_column(pane);
        let top_count = self.columns[column].row_panes[0];
        if pane < start + top_count {
            (column, 0, start)
        } else {
            (column, 1, start + top_count)
        }
    }

    pub(super) fn can_split(&self, active: usize, layout: SplitLayout) -> bool {
        if self.pane_count() >= MAX_PANES {
            return false;
        }
        let (column, row, _) = self.pane_row(active);
        let column = &self.columns[column];
        match layout {
            SplitLayout::SideBySide if column.rows == 2 || column.row_panes[row] > 1 => {
                column.row_panes[row] < MAX_COLUMNS
            }
            SplitLayout::SideBySide => self.columns.len() < MAX_COLUMNS,
            SplitLayout::Stacked => column.rows == 1,
        }
    }

    fn equalize_columns(&mut self) {
        let width = 1. / self.columns.len() as f32;
        for column in &mut self.columns {
            column.width_ratio = width;
        }
    }

    pub(super) fn remove_pane(&mut self, column: usize, row: usize) {
        let layout = &mut self.columns[column];
        if layout.row_panes[row] > 1 {
            layout.row_panes[row] -= 1;
            layout.row_ratios[row] = [0.; 3];
        } else if layout.rows == 2 {
            collapse_rows(layout, 1 - row);
        } else {
            self.columns.remove(column);
            self.equalize_columns();
        }
    }

    pub(super) fn neighbor(
        &self,
        active: usize,
        layout: SplitLayout,
        forward: bool,
    ) -> Option<usize> {
        let (column, row, start) = self.pane_row(active);
        let position = active - start;
        let current = &self.columns[column];
        let offset = if forward { 1 } else { -1 };
        Some(match layout {
            SplitLayout::Stacked => {
                if current.rows != 2 || row == usize::from(forward) {
                    return None;
                }
                let other_start = if forward {
                    start + current.row_panes[0]
                } else {
                    start - current.row_panes[0]
                };
                other_start + position.min(current.row_panes[1 - row] - 1)
            }
            SplitLayout::SideBySide => {
                if let Some(next) = position.checked_add_signed(offset)
                    && next < current.row_panes[row]
                {
                    return Some(start + next);
                }
                let next = column
                    .checked_add_signed(offset)
                    .filter(|&next| next < self.columns.len())?;
                let target = &self.columns[next];
                let next_start: usize = self.columns[..next]
                    .iter()
                    .map(PaneColumn::pane_count)
                    .sum();
                let target_row = row.min(target.rows - 1);
                let row_start = if target_row == 0 {
                    next_start
                } else {
                    next_start + target.row_panes[0]
                };
                row_start
                    + if forward {
                        0
                    } else {
                        target.row_panes[target_row] - 1
                    }
            }
        })
    }

    pub(super) fn split(&mut self, active: usize, layout: SplitLayout, forward: bool) -> usize {
        let (column, row, start) = self.pane_row(active);
        match layout {
            SplitLayout::SideBySide
                if self.columns[column].rows == 2 || self.columns[column].row_panes[row] > 1 =>
            {
                let count = &mut self.columns[column].row_panes[row];
                let index = active + usize::from(forward);
                *count += 1;
                self.columns[column].row_ratios[row] = [0.; 3];
                index
            }
            SplitLayout::SideBySide => {
                let (column, index) = if forward {
                    (self.columns.len(), self.pane_count())
                } else {
                    (column, start)
                };
                self.columns.insert(column, PaneColumn::default());
                self.equalize_columns();
                index
            }
            SplitLayout::Stacked => {
                let column = &mut self.columns[column];
                column.rows = 2;
                column.top_ratio = 0.5;
                if forward {
                    start + column.row_panes[0]
                } else {
                    column.row_panes.swap(0, 1);
                    column.row_ratios.swap(0, 1);
                    start
                }
            }
        }
    }
}

fn collapse_rows(column: &mut PaneColumn, row: usize) {
    column.rows = 1;
    column.row_panes = [column.row_panes[row], 1];
    column.row_ratios = [column.row_ratios[row], [0.; 3]];
}

#[cfg(test)]
mod tests {
    use super::{PaneLayout, SplitLayout};

    #[test]
    fn horizontal_split_only_changes_the_target_row() {
        use SplitLayout::{SideBySide, Stacked};

        for (active, counts) in [(0, [2, 1]), (1, [1, 2])] {
            let mut layout = PaneLayout::default();
            assert_eq!(layout.split(0, Stacked, true), 1);
            assert_eq!(layout.split(active, SideBySide, true), active + 1);
            assert_eq!(layout.columns.len(), 1);
            assert_eq!(layout.columns[0].row_panes, counts);
            assert_eq!(layout.neighbor(active, SideBySide, true), Some(active + 1));
            let (column, row, _) = layout.pane_row(active + 1);
            layout.remove_pane(column, row);
            assert_eq!(layout.columns[0].row_panes, [1, 1]);
            assert_eq!(layout.columns[0].rows, 2);
        }
    }

    #[test]
    fn removing_a_column_leaves_the_other_two_in_order() {
        let mut layout = PaneLayout::default();
        assert_eq!(layout.split(0, SplitLayout::SideBySide, true), 1);
        assert_eq!(layout.split(0, SplitLayout::SideBySide, true), 2);
        let (column, row, _) = layout.pane_row(1);
        layout.remove_pane(column, row);
        assert_eq!(layout.columns.len(), 2);
        assert_eq!(layout.neighbor(0, SplitLayout::SideBySide, true), Some(1));
    }
}
