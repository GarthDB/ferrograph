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
    // `pub(crate)` targets are roots, so check the call edges themselves.
    let called: std::collections::HashSet<String> = store
        .run_query(
            "?[p] := *nodes[t, \"function\", p], *edges[_, t, \"calls\"]",
            std::collections::BTreeMap::new(),
        )
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].to_string().trim_matches('"').to_string())
        .collect();
    for name in [
        "helper",
        "sup",
        "new",
        "tool",
        "other",
        "generate",
        "generate_two",
        "from_mod",
        "renamed_target",
    ] {
        assert!(
            called.contains(name) || called.contains(&format!("pub::{name}")),
            "{name} has no call edge: {called:?}"
        );
    }
    assert!(!called.contains("unused_in_lib") && !called.contains("pub::unused_in_util"));
    // An aliased owner (`use S as Alias`) is only a low-confidence edge, which `--strict` ignores.
    let mut strict = dead_names(&store, true);
    strict.sort();
    assert_eq!(strict, ["made_via_alias", "unused_in_lib"]);
    let mut dead = dead_names(&store, false);
    dead.sort();
    assert_eq!(dead, ["unused_in_lib"]);
}

#[test]
fn pipeline_bare_call_to_external_import_does_not_keep_local_fn_alive() {
    let store = run_fixture("ext_import");
    for strict in [false, true] {
        let mut dead = dead_names(&store, strict);
        dead.sort();
        assert_eq!(dead, ["from_str", "read"], "strict={strict}");
    }
}

#[test]
fn pipeline_self_method_calls_resolve_to_the_enclosing_impl() {
    let store = run_fixture("self_methods");
    let tags = tagged_lines("self_methods", "src/lib.rs");
    let mut want = vec![tags["dead_b_find"], tags["dead_b_macro"]];
    want.sort_unstable();
    // Both modes: `self.find()` is a certain edge to `A::find`, so `B::find` is dead and `A::find`
    // is not (with `--strict` it used to be the other way round).
    for strict in [false, true] {
        assert_eq!(
            dead_lines(&store, strict, "src/lib.rs"),
            want,
            "strict={strict}"
        );
    }
}

#[test]
fn pipeline_typed_receivers_resolve_to_their_type() {
    let store = run_fixture("receiver_types");
    let tags = tagged_lines("receiver_types", "src/lib.rs");
    let mut certain: Vec<usize> = tags
        .iter()
        .filter(|(k, _)| k.starts_with("dead_b_"))
        .map(|(_, l)| *l)
        .collect();
    certain.sort_unstable();
    assert_eq!(certain.len(), 9);
    // Default mode: unknown receivers keep both candidates alive (low confidence).
    assert_eq!(dead_lines(&store, false, "src/lib.rs"), certain);
    // `--strict` drops the low-confidence edges: only the two untyped call sites lose their callees.
    let mut strict = certain;
    strict.extend(
        [
            "unknown_a",
            "unknown_b",
            "shadowed_a",
            "shadowed_b",
            "library_push_a",
            "library_push_b",
            "args_a",
            "args_b",
        ]
        .map(|t| tags[t]),
    );
    strict.sort_unstable();
    assert_eq!(dead_lines(&store, true, "src/lib.rs"), strict);
    let other = tagged_lines("receiver_types", "src/other.rs");
    // `e.describe()` on a `std::io::Error` binds to the same-file `B::describe`, not `other::Error`.
    for strict in [false, true] {
        assert_eq!(
            dead_lines(&store, strict, "src/other.rs"),
            [other["describe_error"]]
        );
    }
}

#[test]
fn pipeline_module_path_types_resolve_to_their_type() {
    let store = run_fixture("module_path_types");
    let tags = tagged_lines("module_path_types", "src/lib.rs");
    let dead = dead_lines(&store, true, "src/lib.rs");
    for class in ["field", "param", "local", "wrapped"] {
        assert!(
            dead.contains(&tags[&format!("dead_b_{class}")]),
            "{class}: B"
        );
        assert!(
            !dead.contains(&tags[&format!("live_a_{class}")]),
            "{class}: A"
        );
    }
    // `io::Error`, `anyhow::Error` and `error::Error` stay unknown: both twins remain candidates, so `--strict` drops both.
    for tag in [
        "dead_b_raw",
        "dead_error_raw",
        "dead_b_ext",
        "dead_error_ext",
    ] {
        assert!(dead.contains(&tags[tag]), "{tag}");
    }
}

