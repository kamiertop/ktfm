mod local;

pub use local::{Directory, Entry, EntryKind, read_directory, read_references};

mod watch;
pub use watch::DirectoryWatch;
