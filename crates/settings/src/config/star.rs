use std::path::{Path, PathBuf};

use hex_color::HexColor;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::workspace::SavedPath;

/// 固定在侧栏的目录收藏。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarItem {
    pub id: Uuid,
    pub path: SavedPath,
    pub color: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Star {
    items: Vec<StarItem>,
}

impl Star {
    pub fn items(&self) -> &[StarItem] {
        &self.items
    }

    pub fn find(&self, path: &Path) -> Option<&StarItem> {
        self.items.iter().find(|item| item.path.0 == path)
    }

    pub fn add(&mut self, path: PathBuf, color: Option<HexColor>) -> Uuid {
        if let Some(item) = self.find(&path) {
            return item.id;
        }
        let id = Uuid::new_v4();
        self.items.push(StarItem {
            id,
            path: SavedPath(path),
            color,
        });
        id
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|item| item.id != id);
    }
}
