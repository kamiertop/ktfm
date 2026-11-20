#[cfg(unix)]
use nix::unistd::{Gid, Group, Uid, User};
#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{fs::Metadata, sync::Arc};

#[derive(Debug)]
pub struct EntryAccess {
    pub owner: Arc<str>,
    pub group: Arc<str>,
    /// POSIX 权限位，包含 setuid、setgid 和 sticky 位。
    pub mode: u32,
}

#[derive(Default)]
pub(crate) struct AccessReader {
    #[cfg(unix)]
    owners: HashMap<u32, Arc<str>>,
    #[cfg(unix)]
    groups: HashMap<u32, Arc<str>>,
}

impl AccessReader {
    #[cfg(unix)]
    pub(crate) fn read(&mut self, metadata: &Metadata) -> Option<EntryAccess> {
        let uid = metadata.uid();
        let gid = metadata.gid();
        let owner = self.owners.entry(uid).or_insert_with(|| {
            User::from_uid(Uid::from_raw(uid))
                .ok()
                .flatten()
                .map_or_else(|| Arc::from(uid.to_string()), |user| Arc::from(user.name))
        });
        let group = self.groups.entry(gid).or_insert_with(|| {
            Group::from_gid(Gid::from_raw(gid))
                .ok()
                .flatten()
                .map_or_else(|| Arc::from(gid.to_string()), |group| Arc::from(group.name))
        });
        Some(EntryAccess {
            owner: Arc::clone(owner),
            group: Arc::clone(group),
            mode: metadata.mode() & 0o7777,
        })
    }

    #[cfg(not(unix))]
    pub(crate) fn read(&mut self, _: &Metadata) -> Option<EntryAccess> {
        None
    }
}
