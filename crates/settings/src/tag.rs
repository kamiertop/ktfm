use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hex_color::HexColor;
use uuid::Uuid;

use crate::config::workspace::SavedPath;

/// 聚合来自不同目录的文件和文件夹。
#[derive(Debug, Clone)]
pub struct TagItem {
    pub id: Uuid,
    pub name: String,
    pub color: Option<HexColor>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub paths: BTreeSet<SavedPath>,
}

impl TagItem {
    pub fn new(name: String, color: Option<HexColor>, note: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            color,
            note,
            created_at: now,
            updated_at: now,
            paths: BTreeSet::new(),
        }
    }

    pub fn update(
        &mut self,
        name: String,
        color: Option<HexColor>,
        note: Option<String>,
        updated_at: DateTime<Utc>,
    ) {
        self.name = name;
        self.color = color;
        self.note = note;
        self.updated_at = updated_at;
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.paths.contains(path)
    }

    pub fn set_path(&mut self, path: PathBuf, present: bool, updated_at: DateTime<Utc>) {
        if present {
            self.paths.insert(SavedPath(path.components().collect()));
        } else {
            self.paths.remove(path.as_path());
        }
        self.updated_at = updated_at;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Tag {
    items: Vec<TagItem>,
}

impl Tag {
    pub(crate) fn from_items(items: Vec<TagItem>) -> Self {
        Self { items }
    }

    pub fn items(&self) -> &[TagItem] {
        &self.items
    }

    pub fn get(&self, id: Uuid) -> Option<&TagItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut TagItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    pub fn insert(&mut self, tag: TagItem) {
        self.items.push(tag);
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|item| item.id != id);
    }
}
