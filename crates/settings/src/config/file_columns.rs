use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileColumn {
    Name,
    Size,
    Kind,
    Modified,
    Created,
    Accessed,
    Location,
    Owner,
    Group,
    Permissions,
}

impl FileColumn {
    const ALL: [Self; 10] = [
        Self::Name,
        Self::Size,
        Self::Kind,
        Self::Modified,
        Self::Created,
        Self::Accessed,
        Self::Location,
        Self::Owner,
        Self::Group,
        Self::Permissions,
    ];

    pub const MIN_WIDTH: f32 = 64.;
    pub const MAX_WIDTH: f32 = 1200.;
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FileColumnSetting {
    pub column: FileColumn,
    pub visible: bool,
    pub width: Option<f32>,
}

impl FileColumnSetting {
    fn new(column: FileColumn) -> Self {
        Self {
            column,
            visible: matches!(
                column,
                FileColumn::Name | FileColumn::Size | FileColumn::Kind | FileColumn::Modified
            ),
            width: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileListColumns {
    pub columns: Vec<FileColumnSetting>,
}

impl Default for FileListColumns {
    fn default() -> Self {
        Self {
            columns: FileColumn::ALL.map(FileColumnSetting::new).into(),
        }
    }
}

impl FileListColumns {
    /// 补齐新增字段，保留用户顺序，并保证名称列始终显示在首位。
    pub fn normalize(mut self) -> Self {
        let mut remaining = FileColumn::ALL.to_vec();
        self.columns.retain_mut(|setting| {
            let Some(index) = remaining
                .iter()
                .position(|column| *column == setting.column)
            else {
                return false;
            };
            remaining.remove(index);
            setting.width = setting
                .width
                .filter(|width| width.is_finite())
                .map(|width| width.clamp(FileColumn::MIN_WIDTH, FileColumn::MAX_WIDTH));
            true
        });
        self.columns
            .extend(remaining.into_iter().map(FileColumnSetting::new));
        let name = self
            .columns
            .iter()
            .position(|setting| setting.column == FileColumn::Name)
            .unwrap();
        let mut setting = self.columns.remove(name);
        setting.visible = true;
        self.columns.insert(0, setting);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::workspace::Workspace;

    #[test]
    fn workspace_preserves_column_order_visibility_and_width() {
        let mut workspace: Workspace = toml::from_str("[session]\ntabs = []\n").unwrap();
        assert_eq!(workspace.file_list_columns, FileListColumns::default());

        let columns = &mut workspace.file_list_columns.columns;
        columns.swap(1, 4);
        columns[1].visible = true;
        columns[1].width = Some(174.);
        columns[4].visible = false;
        columns[4].width = Some(96.);

        let restored: Workspace = toml::from_str(&toml::to_string(&workspace).unwrap()).unwrap();
        assert_eq!(
            restored.file_list_columns.normalize(),
            workspace.file_list_columns
        );
    }

    #[test]
    fn normalizing_recovers_missing_columns_and_keeps_name_first() {
        let columns = FileListColumns {
            columns: vec![
                FileColumnSetting {
                    column: FileColumn::Modified,
                    visible: true,
                    width: Some(f32::NAN),
                },
                FileColumnSetting {
                    column: FileColumn::Name,
                    visible: false,
                    width: Some(1.),
                },
                FileColumnSetting::new(FileColumn::Modified),
            ],
        }
        .normalize()
        .columns;
        assert_eq!(columns.len(), FileColumn::ALL.len());
        assert_eq!(columns[0].column, FileColumn::Name);
        assert!(columns[0].visible);
        assert_eq!(columns[0].width, Some(FileColumn::MIN_WIDTH));
        assert_eq!(columns[1].column, FileColumn::Modified);
        assert_eq!(columns[1].width, None);
    }
}
