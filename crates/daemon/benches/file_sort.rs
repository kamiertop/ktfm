//! cargo bench -p daemon --bench file_sort --offline
//! 在命令行传入平铺目录的绝对路径，可分别测量读取、筛选和排序。
//! 排序计时排除目录读取、输入复制和结果校验。

use std::collections::HashMap;
use std::hint;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use daemon::browse;
use daemon::browse::{DirectorySnapshot, FileSort, ReadSource, SortColumn, SortDirection};
use rayon::slice::ParallelSliceMut;
use rayon::{ThreadPool, ThreadPoolBuilder};
use vfs::{Entry, EntryKind};

fn main() {
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    let pool = ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("基准测试需要可用的 Rayon 线程池");
    println!("Rayon threads: {threads}; times are median microseconds (21 runs)");
    println!(
        "{:>8}  {:<19}  {:>11}  {:>11}  {:>8}",
        "entries", "case", "serial", "Rayon", "speedup"
    );

    let directories = std::env::args_os()
        .skip(1)
        .filter(|argument| argument != "--bench")
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if directories.is_empty() {
        for count in [1_000, 10_000, 50_000, 200_000] {
            let entries = sample_entries(count);
            let counts: Vec<_> = (0..count)
                .map(|index| Some(mix(index as u64) % 200))
                .collect();
            benchmark_sort(&pool, &entries, &counts);
        }
    } else {
        for path in directories {
            benchmark_directory(&pool, &path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
    }
}

fn benchmark_directory(pool: &ThreadPool, path: &std::path::Path) -> std::io::Result<()> {
    let cancelled = AtomicBool::new(false);
    let read = || DirectorySnapshot::read(ReadSource::Directory(path.to_owned()), &cancelled);
    let snapshot = read()?;
    let mut reads = Vec::with_capacity(5);
    for _ in 0..5 {
        let started = Instant::now();
        let fresh = read()?;
        reads.push(started.elapsed());
        assert_eq!(
            fresh.directory.entries.len(),
            snapshot.directory.entries.len()
        );
        assert_eq!(fresh.directory.read_errors, 0);
        hint::black_box(fresh);
    }
    println!(
        "Directory {}: {} entries; warm read median {:.2} ms (5 runs)",
        path.display(),
        snapshot.directory.entries.len(),
        median_us(&mut reads) / 1e3
    );

    let counts = HashMap::new();
    for (label, sort) in [
        ("filter only", FileSort::default()),
        (
            "filter + size sort",
            FileSort::new(SortColumn::Size, SortDirection::Ascending),
        ),
    ] {
        let filter =
            || snapshot.visible_entries(hint::black_box("driver"), true, sort, &counts, &cancelled);
        let matches = filter().len();
        let mut samples = (0..21)
            .map(|_| {
                let started = Instant::now();
                let entries = filter();
                let elapsed = started.elapsed();
                hint::black_box(entries);
                elapsed
            })
            .collect::<Vec<_>>();
        println!(
            "{label}: query 'driver', {matches} matches, median {:.3} ms",
            median_us(&mut samples) / 1e3
        );
    }

    // 平铺目录只有文件，因此无子目录项目数参与排序。
    benchmark_sort(pool, &snapshot.directory.entries, &[]);
    Ok(())
}

fn benchmark_sort(pool: &ThreadPool, entries: &[Entry], counts: &[Option<u64>]) {
    let count = entries.len();
    let ordered = (0..count).collect::<Vec<_>>();
    let mut shuffled = ordered.clone();
    shuffled.sort_unstable_by_key(|&index| mix(index as u64 ^ 0x5a5a));
    for (label, column, input) in [
        ("name, already sorted", SortColumn::Name, ordered.as_slice()),
        ("name, shuffled", SortColumn::Name, shuffled.as_slice()),
        ("size", SortColumn::Size, ordered.as_slice()),
        ("type, cached key", SortColumn::Kind, ordered.as_slice()),
    ] {
        let sort = FileSort::new(column, SortDirection::Ascending);
        let child_count = |index: usize| counts.get(index).copied().flatten();
        let serial = |rows: &mut [usize]| sort.order(entries, rows, child_count);
        let parallel = |rows: &mut [usize]| {
            pool.install(|| {
                if column == SortColumn::Kind {
                    rows.par_sort_by_cached_key(|&index| {
                        browse::type_sort_key(entries, index, SortDirection::Ascending)
                    });
                } else {
                    rows.par_sort_unstable_by(|&a, &b| sort.compare(entries, a, b, child_count));
                }
            });
        };
        compare(pool, count, label, input, serial, parallel);
    }
}

fn compare(
    pool: &ThreadPool,
    count: usize,
    label: &str,
    input: &[usize],
    serial: impl Fn(&mut [usize]),
    parallel: impl Fn(&mut [usize]),
) {
    let mut expected = input.to_vec();
    let mut actual = expected.clone();
    // 预热线程池并验证两种算法的顺序完全一致。
    serial(&mut expected);
    pool.broadcast(|_| ());
    parallel(&mut actual);
    assert_eq!(actual, expected);

    let mut serial_times = Vec::with_capacity(21);
    let mut parallel_times = Vec::with_capacity(21);
    for run in 0..21 {
        // 轮换先后顺序，减少缓存和 CPU 频率造成的系统性偏差。
        if run % 2 == 0 {
            serial_times.push(time(input, &serial));
            parallel_times.push(time(input, &parallel));
        } else {
            parallel_times.push(time(input, &parallel));
            serial_times.push(time(input, &serial));
        }
    }
    let serial_us = median_us(&mut serial_times);
    let parallel_us = median_us(&mut parallel_times);
    println!(
        "{count:>8}  {label:<19}  {serial_us:>11.1}  {parallel_us:>11.1}  {:>7.2}x",
        serial_us / parallel_us
    );
}

fn median_us(samples: &mut [Duration]) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1e6
}

fn time(input: &[usize], sort: impl FnOnce(&mut [usize])) -> Duration {
    let mut rows = input.to_vec();
    let started = Instant::now();
    sort(hint::black_box(&mut rows));
    let elapsed = started.elapsed();
    hint::black_box(rows);
    elapsed
}

fn sample_entries(count: usize) -> Vec<Entry> {
    let extensions = ["txt", "PDF", "jpg", "mp4", "zip", "rs", "docx", "csv"];
    (0..count)
        .map(|index| {
            let random = mix(index as u64);
            let directory = index < count / 8;
            let name = if directory {
                format!("folder-{index:07}")
            } else {
                format!(
                    "project-{index:07}.{}",
                    extensions[random as usize % extensions.len()]
                )
            };
            Entry {
                path: PathBuf::from(&name),
                name: name.into(),
                kind: if directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: (!directory).then_some(random % 100_000_000),
                modified: None,
                created: None,
                accessed: None,
                access: None,
            }
        })
        .collect()
}

fn mix(value: u64) -> u64 {
    let value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
