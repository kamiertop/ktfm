mod access;
pub use access::EntryAccess;

mod local;

pub use local::{
    Directory, Entry, EntryKind, entry_size, read_child_count, read_directory, read_references,
};

mod watch;
pub use watch::DirectoryWatch;
