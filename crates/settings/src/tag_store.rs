//! 本地 Tag 数据库。收藏和标签页会话仍由工作区配置保存。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use futures_lite::future::block_on;
use hex_color::HexColor;
use turso::{Builder, Connection, Row, Value, params};
use uuid::Uuid;

use crate::config::{APP_NAME, workspace::SavedPath};
use crate::tag::{Tag, TagItem};
use crate::{Error, Result};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS tags (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        color TEXT,
        note TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS tag_paths (
        tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        path BLOB NOT NULL,
        PRIMARY KEY (tag_id, path)
    );
    CREATE INDEX IF NOT EXISTS tag_paths_by_path ON tag_paths(path, tag_id);
";

/// 保存 Tag 与路径的多对多关联。每次写入只更新受影响的数据行。
pub struct TagStore {
    connection: Connection,
}

impl TagStore {
    /// 打开本地数据库并返回供界面使用的初始快照。
    pub fn open() -> Result<(Self, Tag)> {
        let dirs = ProjectDirs::from("", "", APP_NAME).ok_or(Error::NoConfigDir)?;
        Self::open_at(&dirs.data_local_dir().join("tags.db"))
    }

    fn open_at(path: &Path) -> Result<(Self, Tag)> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "标签数据库路径必须是 UTF-8")
        })?;
        block_on(async {
            let database = Builder::new_local(path).build().await?;
            let mut connection = database.connect()?;
            connection
                .execute_batch("PRAGMA foreign_keys = ON;")
                .await?;
            connection.execute_batch(SCHEMA).await?;
            // 标签与关联在同一个读事务中加载，避免读到不同提交的状态。
            let transaction = connection.transaction().await?;
            let tags = read_tags(&transaction).await?;
            transaction.commit().await?;
            Ok((Self { connection }, tags))
        })
    }

    /// 创建 Tag 及其初始关联。右键新建并添加时，两个操作一起提交。
    pub fn create(&mut self, tag: &TagItem) -> Result<()> {
        block_on(async {
            let transaction = self.connection.transaction().await?;
            insert_tag(&transaction, tag).await?;
            transaction.commit().await?;
            Ok(())
        })
    }

    /// 修改标签属性，保留原有文件关联。
    pub fn update(
        &mut self,
        id: Uuid,
        name: &str,
        color: Option<HexColor>,
        note: Option<&str>,
    ) -> Result<DateTime<Utc>> {
        let now = Utc::now();
        block_on(async {
            let changed = self.connection.execute(
                "UPDATE tags SET name = ?2, color = ?3, note = ?4, updated_at = ?5 WHERE id = ?1",
                params![id.to_string(), name, color_value(color), note, now.to_rfc3339()],
            ).await?;
            require_tag(changed)?;
            Ok(now)
        })
    }

    /// 显式设置关联是否存在，重复执行同一次变更不会把操作反转。
    pub fn set_path(&mut self, id: Uuid, path: &Path, present: bool) -> Result<DateTime<Utc>> {
        let id = id.to_string();
        let now = Utc::now();
        let path = encode_path(path)?;
        block_on(async {
            let transaction = self.connection.transaction().await?;
            let changed = transaction
                .execute(
                    "UPDATE tags SET updated_at = ?2 WHERE id = ?1",
                    params![id.as_str(), now.to_rfc3339()],
                )
                .await?;
            require_tag(changed)?;
            let sql = if present {
                "INSERT OR IGNORE INTO tag_paths (tag_id, path) VALUES (?1, ?2)"
            } else {
                "DELETE FROM tag_paths WHERE tag_id = ?1 AND path = ?2"
            };
            transaction.execute(sql, params![id, path]).await?;
            transaction.commit().await?;
            Ok(now)
        })
    }

    /// 仅删除标签元数据及其关联，不操作磁盘中的文件。
    pub fn remove(&mut self, id: Uuid) -> Result<()> {
        block_on(async {
            self.connection
                .execute("DELETE FROM tags WHERE id = ?1", [id.to_string()])
                .await?;
            Ok(())
        })
    }
}

async fn insert_tag(connection: &Connection, tag: &TagItem) -> Result<()> {
    let id = tag.id.to_string();
    connection.execute(
        "INSERT INTO tags (id, name, color, note, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.as_str(), tag.name.as_str(), color_value(tag.color), tag.note.as_deref(),
            tag.created_at.to_rfc3339(), tag.updated_at.to_rfc3339()],
    ).await?;
    for path in &tag.paths {
        connection
            .execute(
                "INSERT INTO tag_paths (tag_id, path) VALUES (?1, ?2)",
                params![id.as_str(), encode_path(&path.0)?],
            )
            .await?;
    }
    Ok(())
}