/// #115: the `B` twin of each method is the only dead one. `--strict` must not report the `A` method
/// reached through a pattern, `for` loop, `?` or closure binding.
fn assert_pattern_receivers_resolved(classes: &[&str]) {
    let store = run_fixture("pattern_receivers");
    let tags = tagged_lines("pattern_receivers", "src/lib.rs");
    let dead = dead_lines(&store, true, "src/lib.rs");
    for class in classes {
        assert!(
            dead.contains(&tags[&format!("dead_b_{class}")]),
            "{class}: B"
        );
        assert!(
            !dead.contains(&tags[&format!("live_a_{class}")]),
            "{class}: A"
        );
    }
}

#[test]
fn pipeline_for_loop_receivers_resolve_to_their_type() {
    assert_pattern_receivers_resolved(&[
        "for_field",
        "for_slice",
        "for_iter",
        "for_option",
        "boxed",
    ]);
}

#[test]
fn pipeline_variant_payload_receivers_resolve_to_their_type() {
    assert_pattern_receivers_resolved(&[
        "variant_if_let",
        "variant_match",
        "variant_bare",
        "variant_struct",
        "variant_self",
        "some_pattern",
        "while_let",
    ]);
}

#[test]
fn pipeline_try_operator_receivers_resolve_to_their_type() {
    assert_pattern_receivers_resolved(&["try_option", "try_result"]);
}

#[test]
fn pipeline_closure_param_receivers_resolve_to_their_type() {
    assert_pattern_receivers_resolved(&[
        "closure_map",
        "closure_typed",
        "closure_option",
        "closure_for_each",
        "closure_fold",
    ]);
}

#[test]
fn pipeline_index_and_map_receivers_resolve_to_their_type() {
    assert_pattern_receivers_resolved(&["index_chain", "map_values", "map_index"]);
}

/// #115: receivers bound through tuples, map iteration, `Option` adapters with closures and stored `fn`
/// pointers. The `B` twin of each positive class is the only dead one under `--strict`; a negative class
/// never gives a project type, so both twins stay (dead under `--strict`, kept alive by default).
fn assert_shape_receivers(positive: &[&str], negative: &[&str]) {
    let store = run_fixture("shape_receivers");
    let tags = tagged_lines("shape_receivers", "src/lib.rs");
    let dead = dead_lines(&store, true, "src/lib.rs");
    for class in positive {
        assert!(
            dead.contains(&tags[&format!("dead_b_{class}")]),
            "{class}: B"
        );
        assert!(
            !dead.contains(&tags[&format!("live_a_{class}")]),
            "{class}: A"
        );
    }
    for class in negative {
        for twin in ["a", "b"] {
            assert!(
                dead.contains(&tags[&format!("neg_{twin}_{class}")]),
                "{class}: {twin} stays untyped"
            );
        }
    }
}

#[test]
fn pipeline_tuple_pattern_receivers_resolve_to_their_type() {
    assert_shape_receivers(
        &["tuple_for", "tuple_if_let", "tuple_let"],
        &["tuple_pos", "tuple_rest", "tuple_let_ext"],
    );
}

#[test]
fn pipeline_map_iteration_receivers_resolve_to_their_type() {
    assert_shape_receivers(&["map_pairs", "map_field"], &["map_key"]);
}

#[test]
fn pipeline_option_adapter_receivers_resolve_to_their_type() {
    assert_shape_receivers(
        &["and_then", "opt_map", "then", "and_then_untyped"],
        &["and_then_ext", "then_flag"],
    );
}

#[test]
fn pipeline_fn_pointer_result_receivers_resolve_to_their_type() {
    assert_shape_receivers(
        &["fn_param", "fn_field", "fn_alias"],
        &["fn_ext", "fn_alias_generic"],
    );
}

#[test]
fn pipeline_module_path_constructor_receivers_resolve_to_their_type() {
    assert_shape_receivers(&["maker_path"], &["maker_ext"]);
}

#[test]
fn pipeline_let_else_receivers_resolve_to_their_type() {
    assert_shape_receivers(&["let_else"], &[]);
}

#[test]
fn pipeline_rc_clone_is_not_the_inner_types_clone() {
    let store = run_fixture("pattern_receivers");
    let tags = tagged_lines("pattern_receivers", "src/lib.rs");
    // `Rc::clone` is not `A::clone`: neither candidate gets a certain edge.
    let strict = dead_lines(&store, true, "src/lib.rs");
    assert!(strict.contains(&tags["rc_clone_a"]) && strict.contains(&tags["rc_clone_b"]));
}

