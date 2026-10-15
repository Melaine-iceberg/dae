//! Measures the same file-manager workloads through two dispatch mechanisms:
//! blocking calls on a thread pool (what the app does today — Tokio's
//! `spawn_blocking` pool, and the dedicated rayon transfer pool) versus a
//! completion-based runtime (compio: IOCP on Windows, io_uring on Linux).
//!
//! Usage:
//!   io-backends prepare all [--root DIR] [--big-mib N]
//!   io-backends run <scenario> [--backend tokio,rayon,compio] [--threads 1,4,16]
//!                              [--passes N] [--warmup N] [--root DIR] [--json]
//!   io-backends list
//!
//! Output is TSV, one row per pass plus a median row per configuration, so a
//! run can be pasted into a spreadsheet or diffed between builds.

mod cpu;
mod ops;
mod rng;
mod scenarios;

use crate::cpu::CpuTime;
use crate::ops::{CompioPool, Op, SyncPool};
use crate::scenarios::Workload;
use rayon::prelude::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The app installs mimalloc as its global allocator (`src-tauri/src/main.rs`,
/// where the ~31% listing win is documented). Measuring the I/O mechanism under
/// the system heap would mix two changes together, so the benchmark runs under
/// the same allocator the app ships.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    /// One `tokio::task::spawn_blocking` per operation — what a Tauri command
    /// does today, since `file_system::commands` wraps each piece of work in
    /// its own call and the frontend fires many of them at once.
    Tokio,
    /// One `spawn_blocking` per *batch* of operations, with `threads` batches
    /// running at once — the shape `read_directory`, `stat_named_children` and
    /// the copy path actually have: one blocking call that loops.
    TokioChunked,
    /// A dedicated rayon pool with blocking calls — the `transfer_pool` shape.
    Rayon,
    /// compio: IOCP with `OVERLAPPED` file I/O on Windows, io_uring on Linux.
    Compio,
}

impl Backend {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "tokio" => Ok(Self::Tokio),
            "tokio-chunked" => Ok(Self::TokioChunked),
            "rayon" => Ok(Self::Rayon),
            "compio" => Ok(Self::Compio),
            other => Err(format!("unknown backend `{other}`")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Tokio => "tokio",
            Self::TokioChunked => "tokio-chunked",
            Self::Rayon => "rayon",
            Self::Compio => "compio",
        }
    }
}

#[derive(Clone)]
struct Config {
    root: PathBuf,
    /// Size of the cache-resident blob. Only ever lowered with `--blob-mib`, to
    /// dry-run the harness without writing 8 GiB first.
    blob_mib: u64,
    big_mib: u64,
    backends: Vec<Backend>,
    threads: Vec<usize>,
    /// Operations one `tokio` blocking call carries. `1` is what a Tauri
    /// command does today (one command, one file); larger values are the
    /// "read a batch of files in one call" shape. `tokio-chunked` is the same
    /// idea taken to its limit (one call per thread) and ignores this.
    batch: usize,
    passes: usize,
    warmup: usize,
    json: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: default_root(),
            blob_mib: scenarios::BLOB_MIB,
            big_mib: scenarios::BIG_BLOB_MIB_DEFAULT,
            backends: vec![
                Backend::Tokio,
                Backend::TokioChunked,
                Backend::Rayon,
                Backend::Compio,
            ],
            threads: vec![1, 4, 16],
            batch: 1,
            passes: 3,
            warmup: 1,
            json: false,
        }
    }
}

fn default_root() -> PathBuf {
    if let Ok(value) = std::env::var("DAE_BENCH_ROOT")
        && !value.is_empty()
    {
        return PathBuf::from(value);
    }
    if cfg!(windows) {
        // The measured volume (ReFS on an NVMe SSD). Deliberately outside the
        // repository: a full sweep writes ~57 GiB.
        PathBuf::from("K:/dae-io-bench")
    } else {
        PathBuf::from("/tmp/dae-io-bench")
    }
}

struct Outcome {
    wall: Duration,
    units: u64,
    cpu: Option<CpuTime>,
}