async fn read_tags(connection: &Connection) -> Result<Tag> {
    let mut rows = connection.query(
        "SELECT id, name, color, note, created_at, updated_at FROM tags ORDER BY created_at, id", (),
    ).await?;
    let mut tags = Vec::new();
    let mut indices = BTreeMap::new();
    while let Some(row) = rows.next().await? {
        let id = row.get::<String>(0)?.parse().map_err(invalid)?;
        let color = optional_text(&row, 2)?
            .as_deref()
            .map(HexColor::parse)
            .transpose()
            .map_err(invalid)?;
        indices.insert(id, tags.len());
        tags.push(TagItem {
            id,
            name: row.get(1)?,
            color,
            note: optional_text(&row, 3)?,
            created_at: row.get::<String>(4)?.parse().map_err(invalid)?,
            updated_at: row.get::<String>(5)?.parse().map_err(invalid)?,
            paths: BTreeSet::new(),
        });
    }
    drop(rows);
    let mut rows = connection
        .query("SELECT tag_id, path FROM tag_paths", ())
        .await?;
    while let Some(row) = rows.next().await? {
        let id: Uuid = row.get::<String>(0)?.parse().map_err(invalid)?;
        let index = indices
            .get(&id)
            .ok_or_else(|| invalid("关联引用了不存在的标签"))?;
        let Value::Blob(encoded) = row.get_value(1)? else {
            return Err(invalid("标签路径不是 BLOB"));
        };
        tags[*index].paths.insert(serde_json::from_slice(&encoded)?);
    }
    Ok(Tag::from_items(tags))
}

fn optional_text(row: &Row, index: usize) -> Result<Option<String>> {
    match row.get_value(index)? {
        Value::Null => Ok(None),
        Value::Text(text) => Ok(Some(text)),
        _ => Err(invalid("预期文本或空值")),
    }
}

fn color_value(color: Option<HexColor>) -> Option<String> {
    color.map(|color| color.display_rgba().to_string())
}

/// 以 BLOB 保存路径编码。复用工作区编码以保留非 UTF-8 文件名。
fn encode_path(path: &Path) -> Result<Vec<u8>> {
    let path: PathBuf = path.components().collect();
    Ok(serde_json::to_vec(&SavedPath(path))?)
}

fn require_tag(changed: u64) -> Result<()> {
    if changed == 0 {
        return Err(invalid("标签已不存在"));
    }
    Ok(())
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::InvalidTagData(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_many_to_many_tag_links_and_cascades_removal() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ktfm-test-tag-store-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let result = (|| {
            let db = dir.join("tags.db");
            let file = dir.join("笔记.txt");
            fs::write(&file, "tagged")?;
            #[cfg(unix)]
            let second = {
                use std::os::unix::ffi::OsStringExt;
                dir.join(std::ffi::OsString::from_vec(b"extra-\xff".to_vec()))
            };
            #[cfg(not(unix))]
            let second = dir.join("extra");
            let (mut store, tags) = TagStore::open_at(&db)?;
            assert!(tags.items().is_empty());

            let mut first = TagItem::new("工作".into(), None, None);
            let mut other = TagItem::new("常用".into(), None, None);
            first.set_path(file.clone(), true, first.updated_at);
            other.set_path(file.clone(), true, other.updated_at);
            store.create(&first)?;
            store.create(&other)?;
            store.set_path(first.id, &second, true)?;
            store.set_path(first.id, &second, true)?;
            let color = HexColor::parse_rgb("#195FB5").unwrap();
            store.update(first.id, "归档", Some(color), Some("待整理"))?;
            drop(store);

            let (mut store, tags) = TagStore::open_at(&db)?;
            let saved = tags.get(first.id).unwrap();
            assert_eq!(saved.color, Some(color));
            assert_eq!(saved.name, "归档");
            assert_eq!(saved.note.as_deref(), Some("待整理"));
            assert_eq!(saved.paths.len(), 2);
            assert!(saved.contains(&second));
            assert!(tags.get(other.id).unwrap().contains(&file));

            store.set_path(first.id, &second, false)?;
            store.remove(other.id)?;
            drop(store);

            let (_store, tags) = TagStore::open_at(&db)?;
            assert!(tags.get(other.id).is_none());
            assert!(!tags.get(first.id).unwrap().contains(&second));
            assert_eq!(fs::read_to_string(file)?, "tagged");
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