#[test]
fn pipeline_pattern_receivers_rebound_to_an_unknown_type_stay_unresolved() {
    let store = run_fixture("pattern_receivers");
    let tags = tagged_lines("pattern_receivers", "src/lib.rs");
    // The `for` binding of `a` is out of scope at `a.shadowed()`: both candidates stay low-confidence.
    let mut certain: Vec<usize> = tags
        .iter()
        .filter(|(k, _)| k.starts_with("dead_b_"))
        .map(|(_, l)| *l)
        .collect();
    certain.sort_unstable();
    assert_eq!(dead_lines(&store, false, "src/lib.rs"), certain);
    let strict = dead_lines(&store, true, "src/lib.rs");
    assert!(strict.contains(&tags["shadowed_a"]) && strict.contains(&tags["shadowed_b"]));
}

#[test]
fn pipeline_bare_call_does_not_bind_to_a_method_of_the_same_file() {
    let store = run_fixture("bare_vs_method");
    let tags = tagged_lines("bare_vs_method", "src/lib.rs");
    for strict in [false, true] {
        assert_eq!(
            dead_lines(&store, strict, "src/lib.rs"),
            [tags["dead_method"]],
            "strict={strict}"
        );
    }
}

/// Target behaviour for #91. Ignored: with `pub(crate)`/`pub(super)` as non-roots the accuracy harness
/// measured 3.8% judgeable precision (280 reported, 8 right; 8/207 judgeable) even with receiver
/// types, because `use` paths and pattern/loop-bound receivers are not resolved (#114, #115).
/// Remove the `ignore` together with the `is_pub` change in `ast.rs`.
#[test]
#[ignore = "#91: needs better method/path call resolution first"]
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

/// Targets of `calls` edges leaving the function `caller`, as (callee id, is low-confidence).
fn call_targets(store: &Store, caller: &str) -> Vec<(String, bool)> {
    let run = |script: &str| -> Vec<String> {
        let script = script.replace("$p", &format!("[\"{caller}\", \"pub::{caller}\"]"));
        let rows = store
            .run_query(&script, std::collections::BTreeMap::new())
            .unwrap();
        rows.rows
            .iter()
            .map(|r| r[0].to_string().trim_matches('"').to_string())
            .collect()
    };
    let all = run("?[t] := *nodes[f, \"function\", p], is_in(p, $p), *edges[f, t, \"calls\"]");
    let low = run(
        "?[t] := *nodes[f, \"function\", p], is_in(p, $p), *edge_confidence[f, t, \"calls\", _]",
    );
    all.into_iter()
        .map(|t| {
            let is_low = low.contains(&t);
            (t, is_low)
        })
        .collect()
}

fn assert_one_certain_call(store: &Store, caller: &str, target_prefix: &str) {
    let targets = call_targets(store, caller);
    let certain: Vec<_> = targets.iter().filter(|t| !t.1).collect();
    assert!(
        targets.len() == 1 && certain.len() == 1 && certain[0].0.starts_with(target_prefix),
        "{caller} should call exactly one certain {target_prefix}*, got {targets:?}"
    );
}

#[test]
fn pipeline_use_forms_resolve_same_named_free_fns() {
    let store = run_fixture("use_forms");
    for i in 1..=6 {
        assert_one_certain_call(&store, &format!("api{i}"), &format!("./src/r{i}/a.rs#"));
    }
}

#[test]
fn pipeline_use_resolves_through_reexport() {
    let store = run_fixture("reexport");
    assert_one_certain_call(&store, "api", "./src/output/help.rs#");
}

#[test]
fn pipeline_use_crate_paths_stay_inside_workspace_member() {
    let store = run_fixture("ws_members");
    assert_one_certain_call(&store, "api_m", "./m/src/a.rs#");
    assert_one_certain_call(&store, "api_n", "./n/src/a.rs#");
}

#[test]
fn pipeline_use_super_in_path_attribute_module() {
    let store = run_fixture("path_mods");
    assert_one_certain_call(&store, "caller", "./src/a.rs#");
}

#[test]
fn pipeline_qualified_call_through_use_alias() {
    let store = run_fixture("alias_calls");
    assert_one_certain_call(&store, "api_bash", "./src/flags/complete/bash.rs#");
    assert_one_certain_call(&store, "api_fish", "./src/flags/complete/fish.rs#");
}

#[test]
fn pipeline_qualified_call_through_alias_to_nested_free_fn() {
    let store = run_fixture("alias_calls");
    assert_one_certain_call(&store, "api_nested", "./src/util/inner/deep.rs#");
}

#[test]
fn pipeline_qualified_call_through_alias_of_reexport() {
    let store = run_fixture("alias_calls");
    assert_one_certain_call(&store, "api_chain", "./src/chain/mid/leaf.rs#");
}

#[test]
fn pipeline_qualified_call_alias_is_owner_specific() {
    // `other` has no `gen_bash`; `flags`' alias of that name must not leak into it.
    let store = run_fixture("alias_calls");
    assert_eq!(call_targets(&store, "api_negative"), vec![]);
}

