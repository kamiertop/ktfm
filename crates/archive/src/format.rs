/// 归档层预留的压缩或归档格式标识。
pub enum CompressKind {
    SevenZip,
    Zip,
    Tar,
    Xz2,
    Zstd,
    Flate2,
    Rar,
    UnSupported,
}
