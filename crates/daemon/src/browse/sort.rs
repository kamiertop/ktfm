use std::cmp::{Ordering, Reverse};

use vfs::{Entry, EntryKind};

use crate::file_type::file_type;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct FileSort(Option<(SortColumn, SortDirection)>);

impl FileSort {
    pub fn new(column: SortColumn, direction: SortDirection) -> Self {
        Self(Some((column, direction)))
    }

    pub fn selection(self) -> Option<(SortColumn, SortDirection)> {
        self.0
    }

    pub fn is_default(self) -> bool {
        self.0.is_none()
    }

    pub fn by_size(self) -> bool {
        matches!(self.0, Some((SortColumn::Size, _)))
    }

    /// 当前使用串行排序。file_sort 基准中，千级目录串行更快；
    /// 约 9.5 万项时，Rayon 四线程的大小和类型排序约快 2.3、3.3 倍，
    /// 已排好的名称几乎无收益。后续评估并行时按列和规模选择。
    pub fn order(
        self,
        entries: &[Entry],
        visible: &mut [usize],
        child_count: impl Fn(usize) -> Option<u64>,
    ) {
        let Some((column, direction)) = self.0 else {
            visible.sort_unstable();
            return;
        };
        if column == SortColumn::Kind {
            // 只生成一次类型键，避免比较时反复解析文件扩展名。
            visible.sort_by_cached_key(|&index| type_sort_key(entries, index, direction));
            return;
        }
        visible.sort_unstable_by(|&left, &right| self.compare(entries, left, right, &child_count));
    }

    pub fn compare(
        self,
        entries: &[Entry],
        left: usize,
        right: usize,
        child_count: impl Fn(usize) -> Option<u64>,
    ) -> Ordering {
        let Some((column, direction)) = self.0 else {
            return left.cmp(&right);
        };
        let a = &entries[left];
        let b = &entries[right];
        let folders = (a.kind != EntryKind::Directory).cmp(&(b.kind != EntryKind::Directory));
        if folders != Ordering::Equal {
            return folders;
        }
        let ordered = match column {
            SortColumn::Name => direction_order(a.name.cmp(&b.name), direction),
            SortColumn::Size if a.kind == EntryKind::Directory => {
                compare_optional(child_count(left), child_count(right), direction)
            }
            SortColumn::Size => compare_optional(a.size, b.size, direction),
            SortColumn::Kind => direction_order(
                file_type(a.kind, &a.path)
                    .label()
                    .cmp(file_type(b.kind, &b.path).label()),
                direction,
            ),
            SortColumn::Modified => compare_optional(a.modified, b.modified, direction),
            SortColumn::Created => compare_optional(a.created, b.created, direction),
            SortColumn::Accessed => compare_optional(a.accessed, b.accessed, direction),
            SortColumn::Location => compare_optional(a.path.parent(), b.path.parent(), direction),
            SortColumn::Owner => compare_optional(
                a.access.as_ref().map(|access| access.owner.as_ref()),
                b.access.as_ref().map(|access| access.owner.as_ref()),
                direction,
            ),
            SortColumn::Group => compare_optional(
                a.access.as_ref().map(|access| access.group.as_ref()),
                b.access.as_ref().map(|access| access.group.as_ref()),
                direction,
            ),
            SortColumn::Permissions => compare_optional(
                a.access.as_ref().map(|access| access.mode),
                b.access.as_ref().map(|access| access.mode),
                direction,
            ),
        };
        ordered.then_with(|| left.cmp(&right))
    }
}

/// 类型排序使用预计算键，避免比较时反复解析扩展名。
pub fn type_sort_key(
    entries: &[Entry],
    index: usize,
    direction: SortDirection,
) -> (
    bool,
    Option<&'static str>,
    Option<Reverse<&'static str>>,
    usize,
) {
    let entry = &entries[index];
    let kind = file_type(entry.kind, &entry.path).label();
    let (ascending, descending) = if direction == SortDirection::Descending {
        (None, Some(Reverse(kind)))
    } else {
        (Some(kind), None)
    };
    (
        entry.kind != EntryKind::Directory,
        ascending,
        descending,
        index,
    )
}

fn direction_order(order: Ordering, direction: SortDirection) -> Ordering {
    if direction == SortDirection::Descending {
        order.reverse()
    } else {
        order
    }
}

fn compare_optional<T: Ord>(a: Option<T>, b: Option<T>, direction: SortDirection) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => direction_order(a.cmp(&b), direction),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};
    #[test]
    fn size_sort_uses_cached_directory_counts() {
        let entries = ["first", "second", "unknown"].map(|name| Entry {
            name: name.into(),
            path: name.into(),
            kind: EntryKind::Directory,
            size: None,
            modified: None,
            created: None,
            accessed: None,
            access: None,
        });
        let mut visible = [0, 1, 2];
        let counts = [Some(3), Some(8), None];
        FileSort::new(SortColumn::Size, SortDirection::Descending).order(
            &entries,
            &mut visible,
            |index| counts[index],
        );
        assert_eq!(visible, [1, 0, 2]);
    }

    #[test]
    fn list_sort_uses_raw_metadata_and_keeps_missing_values_last() {
        let entry = |name: &str, kind, size, seconds: Option<u64>| Entry {
            name: name.into(),
            path: name.into(),
            kind,
            size,
            modified: seconds.map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            created: None,
            accessed: None,
            access: None,
        };
        let entries = [
            entry("folder", EntryKind::Directory, None, Some(1_u64)),
            entry("a", EntryKind::File, Some(100), Some(30)),
            entry("b", EntryKind::File, Some(9), Some(50)),
            entry("c", EntryKind::File, None, None),
        ];
        let mut visible = [0, 1, 2, 3];

        FileSort::new(SortColumn::Size, SortDirection::Ascending).order(
            &entries,
            &mut visible,
            |_| None,
        );
        assert_eq!(visible, [0, 2, 1, 3]);
        FileSort::new(SortColumn::Size, SortDirection::Descending).order(
            &entries,
            &mut visible,
            |_| None,
        );
        assert_eq!(visible, [0, 1, 2, 3]);
        FileSort::new(SortColumn::Modified, SortDirection::Descending).order(
            &entries,
            &mut visible,
            |_| None,
        );
        assert_eq!(visible, [0, 2, 1, 3]);
        FileSort::default().order(&entries, &mut visible, |_| None);
        assert_eq!(visible, [0, 1, 2, 3]);
    }
}
