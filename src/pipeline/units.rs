//! Which package and Cargo target a file compiles in, and which of them can call which (#177).
//!
//! A call edge from a test file to a private method of another package, or from a crate to one it
//! does not depend on, cannot exist; resolution by name alone would still create it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use super::modules::crate_dir;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Target {
    Lib,
    Bin,
    Test,
    Example,
    Bench,
    Build,
}

/// Names of a package's dependencies, by Cargo section.
#[derive(Default)]
struct Package {
    deps: Vec<String>,
    dev: Vec<String>,
    build: Vec<String>,
    has_lib: bool,
}

pub(crate) struct Units {
    root: PathBuf,
    pkgs: HashMap<PathBuf, Package>,
    by_name: HashMap<String, PathBuf>,
    files: RefCell<HashMap<String, Option<(PathBuf, Target)>>>,
    reach: RefCell<HashMap<(PathBuf, Target), HashSet<PathBuf>>>,
}

fn norm(name: &str) -> String {
    name.replace('-', "_")
}

/// Dependency names declared in `table` and its `[target.*]` variants, keyed by the package they
/// resolve to (`package = ".."` renames).
fn dep_names(
    manifest: &toml::Value,
    section: &str,
    ws_renames: &HashMap<String, String>,
) -> Vec<String> {
    let tables = std::iter::once(manifest.get(section)).chain(
        manifest
            .get("target")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|t| t.values().map(|v| v.get(section))),
    );
    tables
        .flatten()
        .filter_map(toml::Value::as_table)
        .flat_map(|t| {
            t.iter().map(|(k, v)| {
                // `foo.workspace = true` inherits the rename from the root's `[workspace.dependencies]`.
                let renamed = v
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .or_else(|| ws_renames.get(k).map(String::as_str));
                norm(renamed.unwrap_or(k))
            })
        })
        .collect()
}

impl Units {
    pub(crate) fn load(root: &Path) -> Self {
        let mut manifests = Vec::new();
        let mut ws_renames = HashMap::new();
        for entry in WalkBuilder::new(root).build().flatten() {
            let path = entry.path();
            if path.file_name().is_none_or(|n| n != "Cargo.toml") {
                continue;
            }
            let Some(dir) = path.parent() else { continue };
            let Some(manifest) = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| t.parse::<toml::Value>().ok())
            else {
                continue;
            };
            let shared = manifest
                .get("workspace")
                .and_then(|w| w.get("dependencies"))
                .and_then(toml::Value::as_table);
            for (k, v) in shared.into_iter().flatten() {
                if let Some(real) = v.get("package").and_then(toml::Value::as_str) {
                    ws_renames.insert(k.clone(), real.to_string());
                }
            }
            manifests.push((dir.to_path_buf(), manifest));
        }
        let mut pkgs = HashMap::new();
        let mut by_name = HashMap::new();
        for (dir, manifest) in manifests {
            // A workspace-only manifest has no `[package]`.
            let Some(name) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
            else {
                continue;
            };
            let has_lib = dir.join("src/lib.rs").is_file() || manifest.get("lib").is_some();
            by_name.insert(norm(name), dir.clone());
            pkgs.insert(
                dir,
                Package {
                    deps: dep_names(&manifest, "dependencies", &ws_renames),
                    dev: dep_names(&manifest, "dev-dependencies", &ws_renames),
                    build: dep_names(&manifest, "build-dependencies", &ws_renames),
                    has_lib,
                },
            );
        }
        Self {
            root: root.to_path_buf(),
            pkgs,
            by_name,
            files: RefCell::default(),
            reach: RefCell::default(),
        }
    }

    /// Package directory and target of a file id path (`./crates/a/src/x.rs`); `None` when it is
    /// outside any package or in an unusual layout.
    fn unit_of(&self, file: &str) -> Option<(PathBuf, Target)> {
        if let Some(hit) = self.files.borrow().get(file) {
            return hit.clone();
        }
        let path = self.root.join(file.trim_start_matches("./"));
        let dir = crate_dir(&self.root, &path);
        let unit = self.pkgs.get(&dir).and_then(|pkg| {
            let rel = path.strip_prefix(&dir).ok()?;
            let first = rel.components().next()?.as_os_str().to_str()?;
            let target = match first {
                "tests" => Target::Test,
                "examples" => Target::Example,
                "benches" => Target::Bench,
                "build.rs" | "build" => Target::Build,
                // shortcut: `mod`s of src/main.rs in a package with a lib count as lib files, and lib/bin files may call each other; split by `mod` declarations to fix.
                "src" if rel == Path::new("src/main.rs") || rel.starts_with("src/bin") => {
                    Target::Bin
                }
                "src" if pkg.has_lib => Target::Lib,
                "src" => Target::Bin,
                _ => return None,
            };
            Some((dir.clone(), target))
        });
        self.files
            .borrow_mut()
            .insert(file.to_string(), unit.clone());
        unit
    }

    /// Packages whose library code a unit of `target` in `pkg` can call: its direct dependencies
    /// (plus dev-dependencies, or build-dependencies for `build.rs`) and,
    /// through re-exports, their normal dependencies.
    fn reachable(&self, pkg: &Path, target: Target) -> HashSet<PathBuf> {
        let key = (pkg.to_path_buf(), target);
        if let Some(hit) = self.reach.borrow().get(&key) {
            return hit.clone();
        }
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut queue: Vec<&str> = Vec::new();
        if let Some(p) = self.pkgs.get(pkg) {
            let direct = match target {
                Target::Build => &p.build[..],
                _ => &p.deps[..],
            };
            queue.extend(direct.iter().map(String::as_str));
            // `#[cfg(test)]` modules in `src/` may use dev-dependencies too, and files are not told
            // apart from them, so lib/bin code is allowed them (over-allows, never drops).
            if target != Target::Build {
                queue.extend(p.dev.iter().map(String::as_str));
            }
        }
        while let Some(name) = queue.pop() {
            let Some(dir) = self.by_name.get(name) else {
                continue;
            };
            if seen.insert(dir.clone()) {
                if let Some(p) = self.pkgs.get(dir) {
                    queue.extend(p.deps.iter().map(String::as_str));
                }
            }
        }
        self.reach.borrow_mut().insert(key, seen.clone());
        seen
    }

    /// Whether code in `from_file` can call the function in `to_file`; `to_public` is false when
    /// that function is not visible outside its own package. Unknown files are allowed.
    pub(crate) fn allows(&self, from_file: &str, to_file: &str, to_public: bool) -> bool {
        let (Some((from_pkg, from)), Some((to_pkg, to))) =
            (self.unit_of(from_file), self.unit_of(to_file))
        else {
            return true;
        };
        if from_pkg == to_pkg {
            // Tests, examples, benches and the build script are separate crates that see the package's
            // library, not each other. A build script's generated code (`quote!`, `include!`) lands in it too.
            return from == to || matches!(to, Target::Lib | Target::Bin);
        }
        to_public && to == Target::Lib && self.reachable(&from_pkg, from).contains(&to_pkg)
    }
}
