//! Drives the production updater from a portable install tree, end to end.
//!
//! This is the "run a real upgrade" check from `docs/packaging.md` §4: it is
//! not a unit test, because every interesting part of the flow crosses a
//! process boundary (HTTP fetch, signature check, archive extraction, the
//! detached installer script, the relaunch). It uses the same entry points the
//! app uses — `feed_url`, `check_for_update`, `download_and_install` — so a
//! green run here is evidence about the shipped code, not about a mock.
//!
//! How to run it: build this example, then put the binary in the root of a
//! released portable tree under the name `dae.exe` (the tree must also contain
//! `platforms/`, which is what makes the layout "installed" — see
//! `InstallLayout::current`). Point `DAE_UPDATE_ENDPOINT` at a feed and run it
//! from there. It checks, downloads, verifies, and starts the installer; the
//! installer waits for this process to exit and then replaces the tree and
//! relaunches. The version the relaunched app logs is the proof that the swap
//! happened.
//!
//! `--dry-run` stops after the check, which is what a feed smoke test wants.

use std::io::Write;

use dae_core::update;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let current = args
        .next()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let dry_run = args.any(|arg| arg == "--dry-run");

    let exe = std::env::current_exe().unwrap_or_default();
    println!("exe            {}", exe.display());
    println!("current        {current}");
    println!("feed           {}", update::feed_url());
    println!("updates_enabled {}", update::updates_enabled());

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("could not start the runtime: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let found = match runtime.block_on(update::check_for_update(&update::feed_url(), &current)) {
        Ok(found) => found,
        Err(error) => {
            eprintln!("check failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let Some(available) = found else {
        println!("result         up to date");
        return std::process::ExitCode::SUCCESS;
    };

    println!("available      {}", available.version_text());
    println!("artifact       {}", available.url);
    println!(
        "notes          {}",
        available.notes.lines().next().unwrap_or("")
    );
    if dry_run {
        println!("result         dry run, nothing installed");
        return std::process::ExitCode::SUCCESS;
    }

    // `download_and_install` takes `Fn`, so the dedupe of the progress line
    // lives in an atomic rather than in a captured `mut`.
    let last = std::sync::atomic::AtomicU8::new(255);
    let installed = runtime.block_on(update::download_and_install(&available, |percent| {
        // One line per whole percent, so a 30 MB artifact does not scroll the
        // terminal away while it is being watched.
        if last.swap(percent, std::sync::atomic::Ordering::Relaxed) != percent {
            print!("\rprogress       {percent:>3} %");
            let _ = std::io::stdout().flush();
        }
    }));
    println!();

    match installed {
        Ok(()) => {
            println!("result         installer started; exiting so it can replace the tree");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("install failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
