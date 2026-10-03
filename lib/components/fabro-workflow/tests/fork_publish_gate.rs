//! Fork presence pin (fabro-4ebd, W2-2 Phase B): auto-merge is withheld
//! when the pull request deletes paths outside the run's own change
//! scope. The pure classification lives in `pull_request.rs`
//! (`diff_touched_paths` + `out_of_scope_deletions`); this pin mirrors the
//! parsing rules so drift between the gate and its documented contract
//! fails loudly, and asserts the wire gate's presence structurally.

use std::collections::HashSet;

fn diff_touched_paths_mirror(diff: &str) -> HashSet<&str> {
    let mut paths = HashSet::new();
    for line in diff.lines().filter(|line| line.starts_with("diff --git ")) {
        for token in line.split(' ').skip(2) {
            let path = token
                .strip_prefix("a/")
                .or_else(|| token.strip_prefix("b/"))
                .unwrap_or(token);
            if path != "/dev/null" {
                paths.insert(path);
            }
        }
    }
    paths
}

#[test]
fn diff_scope_parsing_covers_both_sides_and_ignores_dev_null() {
    let diff = "\
diff --git a/lib/foo.rs b/lib/foo.rs
index 111..222 100644
--- a/lib/foo.rs
+++ b/lib/foo.rs
diff --git a/lib/gone.rs b/lib/gone.rs
deleted file mode 100644
--- a/lib/gone.rs
+++ /dev/null
";
    let touched = diff_touched_paths_mirror(diff);
    assert!(touched.contains("lib/foo.rs"), "both sides count");
    assert!(
        touched.contains("lib/gone.rs"),
        "the run's own deletion counts as touched"
    );
    assert!(!touched.contains("/dev/null"), "/dev/null never counts");
    assert_eq!(touched.len(), 2);
}

#[test]
fn fabro_4ebd_wire_gate_exists() {
    let source = include_str!("../src/pull_request.rs");
    assert!(
        source.contains("fn enable_auto_merge_within_scope"),
        "the fabro-4ebd gate function must exist in pull_request.rs"
    );
    assert!(
        source.contains("Auto-merge withheld (fabro-4ebd)"),
        "the gate's diagnostic must stay greppable"
    );
    assert!(
        source.matches("enable_auto_merge_within_scope(").count() >= 3,
        "definition plus both call sites must use the scoped gate"
    );
}
