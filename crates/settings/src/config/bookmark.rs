use std::path::{Path, PathBuf};

use hex_color::HexColor;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::workspace::SavedPath;

/// 侧边书签栏，经常访问的目录
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: Uuid,
    pub path: SavedPath,
    pub color: Option<HexColor>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: BookmarkKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkKind {
    #[default]
    Directory,
    File,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmarks {
    items: Vec<Bookmark>,
}

impl Bookmarks {
    pub fn items(&self) -> &[Bookmark] {
        &self.items
    }

    pub fn find(&self, path: &Path) -> Option<&Bookmark> {
        self.items.iter().find(|item| item.path.0 == path)
    }

    pub fn add(&mut self, path: PathBuf, kind: BookmarkKind) -> Uuid {
        if let Some(item) = self.find(&path) {
            return item.id;
        }
        let id = Uuid::new_v4();
        self.items.push(Bookmark {
            id,
            path: SavedPath(path),
            color: None,
            label: None,
            kind,
        });
        id
    }

    /// 将书签移到指定书签之前，或在没有指定目标时移到末尾。
    pub fn move_before(&mut self, id: Uuid, before: Option<Uuid>) {
        if before == Some(id) {
            return;
        }
        let Some(source) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let target = match before {
            Some(before) => {
                let Some(target) = self.items.iter().position(|item| item.id == before) else {
                    return;
                };
                target
            }
            None => self.items.len(),
        };
        let target = if source < target { target - 1 } else { target };
        if source != target {
            let item = self.items.remove(source);
            self.items.insert(target, item);
        }
    }

    pub fn get(&self, id: Uuid) -> Option<&Bookmark> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn rename(&mut self, id: Uuid, label: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.label = label;
        }
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|item| item.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_bookmarks_between_neighbors_without_changing_their_details() {
        let mut bookmarks = Bookmarks::default();
        let a = bookmarks.add("a".into(), BookmarkKind::Directory);
        let b = bookmarks.add("b".into(), BookmarkKind::Directory);
        let c = bookmarks.add("c".into(), BookmarkKind::File);
        bookmarks.rename(c, Some("别名".into()));

        bookmarks.move_before(c, Some(b));
        assert_eq!(
            bookmarks
                .items()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [a, c, b]
        );
        bookmarks.move_before(a, Some(b));
        assert_eq!(
            bookmarks
                .items()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [c, a, b]
        );
        bookmarks.move_before(c, None);
        assert_eq!(
            bookmarks
                .items()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [a, b, c]
        );
        assert_eq!(bookmarks.get(c).unwrap().label.as_deref(), Some("别名"));
        assert_eq!(bookmarks.get(c).unwrap().kind, BookmarkKind::File);
    }
}
