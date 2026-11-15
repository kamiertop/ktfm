use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
    Missing,
}

#[derive(Debug)]
pub struct Entry {
    pub name: OsString,
    pub kind: EntryKind,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        self.name.as_encoded_bytes().starts_with(b".")
    }
}

#[derive(Debug)]
pub struct Directory {
    pub entries: Vec<Entry>,
    pub read_errors: usize,
}

/// 枚举当前目录，不递归。取消时返回 Interrupted。
pub fn read_directory(path: &Path, cancelled: impl Fn() -> bool) -> io::Result<Directory> {
    let check_cancelled = || {
        if cancelled() {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        } else {
            Ok(())
        }
    };

    check_cancelled()?;
    let mut directory = Directory {
        entries: Vec::new(),
        read_errors: 0,
    };

    for item in fs::read_dir(path)? {
        check_cancelled()?;
        let Ok(item) = item else {
            directory.read_errors += 1;
            continue;
        };
        let kind = match item.file_type() {
            Ok(kind) => entry_kind(kind),
            Err(_) => {
                directory.read_errors += 1;
                continue;
            }
        };
        directory.entries.push(Entry {
            name: item.file_name(),
            kind,
        });
    }

    check_cancelled()?;
    // 同类条目按原始名称排序，保留非 UTF-8 文件名。
    directory.entries.sort_unstable_by(|a, b| {
        (a.kind != EntryKind::Directory, &a.name).cmp(&(b.kind != EntryKind::Directory, &b.name))
    });
    check_cancelled()?;
    Ok(directory)
}

/// 读取标签引用，并保留暂时不可访问的路径供用户管理。
pub fn read_references(
    paths: Vec<PathBuf>,
    cancelled: impl Fn() -> bool,
) -> io::Result<(Directory, Vec<PathBuf>)> {
    let mut entries = Vec::with_capacity(paths.len());
    let mut read_errors = 0;
    for path in paths {
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let kind = match fs::symlink_metadata(&path) {
            Ok(metadata) => entry_kind(metadata.file_type()),
            Err(_) => {
                read_errors += 1;
                EntryKind::Missing
            }
        };
        let name = path.file_name().unwrap_or(path.as_os_str()).to_os_string();
        entries.push((Entry { name, kind }, path));
    }
    entries.sort_unstable_by(|(a, a_path), (b, b_path)| {
        (a.kind != EntryKind::Directory, &a.name, a_path).cmp(&(
            b.kind != EntryKind::Directory,
            &b.name,
            b_path,
        ))
    });
    let (entries, paths) = entries.into_iter().unzip();
    Ok((
        Directory {
            entries,
            read_errors,
        },
        paths,
    ))
}

fn entry_kind(kind: fs::FileType) -> EntryKind {
    if kind.is_dir() {
        EntryKind::Directory
    } else if kind.is_file() {
        EntryKind::File
    } else if kind.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_distinct_paths_for_same_name_and_missing_references() -> io::Result<()> {
        let root =
            std::env::temp_dir().join(format!("ktfm-test-references-{}", std::process::id()));
        fs::create_dir(&root)?;
        let result = (|| {
            fs::create_dir(root.join("a"))?;
            fs::create_dir(root.join("b"))?;
            let first = root.join("a/same");
            let second = root.join("b/same");
            fs::write(&first, "")?;
            fs::write(&second, "")?;
            let missing = root.join("z-missing");
            let (directory, paths) =
                read_references(vec![missing.clone(), second.clone(), first.clone()], || {
                    false
                })?;
            assert_eq!(directory.read_errors, 1);
            assert_eq!(paths, [first, second, missing]);
            assert_eq!(directory.entries[0].name, directory.entries[1].name);
            assert_eq!(directory.entries[2].kind, EntryKind::Missing);
            Ok(())
        })();
        fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn reads_only_immediate_entries_and_sorts_folders_first() -> io::Result<()> {
        let path = std::env::temp_dir().join(format!("ktfm-test-directory-{}", std::process::id()));
        fs::create_dir(&path)?;
        let result = (|| {
            fs::create_dir(path.join("z-folder"))?;
            fs::write(path.join("z-folder/nested"), "")?;
            fs::write(path.join("b.txt"), "")?;
            fs::write(path.join(".a.txt"), "")?;
            #[cfg(unix)]
            let raw_name = {
                use std::os::unix::ffi::OsStringExt;
                let name = OsString::from_vec(vec![0xff, b'x']);
                fs::write(path.join(&name), "")?;
                std::os::unix::fs::symlink("missing", path.join("dangling"))?;
                name
            };

            let directory = read_directory(&path, || false)?;
            assert_eq!(directory.read_errors, 0);
            assert_eq!(directory.entries[0].kind, EntryKind::Directory);
            assert_eq!(directory.entries[0].name, "z-folder");
            assert_eq!(directory.entries[1].name, ".a.txt");
            assert!(directory.entries[1].is_hidden());
            assert_eq!(directory.entries[2].name, "b.txt");
            #[cfg(unix)]
            {
                assert_eq!(directory.entries[3].kind, EntryKind::Symlink);
                assert_eq!(directory.entries[4].name, raw_name);
            }
            assert_eq!(directory.entries.len(), if cfg!(unix) { 5 } else { 3 });

            let calls = std::cell::Cell::new(0);
            let error = read_directory(&path, || {
                let count = calls.get();
                calls.set(count + 1);
                count >= 2
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            Ok(())
        })();
        fs::remove_dir_all(&path)?;
        result
    }
}
