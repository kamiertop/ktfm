use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::workspace::SavedPath;

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
}
