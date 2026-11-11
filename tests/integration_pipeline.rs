//! End-to-end pipeline tests using fixture projects.

use std::path::Path;

use ferrograph::graph::Store;
use ferrograph::pipeline::{run_pipeline, PipelineConfig};

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_fixture(name: &str) -> Store {
    let root = fixture_path(name);
    assert!(
        root.exists(),
        "fixture missing: {} (run from repo root)",
        root.display()
    );
    let store = Store::new_memory().unwrap();
    let config = PipelineConfig::default();
    run_pipeline(&store, &root, &config).expect("pipeline failed");
    store
}

fn node_types(store: &Store) -> Vec<String> {
    ferrograph::graph::Query::all_nodes(store)
        .unwrap()
        .rows
        .iter()
        .filter_map(|r| r.get(1))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect()
}

fn edge_types(store: &Store) -> Vec<String> {
    ferrograph::graph::Query::all_edges(store)
        .unwrap()
        .rows
        .iter()
        .filter_map(|r| r.get(2))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect()
}

fn assert_test_fns_not_dead(store: &Store) {
    let dead = ferrograph::graph::Query::stored_dead_functions(store).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    let test_fn_ids: Vec<String> = nodes
        .rows
        .iter()
        .filter_map(|r| {
            let payload = r
                .get(2)
                .map(|v| v.to_string().trim_matches('"').to_string())?;
            if payload.starts_with("test::") {
                r.first()
                    .map(|v| v.to_string().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect();
    for id in &test_fn_ids {
        assert!(
            !dead.contains(id),
            "test function {id} must not be in dead list, dead: {dead:?}"
        );
    }
}

/// Find a node ID by payload (substring match) and optional type. Prefers lib.rs for `single_crate`.
fn find_node_id(
    store: &Store,
    payload_contains: &str,
    node_type_filter: Option<&str>,
) -> Option<String> {
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    for row in &nodes.rows {
        let id = row
            .first()
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let typ = row
            .get(1)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let payload = row
            .get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        if payload.contains(payload_contains)
            && node_type_filter.is_none_or(|t| typ == t)
            && (id.contains("lib.rs") || node_type_filter == Some("primitive"))
        {
            return Some(id);
        }
    }
    None
}

/// Assert that an edge exists from a node (by payload) to another (by payload) with the given edge type.
fn assert_has_edge(
    store: &Store,
    from_payload: &str,
    from_type: &str,
    to_payload: &str,
    to_type: &str,
    edge_type: &str,
) {
    let from_id = find_node_id(store, from_payload, Some(from_type))
        .unwrap_or_else(|| panic!("node not found: {from_payload} ({from_type})"));
    let to_id = find_node_id(store, to_payload, Some(to_type))
        .unwrap_or_else(|| panic!("node not found: {to_payload} ({to_type})"));
    let edges = ferrograph::graph::Query::all_edges(store).unwrap();
    let has = edges.rows.iter().any(|r| {
        r.get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .as_deref()
            == Some(edge_type)
            && r.first()
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some(from_id.as_str())
            && r.get(1)
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some(to_id.as_str())
    });
    assert!(
        has,
        "expected {edge_type} edge from {from_payload} to {to_payload}, from_id={from_id}, to_id={to_id}"
    );
}

fn assert_call_to_greet(store: &Store) {
    let edges = ferrograph::graph::Query::all_edges(store).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    let greet_id = nodes.rows.iter().find_map(|r| {
        let id = r
            .first()
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        let payload = r
            .get(2)
            .map(|v| v.to_string().trim_matches('"').to_string())?;
        if (payload == "greet" || payload == "pub::greet") && id.contains("lib.rs") {
            Some(id)
        } else {
            None
        }
    });
    let has_call = greet_id.is_some_and(|gid| {
        edges.rows.iter().any(|r| {
            r.get(2)
                .map(|v| v.to_string().trim_matches('"').to_string())
                .as_deref()
                == Some("calls")
                && r.get(1)
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .as_deref()
                    == Some(&gid)
        })
    });
    assert!(
        has_call,
        "expected a Calls edge to greet (from main.rs use crate::greet; greet();)"
    );
}

#[test]
fn pipeline_indexes_single_crate() {
    let store = run_fixture("single_crate");
    let types = node_types(&store);
    assert!(
        types.contains(&"function".to_string()),
        "expected at least one function node, got: {types:?}"
    );
    assert!(
        types.contains(&"file".to_string()),
        "expected file node, got: {types:?}"
    );
    for want in ["struct", "enum", "trait", "impl"] {
        assert!(
            types.contains(&want.to_string()),
            "expected {want} node from fixture, got: {types:?}"
        );
    }
    let etypes = edge_types(&store);
    assert!(
        etypes.contains(&"contains".to_string()),
        "expected Contains edges, got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"calls".to_string()),
        "expected Calls edges (e.g. main -> bar), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"owns".to_string()),
        "expected Owns edges (e.g. Container->Point, take_ownership->Point), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"borrows".to_string()),
        "expected Borrows edges (e.g. borrow_ref->Point), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"borrows_mut".to_string()),
        "expected BorrowsMut edges (e.g. borrow_mut_ref->Point), got: {etypes:?}"
    );
    assert!(
        etypes.contains(&"lifetime_scope".to_string()),
        "expected LifetimeScope edges (e.g. self-loop on Wrapper, with_lifetime), got: {etypes:?}"
    );
    assert_has_edge(&store, "Container", "struct", "Point", "struct", "owns");
    assert_has_edge(
        &store,
        "borrow_ref",
        "function",
        "Point",
        "struct",
        "borrows",
    );
    assert_has_edge(
        &store,
        "borrow_mut_ref",
        "function",
        "Point",
        "struct",
        "borrows_mut",
    );
    assert_has_edge(
        &store,
        "get_mut_point",
        "function",
        "Point",
        "struct",
        "borrows_mut",
    );
    assert_has_edge(&store, "Pair", "struct", "Point", "struct", "owns");
    assert_has_edge(&store, "Container", "struct", "str", "primitive", "borrows");
    assert_has_edge(&store, "Point", "struct", "i32", "primitive", "owns");
    assert_has_edge(&store, "greet", "function", "str", "primitive", "borrows");
    assert_has_edge(
        &store,
        "create_point",
        "function",
        "Point",
        "struct",
        "owns",
    );
    let dead = ferrograph::graph::Query::stored_dead_functions(&store).unwrap();
    assert!(
        !dead.is_empty(),
        "fixture has unused() which should be detected as dead, got: {dead:?}"
    );
    assert_test_fns_not_dead(&store);
    assert_call_to_greet(&store);
}

#[test]
fn pipeline_indexes_workspace() {
    let store = run_fixture("workspace");
    let rows = ferrograph::graph::Query::all_nodes(&store).unwrap();
    assert!(
        rows.rows.len() >= 4,
        "workspace should have nodes from multiple crates, got {}",
        rows.rows.len()
    );
    let payloads: Vec<String> = rows
        .rows
        .iter()
        .filter_map(|r| r.get(2))
        .map(|v| v.to_string().trim_matches('"').to_string())
        .collect();
    assert!(
        payloads.iter().any(|p| p == "a" || p.starts_with("pub::a")),
        "expected node from crate_a (e.g. 'a'), got payloads: {payloads:?}"
    );
    assert!(
        payloads.iter().any(|p| p == "b" || p.starts_with("pub::b")),
        "expected node from crate_b (e.g. 'b'), got payloads: {payloads:?}"
    );
}

#[test]
fn pipeline_fn_used_as_value_is_not_dead() {
    let store = run_fixture("fn_values");
    let dead = ferrograph::graph::Query::stored_dead_functions_with_payload(&store).unwrap();
    let names: Vec<&str> = dead.iter().filter_map(|(_, _, p)| p.as_deref()).collect();
    for live in ["as_arg", "in_field", "in_array", "in_map", "in_let"] {
        assert!(
            !names.contains(&live),
            "{live} is used as a value, dead: {names:?}"
        );
    }
    assert!(
        names.contains(&"truly_unused"),
        "truly_unused must stay dead, dead: {names:?}"
    );
}

fn dead_names(store: &Store, strict: bool) -> Vec<String> {
    let ids = ferrograph::graph::Query::compute_dead_functions_opts(store, strict).unwrap();
    let nodes = ferrograph::graph::Query::all_nodes(store).unwrap();
    nodes
        .rows
        .iter()
        .filter(|r| ids.contains(&r[0].to_string().trim_matches('"').to_string()))
        .map(|r| r[2].to_string().trim_matches('"').to_string())
        .collect()
}

#[test]
fn pipeline_method_and_turbofish_calls_keep_callees_alive() {
    let store = run_fixture("method_calls");
    let dead = dead_names(&store, false);
    for live in [
        "helper",
        "outer",
        "inner",
        "free_from_inner",
        "assoc",
        "first",
        "second",
        "parse",
        "gen_collect",
        "in_macro",
        "in_format",
    ] {
        assert!(
            !dead.iter().any(|n| n == live),
            "{live} is live, dead: {dead:?}"
        );
    }
    for still_dead in ["truly_unused_method", "orphan_free"] {
        assert!(
            dead.iter().any(|n| n == still_dead),
            "{still_dead} must stay dead, dead: {dead:?}"
        );
    }
}

#[test]
fn pipeline_method_name_collision_is_low_confidence() {
    let store = run_fixture("method_calls");
    let rows = store
        .run_query(
            "?[t, c] := *edge_confidence[_, t, \"calls\", c]",
            std::collections::BTreeMap::new(),
        )
        .unwrap();
    // `A.run()` has two same-named candidates (A::run, B::run): both at 1/2.
    assert_eq!(rows.rows.len(), 2, "rows: {:?}", rows.rows);
    assert!(rows.rows.iter().all(|r| r[1].get_float() == Some(0.5)));
    // Default mode keeps both alive; strict drops the low-confidence edges (#57).
    let default = dead_names(&store, false);
    let strict = dead_names(&store, true);
    assert!(!default.iter().any(|n| n == "run"), "{default:?}");
    assert_eq!(
        strict.iter().filter(|n| *n == "run").count(),
        2,
        "{strict:?}"
    );
}

#[test]
fn pipeline_trait_impl_methods_are_roots() {
    let store = run_fixture("trait_impls");
    for strict in [false, true] {
        let dead = dead_names(&store, strict);
        for live in [
            "area",
            "fmt",
            "add",
            "fmt_helper",
            "drop",
            "drop_helper",
            "alloc",
            "dealloc",
            "realloc",
            "alloc_helper",
            "dealloc_helper",
            "realloc_helper",
            "dflt",
            "gdflt",
        ] {
            assert!(
                !dead.iter().any(|n| n == live),
                "{live} is live (strict={strict}), dead: {dead:?}"
            );
        }
        for still_dead in ["truly_unused_private", "orphan_default"] {
            assert!(
                dead.iter().any(|n| n == still_dead),
                "{still_dead} must stay dead (strict={strict}), dead: {dead:?}"
            );
        }
    }
}

#[test]
fn pipeline_default_entry_attributes_and_commented_tests() {
    let store = run_fixture("entry_attrs");
    for strict in [false, true] {
        let dead = dead_names(&store, strict);
        assert_eq!(dead, vec!["unused_private"], "strict={strict}");
    }
}

#[test]
fn pipeline_fns_named_in_attribute_arguments_are_live() {
    let store = run_fixture("attr_args");
    for strict in [false, true] {
        let mut dead = dead_names(&store, strict);
        dead.sort();
        assert_eq!(dead, ["long", "short", "unused_private"], "strict={strict}");
    }
}

#[test]
fn pipeline_cross_file_qualified_paths_resolve() {
    let store = run_fixture("xfile_paths");
    for strict in [false, true] {
        let mut dead = dead_names(&store, strict);
        dead.sort();
        assert_eq!(dead, ["unused_in_lib", "unused_in_util"], "strict={strict}");
    }
}

#[test]
fn pipeline_restricted_visibility_fns_are_not_roots() {
    let store = run_fixture("restricted_vis");
    for strict in [false, true] {
        let mut dead = dead_names(&store, strict);
        dead.sort();
        assert_eq!(
            dead,
            [
                "unused_crate",
                "unused_in_path",
                "unused_nested",
                "unused_super"
            ],
            "strict={strict}"
        );
    }
}

/// Line numbers (1-based) of fixture definitions, keyed by their trailing `// tag` or `/* tag */`.
fn tagged_lines(fixture: &str, file: &str) -> std::collections::HashMap<String, usize> {
    let src = std::fs::read_to_string(fixture_path(fixture).join(file)).unwrap();
    src.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let tag = l.rsplit_once("// ").map(|(_, t)| t.trim()).or_else(|| {
                l.split_once("/* ")
                    .and_then(|(_, r)| r.split_once(" */"))
                    .map(|(t, _)| t)
            })?;
            Some((tag.to_string(), i + 1))
        })
        .collect()
}

/// Dead function lines in `file` (`./src/main.rs` style ids).
fn dead_lines(store: &Store, strict: bool, file: &str) -> Vec<usize> {
    let ids = ferrograph::graph::Query::compute_dead_functions_opts(store, strict).unwrap();
    let prefix = format!("./{file}#");
    let mut v: Vec<usize> = ids
        .iter()
        .filter_map(|id| id.strip_prefix(&prefix))
        .filter_map(|r| r.split(':').next()?.parse().ok())
        .collect();
    v.sort_unstable();
    v
}

fn low_confidence_edges(store: &Store) -> usize {
    store
        .run_query(
            "?[t, c] := *edge_confidence[_, t, \"calls\", c]",
            std::collections::BTreeMap::new(),
        )
        .unwrap()
        .rows
        .len()
}

#[test]
fn pipeline_same_name_fns_resolve_by_lexical_scope() {
    let store = run_fixture("scoped_names");
    let tags = tagged_lines("scoped_names", "src/main.rs");
    let mut want: Vec<usize> = [
        "outer_shadowed",
        "s2_shadowed",
        "m2_helper",
        "outer_inner",
        "tri3",
        "e_h",
    ]
    .iter()
    .map(|t| tags[*t])
    .collect();
    want.sort_unstable();
    // Scope decides every case: no low-confidence edges, so default and strict agree.
    assert_eq!(low_confidence_edges(&store), 0);
    for strict in [false, true] {
        let got = dead_lines(&store, strict, "src/main.rs");
        assert_eq!(got, want, "strict={strict}");
    }
    // #88: both `view`s are live.
    for live in [
        "outer_view",
        "b_view",
        "s1_shadowed",
        "m1_helper",
        "c_inner",
        "tri1",
        "tri2",
        "d_h",
    ] {
        assert!(!want.contains(&tags[live]), "{live} must be live");
    }
}

#[test]
fn pipeline_path_calls_resolve_among_same_named_assoc_fns() {
    let store = run_fixture("path_calls");
    let tags = tagged_lines("path_calls", "src/main.rs");
    let lines = |names: &[&str]| -> Vec<usize> {
        let mut v: Vec<usize> = names.iter().map(|t| tags[*t]).collect();
        v.sort_unstable();
        v
    };
    // Default: cfg variants are kept alive through low-confidence edges; wrong-type fns stay dead.
    let default = dead_lines(&store, false, "src/main.rs");
    assert_eq!(default, lines(&["p_mk", "y_g"]));
    // `--strict` drops the low-confidence edges: the cfg variants (undecidable statically) show up.
    let strict = dead_lines(&store, true, "src/main.rs");
    assert_eq!(strict, lines(&["f_unix", "f_other", "p_mk", "y_g"]));
    // Exactly the two cfg variants carry confidence 1/2; the rest are normal edges.
    let rows = store
        .run_query(
            "?[t, c] := *edge_confidence[_, t, \"calls\", c]",
            std::collections::BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(rows.rows.len(), 2, "{:?}", rows.rows);
    assert!(rows.rows.iter().all(|r| r[1].get_float() == Some(0.5)));
}

fn low_confidence_targets(store: &Store) -> Vec<(String, f64)> {
    store
        .run_query(
            "?[t, c] := *edge_confidence[_, t, \"calls\", c]",
            std::collections::BTreeMap::new(),
        )
        .unwrap()
        .rows
        .iter()
        .map(|r| {
            (
                r[0].to_string().trim_matches('"').to_string(),
                r[1].get_float().unwrap(),
            )
        })
        .collect()
}

#[test]
fn pipeline_macro_bodies_with_any_delimiter_keep_callees_alive() {
    let store = run_fixture("macro_bodies");
    for strict in [false, true] {
        let dead = dead_names(&store, strict);
        for live in [
            "bubble",
            "third",
            "in_macro_def",
            "observer_class",
            "observer_class_helper",
            "observe",
            "value_in_vec",
            "story_a",
            "story_b",
            "story_helper",
            "body_helper",
            "inner_callee",
            "turbo_in_macro",
        ] {
            assert!(
                !dead.iter().any(|n| n == live),
                "{live} is live (strict={strict}), dead: {dead:?}"
            );
        }
        for still_dead in [
            "neighbour_unused",
            "only_in_unused_macro",
            "only_in_dead_caller",
            "dead_caller",
            "story_unlisted",
        ] {
            assert!(
                dead.iter().any(|n| n == still_dead),
                "{still_dead} must stay dead (strict={strict}), dead: {dead:?}"
            );
        }
    }
    // `vec![Maker::make()]` reaches the method; the free fn of the same name stays dead.
    let dead = dead_names(&store, false);
    assert_eq!(dead.iter().filter(|n| *n == "make").count(), 1, "{dead:?}");
    // `vec![a.go()]` with two `go` methods: low-confidence (1/2) edges, dropped by --strict only.
    let low = low_confidence_targets(&store);
    assert_eq!(low.len(), 2, "{low:?}");
    assert!(low.iter().all(|(_, c)| (*c - 0.5).abs() < f64::EPSILON));
    assert!(!dead_names(&store, false).iter().any(|n| n == "go"));
    assert_eq!(
        dead_names(&store, true)
            .iter()
            .filter(|n| *n == "go")
            .count(),
        2
    );
}

#[test]
fn pipeline_const_and_static_initialisers_keep_fns_alive() {
    let store = run_fixture("const_refs");
    for strict in [false, true] {
        let dead = dead_names(&store, strict);
        for live in [
            "handler",
            "init",
            "adapter",
            "adapter_field",
            "table_fn",
            "const_helper",
            "lazy_init",
            "builtin",
            "used_builtin",
            "trait_const_init",
            "kept_by_used",
            "reg_fn",
        ] {
            assert!(
                !dead.iter().any(|n| n == live),
                "{live} is live (strict={strict}), dead: {dead:?}"
            );
        }
        // `dv` is the cross-file value path of #97 item 3, not fixed yet.
        for still_dead in [
            "dv",
            "unref_fn",
            "unref_const_fn",
            "max_init",
            "hidden_builtin",
            "dead_turbofish",
            "unused_reg_fn",
        ] {
            assert!(
                dead.iter().any(|n| n == still_dead),
                "{still_dead} must stay dead (strict={strict}), dead: {dead:?}"
            );
        }
    }
    let low = low_confidence_targets(&store);
    assert!(low.is_empty(), "{low:?}");
}
