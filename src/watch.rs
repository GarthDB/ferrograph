//! File watcher for incremental re-indexing.
//!
//! Watches a project root for changes and re-runs the pipeline when files change.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::graph::Store;
use crate::pipeline::{run_pipeline, PipelineConfig};

/// Debounce interval before re-indexing after a change (seconds).
const DEBOUNCE_SECS: u64 = 2;

/// Run the pipeline once, then watch for filesystem changes and re-run (with debounce).
/// Exits gracefully on Ctrl+C.
///
/// # Errors
/// Fails if the store cannot be opened, the initial pipeline run fails, or the watcher cannot be created.
pub fn watch_and_reindex(store: &Store, root: &Path, config: &PipelineConfig) -> Result<()> {
    run_pipeline(store, root, config)?;
    println!(
        "Watching {} (re-index in {}s after changes). Ctrl+C to stop.",
        root.display(),
        DEBOUNCE_SECS
    );

    let running = std::sync::Arc::new(AtomicBool::new(true));
    let running_clone = std::sync::Arc::clone(&running);
    ctrlc::set_handler(move || {
        running_clone.store(false, Ordering::Relaxed);
    })?;

    let dirty = std::sync::Arc::new(AtomicBool::new(false));
    let dirty_clone = std::sync::Arc::clone(&dirty);
    let root_path = root.to_path_buf();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                if matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    let has_rs = ev
                        .paths
                        .iter()
                        .any(|p| p.extension().is_some_and(|e| e == "rs"));
                    if has_rs {
                        dirty_clone.store(true, Ordering::Relaxed);
                    }
                }
            }
        },
        Config::default(),
    )?;

    watcher.watch(root, RecursiveMode::Recursive)?;

    while running.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_secs(DEBOUNCE_SECS));
        if !running.load(Ordering::Relaxed) {
            break;
        }
        if dirty.load(Ordering::Relaxed) {
            dirty.store(false, Ordering::Relaxed);
            match store
                .clear()
                .and_then(|()| run_pipeline(store, &root_path, config))
            {
                Ok(()) => println!("Re-indexed {}", root_path.display()),
                Err(e) => eprintln!("Re-index failed: {e:#}"),
            }
        }
    }
    println!("Stopped watching.");
    Ok(())
}
