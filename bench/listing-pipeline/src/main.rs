//! Measures where a large local directory listing spends its time on this host,
//! and what a cheaper enumeration and a cheaper ordering key are worth.
//!
//! Usage:
//!   listing-pipeline prepare <dir> <count>
//!   listing-pipeline measure <dir> [--passes N] [--warmup N] [--variants a,b,c]
//!   listing-pipeline allocs  <dir>
//!
//! Output is TSV, one row per pass plus a median row per variant, so a run can
//! be pasted into a spreadsheet or diffed between builds — the same shape
//! `bench/io-backends` uses.
//!
//! The pipeline being measured is a verbatim mirror of the app's; see
//! `src/pipeline.rs` for the table of what was copied from where, and
//! `README.md` for how the mirror is kept honest.

mod pipeline;
mod windows_find;

use pipeline::{Config, Timings, FIRST_BATCH_SIZE, MAX_BATCH_SIZE};
use std::alloc::{GlobalAlloc, Layout};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// The app's allocator, with counters that are off unless a run asks for them.
///
/// Counting is a relaxed load and a predictable branch per allocation when
/// disabled, which is below the noise floor of a listing that does tens of
/// thousands of them; the timed runs therefore keep this wrapper installed
/// rather than switching allocators between the timing and counting modes.
struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { mimalloc::MiMalloc.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { mimalloc::MiMalloc.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        unsafe { mimalloc::MiMalloc.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The configurations a default run measures, in reporting order: the app's
/// pipeline, the same walk with the comparator, then the Win32 walk on its own
/// and with each ordering, so every millisecond of the difference has an owner.
const DEFAULT_VARIANTS: [&str; 9] = [
    "std",
    "std+rows",
    "std+key",
    "std+cmp",
    "win32",
    "win32-nofetch",
    "win32+rows",
    "win32+key",
    "win32+cmp",
];

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = arguments.first().map(String::as_str) else {
        usage();
        std::process::exit(2);
    };

    let result = match command {
        "prepare" => prepare_command(&arguments[1..]),
        "measure" => measure_command(&arguments[1..]),
        "allocs" => allocs_command(&arguments[1..]),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn usage() {
    eprintln!(
        "usage:\n  \
         listing-pipeline prepare <dir> <count>\n  \
         listing-pipeline measure <dir> [--passes N] [--warmup N] [--variants a,b,c]\n  \
         listing-pipeline allocs  <dir>\n\n\
         variants: {}",
        DEFAULT_VARIANTS.join(", ")
    );
}

fn positional(arguments: &[String], index: usize, what: &str) -> Result<String, String> {
    arguments
        .get(index)
        .cloned()
        .ok_or_else(|| format!("{what} is required"))
}

fn flag(arguments: &[String], name: &str) -> Result<Option<String>, String> {
    let Some(position) = arguments.iter().position(|value| value == name) else {
        return Ok(None);
    };
    arguments
        .get(position + 1)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("{name} needs a value"))
}

// --------------------------------------------------------------------- prepare

/// Writes a dataset that sorts like a real directory: a small set of shared
/// prefixes, so comparisons have to walk past them, mixed with the casing,
/// digits and extensions a download or a source tree actually holds.
///
/// Only the names matter here — the listing cost is the walk, the metadata and
/// the ordering, and an empty file has all three. Empty rather than sized on
/// purpose: `WIN32_FIND_DATAW`/`statx` carry the size without a data read, so a
/// populated file would only slow the fixture down.
fn prepare_command(arguments: &[String]) -> Result<(), String> {
    let directory = PathBuf::from(positional(arguments, 0, "a directory")?);
    let count: usize = positional(arguments, 1, "a file count")?
        .parse()
        .map_err(|error| format!("invalid count: {error}"))?;
    // `mixed` (default) is the realistic fixture; `cjk` is names that are almost
    // all non-ASCII, which is the case the allocation-free comparator's Unicode
    // arm has to be measured on rather than assumed about.
    let style = arguments
        .get(2)
        .map(String::as_str)
        .unwrap_or("mixed")
        .to_owned();
    if style != "mixed" && style != "cjk" {
        return Err(format!("unknown name style `{style}`"));
    }

    fs::create_dir_all(&directory).map_err(|error| format!("create {directory:?}: {error}"))?;

    let started = Instant::now();
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(8);
    let mut created = 0usize;

    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::with_capacity(threads);
        for thread in 0..threads {
            let directory = directory.clone();
            let style = style.clone();
            handles.push(scope.spawn(move || -> Result<usize, String> {
                let mut created = 0;
                let mut index = thread;
                while index < count {
                    let name = fixture_name(index, &style);
                    let path = directory.join(&name);
                    if !path.exists() {
                        fs::File::create(&path)
                            .map_err(|error| format!("create {}: {error}", path.display()))?;
                    }
                    created += 1;
                    index += threads;
                }
                Ok(created)
            }));
        }

        for handle in handles {
            created += handle
                .join()
                .map_err(|_| "a prepare thread panicked".to_owned())??;
        }

        Ok(())
    })?;

    println!(
        "# {} entries ready in {} ({created} visited, {:.1}s)",
        count,
        directory.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// The `index`-th fixture name. Deterministic, so two runs compare like for like.
fn fixture_name(index: usize, style: &str) -> String {
    const PREFIXES: [&str; 40] = [
        "component", "service", "handler", "util", "image", "photo", "report", "chapter",
        "index", "module", "screen", "widget", "store", "client", "server", "schema",
        "fixture", "sample", "export", "import", "render", "layout", "parser", "tokenizer",
        "buffer", "queue", "worker", "config", "manifest", "package", "vendor", "bundle",
        "asset", "sprite", "tile", "frame", "packet", "stream", "cache", "session",
    ];
    const SUFFIXES: [&str; 14] = [
        "ts", "tsx", "js", "json", "md", "png", "jpg", "css", "html", "rs", "toml", "txt", "log",
        "bak",
    ];
    const JOINERS: [char; 3] = ['-', '_', '.'];
    // A handful of non-ASCII names, so the Unicode arm is exercised rather than
    // assumed. Real directories hold some, and the mirror cannot be validated on
    // the case that never runs.
    const NON_ASCII: [&str; 6] = [
        "报告-2024",
        "説明_資料",
        "naïve-résumé",
        "Übersicht",
        "café-notes",
        "数据备份",
    ];

    const CJK_STEMS: [&str; 40] = [
        "报告", "数据", "备份", "文档", "项目", "会议", "记录", "预算", "合同", "发票",
        "设计", "需求", "测试", "部署", "发布", "版本", "配置", "日志", "监控", "告警",
        "用户", "订单", "库存", "财务", "报表", "分析", "调研", "方案", "计划", "总结",
        "培训", "招聘", "绩效", "流程", "制度", "模板", "素材", "图片", "视频", "音频",
    ];
    if style == "cjk" {
        let stem = CJK_STEMS[index % CJK_STEMS.len()];
        let suffix = SUFFIXES[(index / CJK_STEMS.len()) % SUFFIXES.len()];
        let number = (index * 7919) % 100_000;
        return format!("{stem}-{number:05}.{suffix}");
    }

    let prefix = PREFIXES[index % PREFIXES.len()];
    let suffix = SUFFIXES[(index / PREFIXES.len()) % SUFFIXES.len()];
    let joiner = JOINERS[(index / (PREFIXES.len() * SUFFIXES.len())) % JOINERS.len()];
    let number = (index * 7919) % 100_000;

    // Every 37th name is non-ASCII; every 5th is cased away from the prefix's
    // own spelling. Both land often enough to matter in a 35k fixture and rarely
    // enough not to dominate it.
    if index % 37 == 0 {
        let stem = NON_ASCII[(index / 37) % NON_ASCII.len()];
        return format!("{stem}-{number:05}.{suffix}");
    }

    let cased = match index % 5 {
        0 => prefix.to_ascii_uppercase(),
        1 => {
            let mut capitalized = prefix.to_owned();
            capitalized.replace_range(0..1, &prefix[0..1].to_ascii_uppercase());
            capitalized
        }
        _ => prefix.to_owned(),
    };

    format!("{cased}{joiner}{number:05}.{suffix}")
}

// --------------------------------------------------------------------- measure

struct Measurement {
    config: Config,
    timings: Timings,
}

fn measure_command(arguments: &[String]) -> Result<(), String> {
    let path = PathBuf::from(positional(arguments, 0, "a directory")?);
    let passes: usize = flag(arguments, "--passes")?
        .map(|value| value.parse())
        .transpose()
        .map_err(|error| format!("invalid --passes: {error}"))?
        .unwrap_or(7);
    let warmup: usize = flag(arguments, "--warmup")?
        .map(|value| value.parse())
        .transpose()
        .map_err(|error| format!("invalid --warmup: {error}"))?
        .unwrap_or(2);

    let specs: Vec<String> = match flag(arguments, "--variants")? {
        Some(value) => value.split(',').map(str::to_owned).collect(),
        None => DEFAULT_VARIANTS.iter().map(|spec| (*spec).to_owned()).collect(),
    };
    let configs = specs
        .iter()
        .map(|spec| Config::parse(spec).map(|config| (spec.clone(), config)))
        .collect::<Result<Vec<_>, _>>()?;

    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }

    let entries = fs::read_dir(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?
        .count();

    println!(
        "# listing-pipeline: host={} entries={entries} batches={FIRST_BATCH_SIZE},..,{MAX_BATCH_SIZE}",
        std::env::consts::OS
    );
    println!(
        "# serial arm (what Windows takes for every batch): {}",
        pipeline::SERIAL_ARM
    );

    // Parity first: a faster order that is a *different* order is not a result,
    // and neither is a faster walk that reports different rows.
    let reference = pipeline::names_of(&path, Config::parse("std+key")?)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    println!("# rows: {}", reference.len());

    for (spec, config) in &configs {
        if !config.is_pipeline() {
            continue;
        }
        let names = pipeline::names_of(&path, *config)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let verdict = if names == reference {
            "identical".to_owned()
        } else {
            let first = names
                .iter()
                .zip(&reference)
                .position(|(left, right)| left != right);
            format!("DIFFERENT ({} rows, first mismatch at {first:?})", names.len())
        };
        println!("# parity  {spec:<14} vs std+key: {verdict}");
    }

    for _ in 0..warmup {
        for (_, config) in &configs {
            let mut timings = Timings::default();
            pipeline::measure_config(&path, *config, &mut timings)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
        }
    }

    let mut measured: Vec<Vec<Measurement>> = Vec::with_capacity(passes);
    for _ in 0..passes {
        let mut pass = Vec::with_capacity(configs.len());
        for (_, config) in &configs {
            let mut timings = Timings::default();
            pipeline::measure_config(&path, *config, &mut timings)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            pass.push(Measurement {
                config: *config,
                timings,
            });
        }
        measured.push(pass);
    }

    println!("variant\tpass\twalk_ms\trows_ms\tsort_ms\ttotal_ms\tentries\tentries_per_s");
    for (ordinal, pass) in measured.iter().enumerate() {
        for measurement in pass {
            print_row(&measurement.config.label(), ordinal + 1, &measurement.timings);
        }
    }

    println!();
    println!("median of {passes} passes");
    println!("variant\twalk_ms\trows_ms\tsort_ms\ttotal_ms\tentries_per_s");
    let mut medians = Vec::with_capacity(configs.len());
    for (index, (spec, config)) in configs.iter().enumerate() {
        let mut walk = Vec::new();
        let mut rows = Vec::new();
        let mut sort = Vec::new();
        let mut total = Vec::new();
        let mut count = 0u64;

        for pass in &measured {
            let timings = pass[index].timings;
            walk.push(timings.walk_ns);
            rows.push(timings.rows_ns);
            sort.push(timings.sort_ns);
            total.push(timings.total_ns());
            count = timings.entries;
        }

        let walk = median(&mut walk);
        let rows = median(&mut rows);
        let sort = median(&mut sort);
        let total = median(&mut total);

        println!(
            "{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.0}",
            config.label(),
            walk as f64 / 1e6,
            rows as f64 / 1e6,
            sort as f64 / 1e6,
            total as f64 / 1e6,
            count as f64 / (total as f64 / 1e9)
        );
        medians.push((spec.clone(), total));
    }

    report(&configs, &medians);
    Ok(())
}

/// The headline: what each change is worth against the app's pipeline, with the
/// walk and the ordering attributed separately.
fn report(configs: &[(String, Config)], medians: &[(String, u128)]) {
    let get = |spec: &str| -> Option<u128> {
        configs
            .iter()
            .position(|(candidate, _)| candidate == spec)
            .map(|index| medians[index].1)
    };

    let ms = |value: u128| value as f64 / 1e6;

    let Some(app) = get("std+key") else {
        return;
    };

    println!();
    println!("app pipeline (std walk, app sort key)        {:>8.3} ms", ms(app));

    let mut rows = Vec::new();
    for (spec, config) in configs {
        if *spec == "std+key" {
            continue;
        }
        let Some(total) = get(spec) else {
            continue;
        };
        rows.push((config.label(), total));
    }

    for (label, total) in &rows {
        let delta = *total as f64 - app as f64;
        println!(
            "{label:<44} {:.3} ms  ({:+.1}%, {:+.3} ms vs app)",
            ms(*total),
            delta / app as f64 * 100.0,
            -delta / 1e6
        );
    }

    // The two attributions that matter: how much of the app's pipeline is the
    // walk, and how much is the ordering.
    if let Some(rows_only) = get("std+rows") {
        println!();
        println!(
            "std walk's share of the app pipeline         {:.3} ms of {:.3} ms ({:.0}%)",
            ms(rows_only),
            ms(app),
            rows_only as f64 / app as f64 * 100.0
        );
        println!(
            "app ordering (key)                           {:.3} ms ({:.0}%)",
            ms(app - rows_only),
            (app - rows_only) as f64 / app as f64 * 100.0
        );
    }
    if let Some(cmp) = get("std+cmp") {
        println!(
            "app ordering (comparator)                    {:.3} ms ({:.0}%)  -> {:+.3} ms",
            ms(cmp.saturating_sub(get("std+rows").unwrap_or(0))),
            cmp.saturating_sub(get("std+rows").unwrap_or(0)) as f64 / app as f64 * 100.0,
            -(app as f64 - cmp as f64) / 1e6
        );
    }
}

fn print_row(label: &str, pass: usize, timings: &Timings) {
    let seconds = timings.total_ns() as f64 / 1e9;
    println!(
        "{label}\t{pass}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{:.0}",
        timings.walk_ns as f64 / 1e6,
        timings.rows_ns as f64 / 1e6,
        timings.sort_ns as f64 / 1e6,
        timings.total_ns() as f64 / 1e6,
        timings.entries,
        timings.entries as f64 / seconds
    );
}

/// Median of a set of per-pass values — for an even count, the lower middle, so
/// the reported number is always one pass that actually happened.
fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

// ---------------------------------------------------------------------- allocs

/// Counts the allocations one listing makes under each configuration. Timing
/// says a variant is faster; this says why, and it is the number that does not
/// depend on how loaded the machine is.
fn allocs_command(arguments: &[String]) -> Result<(), String> {
    let path = PathBuf::from(positional(arguments, 0, "a directory")?);
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }

    let specs = ["std+rows", "win32+rows", "std+key", "std+cmp", "win32+key", "win32+cmp"];

    // Warm the directory metadata and the allocator's own arenas up, so the
    // counted pass is not paying for first touch, and so every variant sees the
    // same starting state.
    for _ in 0..2 {
        for spec in specs {
            let mut timings = Timings::default();
            pipeline::measure_config(&path, Config::parse(spec)?, &mut timings)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
        }
    }

    println!("variant\tallocations\tbytes\tentries\tallocs_per_entry");
    for spec in specs {
        let config = Config::parse(spec)?;
        ALLOCATIONS.store(0, Ordering::Relaxed);
        ALLOCATED_BYTES.store(0, Ordering::Relaxed);
        COUNTING.store(true, Ordering::Relaxed);

        let mut timings = Timings::default();
        let outcome = pipeline::measure_config(&path, config, &mut timings);

        COUNTING.store(false, Ordering::Relaxed);
        outcome.map_err(|error| format!("read {}: {error}", path.display()))?;

        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        let bytes = ALLOCATED_BYTES.load(Ordering::Relaxed);
        println!(
            "{}\t{allocations}\t{bytes}\t{}\t{:.2}",
            config.label(),
            timings.entries,
            allocations as f64 / timings.entries.max(1) as f64
        );
    }

    Ok(())
}