#[test]
fn pipeline_qualified_alias_in_two_same_named_modules_is_not_certain() {
    let store = run_fixture("alias_calls");
    assert!(call_targets(&store, "api_two_stems").iter().all(|t| t.1));
}

#[test]
fn pipeline_qualified_alias_behind_foreign_leading_segment_is_low_confidence() {
    // `somecrate::util::ext_alias()` must not be a certain edge into the project's `ext/util`.
    let store = run_fixture("alias_calls");
    let targets = call_targets(&store, "api_external_path");
    assert!(targets.iter().all(|t| t.1), "{targets:?}");
}

#[test]
fn pipeline_bare_call_resolves_through_glob_import() {
    let store = run_fixture("glob_calls");
    assert_one_certain_call(&store, "api_glob", "./src/lib.rs#");
}

#[test]
fn pipeline_glob_import_does_not_pick_between_same_named_fns() {
    let store = run_fixture("glob_calls");
    let targets = call_targets(&store, "api_glob_twin");
    assert!(
        targets.len() == 2 && targets.iter().all(|t| t.1),
        "{targets:?}"
    );
}

#[test]
fn pipeline_module_path_call_prefers_the_module_file_over_variants_and_nested_mods() {
    let store = run_fixture("module_files");
    assert_one_certain_call(&store, "api_own", "./src/drag/mod.rs#");
}

#[test]
fn pipeline_module_path_call_follows_the_module_file_reexport() {
    let store = run_fixture("module_files");
    assert_one_certain_call(&store, "api_reexport", "./src/render/warmup.rs#");
}

#[test]
fn pipeline_module_path_call_through_path_attribute_module_name() {
    let store = run_fixture("path_mod_names");
    assert_one_certain_call(&store, "api_path_mod", "./src/hidden/real.rs#");
}

#[test]
fn pipeline_type_path_call_picks_the_type_of_the_named_module() {
    let store = run_fixture("module_types");
    assert_one_certain_call(&store, "api_typed", "./src/a.rs#");
}

#[test]
fn pipeline_crate_root_declared_by_bin_path_outside_src() {
    let store = run_fixture("bin_path_root");
    assert_one_certain_call(&store, "api_bin_root", "./app/flags/complete/bash.rs#");
}

#[test]
fn pipeline_module_path_call_prefers_the_callers_child_module_over_inline_mods_elsewhere() {
    let store = run_fixture("child_module_calls");
    assert_one_certain_call(&store, "api_child", "./src/run/mac.rs#");
}

#[test]
fn pipeline_module_path_call_through_use_alias_of_an_inline_mod() {
    let store = run_fixture("owner_alias");
    assert_eq!(
        call_targets(&store, "api_module_alias"),
        vec![("./src/lib.rs#2:5".to_string(), false)]
    );
}

/// #122: receivers behind `RefCell`/`Mutex`/`RwLock` guards. The `B` twin of each positive class is the
/// only dead one under `--strict`; `neg_*` sits behind a guard of a non-project type and stays untyped.
#[test]
fn pipeline_guard_receivers_resolve_to_their_inner_type() {
    let store = run_fixture("guard_receivers");
    let tags = tagged_lines("guard_receivers", "src/lib.rs");
    let dead = dead_lines(&store, true, "src/lib.rs");
    for class in [
        "refcell",
        "refcell_mut",
        "mutex",
        "rwlock_read",
        "rwlock_write",
        "guard_ret",
        "let_guard",
        "poisoned",
        "scoped",
    ] {
        assert!(
            dead.contains(&tags[&format!("dead_b_{class}")]),
            "{class}: B"
        );
        assert!(
            !dead.contains(&tags[&format!("live_a_{class}")]),
            "{class}: A"
        );
    }
    for twin in ["a", "b"] {
        assert!(dead.contains(&tags[&format!("neg_{twin}_ext")]), "{twin}");
    }
}

/// #131: a `use .. as Name` inside a function is only visible there.
#[test]
fn pipeline_function_local_aliases_do_not_collide() {
    let store = run_fixture("alias_scopes");
    let tags = tagged_lines("alias_scopes", "src/main.rs");
    let dead = dead_lines(&store, true, "src/main.rs");
    for live in ["live_a_new", "live_b_new"] {
        assert!(!dead.contains(&tags[live]), "{live}");
    }
    for stays_dead in ["dead_d_new", "dead_only_b", "dead_c_make", "neg_ext_make"] {
        assert!(dead.contains(&tags[stays_dead]), "{stays_dead}");
    }
}
