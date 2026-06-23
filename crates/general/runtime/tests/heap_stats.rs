//! End-to-end check that `ZUTAI_HEAP_STATS` emits an accurate allocation report
//! at process exit (measurement groundwork for the GC decision).
//!
//! Re-execs its own binary: the child drives a fixed set of allocations through
//! the exported ABI and exits normally so the C `atexit` dump fires; the parent
//! asserts the per-kind counts on stderr. Exercises the real reporting path
//! without the LLVM toolchain.

use std::process::Command;

const CHILD_ENV: &str = "ZUTAI_RT_HEAP_STATS_CHILD";

#[test]
fn heap_stats_dump_reports_accurate_counts() {
    if std::env::var(CHILD_ENV).is_ok() {
        // A fixed, known allocation mix through the exported ABI: 3 records,
        // 1 tuple, 2 cons cells, 1 variant, 1 text, 1 raw (== closure-shaped)
        // allocation -> 9 objects, 1 of them outside the tracked tags.
        for _ in 0..3 {
            let _ = zutai_rt::record_new(1);
        }
        let _ = zutai_rt::tuple_new(2);
        for _ in 0..2 {
            let _ = zutai_rt::list_cons(0, zutai_rt::list_nil());
        }
        let _ = zutai_rt::variant_new(0, 0);
        let s = "hi";
        let _ = zutai_rt::text_from_global(s.as_ptr() as i64, s.len() as i64);
        let _ = zutai_rt::alloc(16);
        // Normal exit so the registered `atexit` dump runs.
        std::process::exit(0);
    }

    let exe = std::env::current_exe().expect("current test binary path");
    let output = Command::new(exe)
        .args([
            "--exact",
            "--nocapture",
            "heap_stats_dump_reports_accurate_counts",
        ])
        .env(CHILD_ENV, "1")
        .env("ZUTAI_HEAP_STATS", "1")
        .output()
        .expect("spawn child test process");

    assert!(
        output.status.success(),
        "child should exit cleanly; got {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("zutai heap stats:"),
        "child should print a stats line; got: {stderr}"
    );
    for needle in [
        "in 9 objects",
        "record 3",
        "tuple 1",
        "cons 2",
        "variant 1",
        "text 1",
        "closure/raw 1",
    ] {
        assert!(
            stderr.contains(needle),
            "stats line missing {needle:?}; got: {stderr}"
        );
    }
}