fn main() {
    if let Err(error) = run_cli() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run_cli() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
        return Ok(());
    }

    match args[0].as_str() {
        "list" => {
            for name in scenarios::NAMES {
                println!("{name}\t{}", scenarios::unit_label(name));
            }
            Ok(())
        }
        "prepare" => {
            let name = args.get(1).cloned().unwrap_or_else(|| "all".to_owned());
            let config = parse_flags(&args[2.min(args.len())..])?;
            scenarios::prepare(&name, &config.root, config.blob_mib, config.big_mib)
        }
        "run" => {
            let name = args
                .get(1)
                .cloned()
                .ok_or_else(|| "run needs a scenario name".to_owned())?;
            let config = parse_flags(&args[2.min(args.len())..])?;
            run(&name, &config)
        }
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn usage() {
    println!(
        "io-backends — blocking thread pool vs completion I/O, on the app's own workflows\n\
         \n\
         commands:\n  \
           list                                        scenarios and their throughput unit\n  \
           prepare <scenario|all> [flags]              create the datasets (idempotent)\n  \
           run <scenario> [flags]                      measure\n\
         \n\
         flags:\n  \
           --root DIR        dataset root (default: {}, or $DAE_BENCH_ROOT)\n  \
           --blob-mib N      size of the cache-resident blob (default: 8192 MiB; lower it to dry-run)\n  \
           --big-mib N       size of the cache-defeating blob (default: {})\n  \
           --backend LIST    tokio,rayon,compio (default: all three)\n  \
           --threads LIST    concurrency levels (default: 1,4,16)\n  \
           --batch N         operations one blocking call carries (default: 1, = one per\n  \
                             Tauri command; `tokio-chunked` is this at its limit)\n  \
           --passes N        measured passes per configuration (default: 3)\n  \
           --warmup N        unmeasured passes first (default: 1; use 0 for cold workloads)\n  \
           --json            emit JSON lines instead of TSV\n\
         \n\
         Copy semantics for the completion backend are selected by DAE_BENCH_COPY_MODE:\n  \
           kernel (default)  keep CopyFileExW, run it on the blocking pool\n  \
           readwrite         read into user space and write back out, on the driver",
        default_root().display(),
        scenarios::BIG_BLOB_MIB_DEFAULT
    );
}

fn parse_flags(args: &[String]) -> Result<Config, String> {
    let mut config = Config::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let mut value = || -> Result<String, String> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| format!("`{flag}` needs a value"))
        };
        match flag {
            "--root" => config.root = PathBuf::from(value()?),
            "--blob-mib" => {
                config.blob_mib = value()?
                    .parse()
                    .map_err(|error| format!("--blob-mib: {error}"))?
            }
            "--big-mib" => {
                config.big_mib = value()?
                    .parse()
                    .map_err(|error| format!("--big-mib: {error}"))?
            }
            "--batch" => {
                config.batch = value()?
                    .parse::<usize>()
                    .map_err(|error| format!("--batch: {error}"))?
                    .max(1)
            }
            "--passes" => {
                config.passes = value()?
                    .parse()
                    .map_err(|error| format!("--passes: {error}"))?
            }
            "--warmup" => {
                config.warmup = value()?
                    .parse()
                    .map_err(|error| format!("--warmup: {error}"))?
            }
            "--backend" => {
                config.backends = value()?
                    .split(',')
                    .map(Backend::parse)
                    .collect::<Result<Vec<_>, _>>()?
            }
            "--threads" => {
                config.threads = value()?
                    .split(',')
                    .map(|part| {
                        part.parse::<usize>()
                            .map_err(|error| format!("--threads: {error}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            "--json" => config.json = true,
            other => return Err(format!("unknown flag `{other}`")),
        }
        index += 1;
    }
    Ok(config)
}

/// One measured row, kept so the comparison summary can be printed at the end.
struct Row {
    scenario: String,
    backend: &'static str,
    threads: usize,
    wall_ms: f64,
    hero: f64,
    ops_per_s: f64,
    cpu_ms: Option<f64>,
    cpu_pct: Option<f64>,
}

fn run(scenario: &str, config: &Config) -> Result<(), String> {
    if !scenarios::NAMES.contains(&scenario) {
        return Err(format!("unknown scenario `{scenario}`"));
    }

    let unit = scenarios::unit_label(scenario);
    // Which driver the completion backend actually bound to. On Windows that is
    // IOCP. On Linux it is io_uring unless the kernel refuses it, in which case
    // compio falls back to `polling` (epoll) — a fallback reported as "io_uring"
    // would be a lie, so the driver is printed rather than assumed.
    if config.backends.contains(&Backend::Compio)
        && let Ok(probe) = compio::runtime::Runtime::new()
    {
        if config.json {
            println!(
                "{{\"kind\":\"driver\",\"driver\":\"{:?}\"}}",
                probe.driver_type()
            );
        } else {
            println!("# compio driver: {:?}", probe.driver_type());
        }
    }
    if config.json {
        println!(
            "{{\"kind\":\"header\",\"scenario\":\"{scenario}\",\"unit\":\"{unit}\",\"root\":\"{}\",\"batch\":{}}}",
            config.root.display(),
            config.batch
        );
    } else {
        println!(
            "# scenario={scenario} unit={unit} root={} passes={} warmup={} batch={}",
            config.root.display(),
            config.passes,
            config.warmup,
            config.batch
        );
        println!(
            "scenario\tbackend\tthreads\tpass\twall_ms\tunit_per_s\tops_per_s\tcpu_ms\tcpu_pct"
        );
    }

    let mut rows: Vec<Row> = Vec::new();
    // Distinguishes each pass's output files, and re-seeds the random offsets so
    // two cold passes never read the same slice twice.
    let mut run_id = 0_u64;

    for backend in &config.backends {
        for &threads in &config.threads {
            // Runtimes and pools are built once per configuration: creating a
            // Tokio runtime costs milliseconds and would otherwise land inside
            // the measurement.
            let mut machinery = Machinery::new(*backend, threads, config.batch, scenario)?;

            for _ in 0..config.warmup {
                let workload = scenarios::build(
                    scenario,
                    &config.root,
                    config.blob_mib,
                    config.big_mib,
                    run_id,
                )?;
                run_id += 1;
                scenarios::reset(&workload);
                machinery.measure(&workload, threads)?;
                if !config.json {
                    println!(
                        "{scenario}\t{}\t{threads}\twarm\t-\t-\t-\t-\t-",
                        backend.name()
                    );
                }
            }

            let mut measured: Vec<Row> = Vec::new();
            for pass in 1..=config.passes {
                let workload = scenarios::build(
                    scenario,
                    &config.root,
                    config.blob_mib,
                    config.big_mib,
                    run_id,
                )?;
                run_id += 1;
                scenarios::reset(&workload);
                let outcome = machinery.measure(&workload, threads)?;

                let wall_ms = outcome.wall.as_secs_f64() * 1000.0;
                let ops = workload.items.len() as f64;
                // Below the clock tick's resolution the process CPU delta is
                // noise; see `cpu.rs`.
                let cpu = outcome
                    .cpu
                    .filter(|_| cpu::worth_reporting(wall_ms, threads));
                let hero = hero_rate(
                    scenario,
                    outcome.units,
                    workload.items.len(),
                    workload.nominal_bytes,
                    outcome.wall,
                );
                let row = Row {
                    scenario: scenario.to_owned(),
                    backend: backend.name(),
                    threads,
                    wall_ms,
                    hero,
                    ops_per_s: ops / outcome.wall.as_secs_f64(),
                    cpu_ms: cpu.map(|cpu| cpu.total_ms()),
                    cpu_pct: cpu.map(|cpu| cpu.total_ms() / wall_ms * 100.0),
                };
                report(&row, pass, config.json);
                measured.push(row);
            }

            let median = median_row(&measured);
            report(&median, 0, config.json);
            rows.extend(measured);
            rows.push(median);
        }
    }

    if !config.json {
        print_comparison(&rows, unit, &config.backends);
    }
    Ok(())
}

/// The scenario's headline rate: entries per second for a listing, MiB/s
/// otherwise.
fn hero_rate(scenario: &str, units: u64, ops: usize, nominal_bytes: u64, wall: Duration) -> f64 {
    let seconds = wall.as_secs_f64().max(1e-9);
    match scenarios::hero(scenario) {
        scenarios::Hero::Bytes => (nominal_bytes as f64 / (1024.0 * 1024.0)) / seconds,
        scenarios::Hero::Ops => (ops as f64 / seconds) / 1000.0,
        scenarios::Hero::Entries => units as f64 / seconds,
    }
}

fn report(row: &Row, pass: usize, json: bool) {
    let label = if pass == 0 {
        "median".to_owned()
    } else {
        pass.to_string()
    };
    if json {
        println!(
            "{{\"kind\":\"pass\",\"scenario\":\"{}\",\"backend\":\"{}\",\"threads\":{},\"pass\":\"{}\",\"wall_ms\":{:.2},\"unit_per_s\":{:.1},\"ops_per_s\":{:.1},\"cpu_ms\":{},\"cpu_pct\":{}}}",
            row.scenario,
            row.backend,
            row.threads,
            label,
            row.wall_ms,
            row.hero,
            row.ops_per_s,
            row.cpu_ms
                .map_or("null".to_owned(), |value| format!("{value:.1}")),
            row.cpu_pct
                .map_or("null".to_owned(), |value| format!("{value:.1}")),
        );
    } else {
        println!(
            "{}\t{}\t{}\t{}\t{:.2}\t{:.1}\t{:.1}\t{}\t{}",
            row.scenario,
            row.backend,
            row.threads,
            label,
            row.wall_ms,
            row.hero,
            row.ops_per_s,
            row.cpu_ms
                .map_or("-".to_owned(), |value| format!("{value:.1}")),
            row.cpu_pct
                .map_or("-".to_owned(), |value| format!("{value:.1}")),
        );
    }
}

/// The pass whose wall time sits in the middle, reported as it ran.
///
/// Blending fields from different passes would produce a row no run ever
/// produced — a median wall time next to a best-case rate — so the medians
/// printed here are one real pass.
fn median_row(rows: &[Row]) -> Row {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|left, right| rows[*left].wall_ms.total_cmp(&rows[*right].wall_ms));
    let middle = order[order.len() / 2];

    Row {
        scenario: rows[middle].scenario.clone(),
        backend: rows[middle].backend,
        threads: rows[middle].threads,
        wall_ms: rows[middle].wall_ms,
        hero: rows[middle].hero,
        ops_per_s: rows[middle].ops_per_s,
        cpu_ms: rows[middle].cpu_ms,
        cpu_pct: rows[middle].cpu_pct,
    }
}

/// Per-scenario, per-concurrency comparison of the median rows — the table the
/// report is written from.
fn print_comparison(rows: &[Row], unit: &str, backends: &[Backend]) {
    let mut keys: Vec<(String, usize)> = Vec::new();
    for row in rows {
        let key = (row.scenario.clone(), row.threads);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }

    let order: Vec<&'static str> = backends.iter().map(|backend| backend.name()).collect();

    println!(
        "\n# medians ({unit}; ratio = tokio / compio, so > 1 means the thread pool was faster)"
    );
    let header: Vec<String> = order
        .iter()
        .map(|backend| format!("{backend}\t{backend}_cpu_ms"))
        .collect();
    println!("scenario\tthreads\t{}\ttokio/compio", header.join("\t"));

    for (scenario, threads) in keys {
        let hero_of = |backend: &str| -> Option<f64> {
            rows.iter()
                .find(|row| {
                    row.scenario == scenario && row.threads == threads && row.backend == backend
                })
                .map(|row| row.hero)
        };

        let mut cells: Vec<String> = Vec::new();
        for backend in &order {
            match rows.iter().find(|row| {
                row.scenario == scenario && row.threads == threads && row.backend == *backend
            }) {
                Some(row) => {
                    cells.push(format!("{:.1}", row.hero));
                    cells.push(row.cpu_ms.map_or("-".to_owned(), |cpu| format!("{cpu:.0}")));
                }
                None => {
                    cells.push("-".to_owned());
                    cells.push("-".to_owned());
                }
            }
        }

        let ratio = hero_of("tokio")
            .zip(hero_of("compio"))
            .map(|(pool, completion)| format!("{:.2}", pool / completion))
            .unwrap_or_else(|| "-".to_owned());
        println!("{scenario}\t{threads}\t{}\t{ratio}", cells.join("\t"));
    }
}

/// Whatever the backend needs to exist before the clock starts.
enum Machinery {
    Tokio {
        runtime: tokio::runtime::Runtime,
        pool: Arc<SyncPool>,
        /// One blocking call per thread instead of per operation.
        chunked: bool,
        /// Operations per blocking call when `chunked` is false.
        batch: usize,
    },
    Rayon(rayon::ThreadPool, Arc<SyncPool>),
    Compio(compio::runtime::Runtime, Arc<CompioPool>),
}

impl Machinery {
    fn new(backend: Backend, threads: usize, batch: usize, scenario: &str) -> Result<Self, String> {
        let chunked = backend == Backend::TokioChunked;
        // The op list depends on the pass, but the handle pool does not: the
        // workloads that need one always use the same file. Building pass 0 here
        // would be wasteful, so the pool is opened lazily inside `measure` and
        // then reused.
        let _ = scenario;
        match backend {
            Backend::Tokio | Backend::TokioChunked => {
                // `spawn_blocking` tasks run on the blocking pool, not on the
                // worker threads; two workers is enough to drive them and keeps
                // the runtime's own overhead out of the measurement.
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("bench-tokio")
                    .build()
                    .map_err(|error| format!("tokio runtime: {error}"))?;
                Ok(Self::Tokio {
                    runtime,
                    pool: Arc::new(SyncPool::default()),
                    chunked,
                    batch,
                })
            }
            Backend::Rayon => {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|index| format!("bench-rayon-{index}"))
                    .build()
                    .map_err(|error| format!("rayon pool: {error}"))?;
                Ok(Self::Rayon(pool, Arc::new(SyncPool::default())))
            }
            Backend::Compio => {
                let runtime = compio::runtime::Runtime::new()
                    .map_err(|error| format!("compio runtime: {error}"))?;
                let pool = runtime
                    .block_on(CompioPool::open(&[]))
                    .map_err(|error| format!("compio pool: {error}"))?;
                Ok(Self::Compio(runtime, Arc::new(pool)))
            }
        }
    }

    fn measure(&mut self, workload: &Workload, threads: usize) -> Result<Outcome, String> {
        match self {
            Self::Tokio {
                runtime,
                pool,
                chunked,
                batch,
            } => {
                let pool = open_sync(pool, workload)?;
                let before = cpu::process_cpu();
                let started = Instant::now();
                let units = if *chunked {
                    runtime
                        .block_on(tokio_chunked(&workload.items, threads, pool))
                        .map_err(|error| format!("tokio: {error}"))?
                } else {
                    runtime
                        .block_on(tokio_window(&workload.items, threads, *batch, pool))
                        .map_err(|error| format!("tokio: {error}"))?
                };
                Ok(finish(started, units, before))
            }
            Self::Rayon(pool, sync_pool) => {
                let sync_pool = open_sync(sync_pool, workload)?;
                let before = cpu::process_cpu();
                let started = Instant::now();
                let units: u64 = pool
                    .install(|| {
                        workload
                            .items
                            .par_iter()
                            .map(|op| ops::run_sync(&sync_pool, op))
                            .try_reduce(|| 0_u64, |left, right| Ok(left + right))
                    })
                    .map_err(|error| format!("rayon: {error}"))?;
                Ok(finish(started, units, before))
            }
            Self::Compio(runtime, pool) => {
                let pool = if workload.handles.is_empty() {
                    // Reuse the empty pool: `Arc::clone` keeps the handles alive
                    // for the pass without reopening them.
                    Arc::clone(pool)
                } else {
                    Arc::new(
                        runtime
                            .block_on(CompioPool::open(&workload.handles))
                            .map_err(|error| format!("compio handles: {error}"))?,
                    )
                };
                let before = cpu::process_cpu();
                let started = Instant::now();
                let units = runtime
                    .block_on(compio_window(runtime, &workload.items, threads, pool))
                    .map_err(|error| format!("compio: {error}"))?;
                Ok(finish(started, units, before))
            }
        }
    }
}

fn finish(started: Instant, units: u64, before: Option<CpuTime>) -> Outcome {
    let wall = started.elapsed();
    let after = cpu::process_cpu();
    Outcome {
        wall,
        units,
        cpu: before.zip(after).map(|(before, after)| after.since(before)),
    }
}

fn open_sync(shared: &Arc<SyncPool>, workload: &Workload) -> Result<Arc<SyncPool>, String> {
    if workload.handles.is_empty() {
        return Ok(Arc::clone(shared));
    }
    SyncPool::open(&workload.handles)
        .map(Arc::new)
        .map_err(|error| format!("open handles: {error}"))
}

/// Keeps exactly `threads` operations in flight: a new one is started as soon
/// as the oldest finishes. Both backends use this same rolling window, so the
/// outstanding-request count is identical — which is the number a completion
/// driver is supposed to be able to raise and a thread pool is not.
async fn tokio_window(
    items: &[Op],
    threads: usize,
    batch: usize,
    pool: Arc<SyncPool>,
) -> Result<u64, String> {
    // One blocking call carries up to `batch` operations, so `batch == 1` is
    // the per-command shape and `batch == len` is `tokio-chunked`'s shape.
    // Grouping up front keeps the window itself identical at every batch size.
    let groups: Vec<Vec<Op>> = items.chunks(batch.max(1)).map(<[Op]>::to_vec).collect();

    let mut window: VecDeque<tokio::task::JoinHandle<std::io::Result<u64>>> = VecDeque::new();
    let mut queue = groups.into_iter();
    let mut units = 0_u64;

    loop {
        while window.len() < threads {
            let Some(group) = queue.next() else { break };
            let pool = Arc::clone(&pool);
            window.push_back(tokio::task::spawn_blocking(move || {
                let mut units = 0_u64;
                for op in &group {
                    units += ops::run_sync(&pool, op)?;
                }
                Ok(units)
            }));
        }
        let Some(handle) = window.pop_front() else {
            break;
        };
        units += handle
            .await
            .map_err(|error| format!("blocking task: {error}"))?
            .map_err(|error| format!("io: {error}"))?;
    }

    Ok(units)
}

/// Runs the work as `threads` blocking calls, each looping over a contiguous
/// slice — the shape a single command has when it lists a directory or copies a
/// tree, and the shape that keeps `spawn_blocking`'s per-call cost off the
/// per-operation path.
async fn tokio_chunked(items: &[Op], threads: usize, pool: Arc<SyncPool>) -> Result<u64, String> {
    let per_chunk = items.len().div_ceil(threads.max(1)).max(1);
    let mut handles = Vec::new();

    for chunk in items.chunks(per_chunk) {
        let chunk = chunk.to_vec();
        let pool = Arc::clone(&pool);
        handles.push(tokio::task::spawn_blocking(move || {
            let mut units = 0_u64;
            for op in &chunk {
                units += ops::run_sync(&pool, op)?;
            }
            Ok::<u64, std::io::Error>(units)
        }));
    }

    let mut units = 0_u64;
    for handle in handles {
        units += handle
            .await
            .map_err(|error| format!("blocking task: {error}"))?
            .map_err(|error| format!("io: {error}"))?;
    }
    Ok(units)
}

async fn compio_window(
    runtime: &compio::runtime::Runtime,
    items: &[Op],
    threads: usize,
    pool: Arc<CompioPool>,
) -> Result<u64, String> {
    let mut window: VecDeque<compio::runtime::JoinHandle<std::io::Result<u64>>> = VecDeque::new();
    let mut queue = items.iter();
    let mut units = 0_u64;

    loop {
        while window.len() < threads {
            let Some(op) = queue.next() else { break };
            let op = op.clone();
            let pool = Arc::clone(&pool);
            window.push_back(runtime.spawn(ops::run_compio(pool, op)));
        }
        let Some(handle) = window.pop_front() else {
            break;
        };
        units += handle
            .await
            .map_err(|error| format!("compio task: {error}"))?
            .map_err(|error| format!("io: {error}"))?;
    }

    Ok(units)
}
