//! In-process soundness check for the conservative collector (Phase 34).
//!
//! Drives the real allocation ABI under `ZUTAI_GC_STRESS` (collect before every
//! allocation) while keeping live objects referenced only from the stack, then
//! asserts they survived every collection. This exercises the actual root scan
//! (stack bounds + `setjmp` register spill) and transitive marking on whatever
//! platform the test is built for — so it is the end-to-end check for each
//! platform's `stack_base` path, no LLVM toolchain required.
//!
//! Re-execs its own binary so the child sees `ZUTAI_GC_STRESS` from the start
//! (the collector mode is resolved once per process).

use std::process::Command;

const CHILD_ENV: &str = "ZUTAI_RT_GC_INPROCESS_CHILD";

/// Build a linked list of `n` two-slot records (slot 0 = value, slot 1 = pointer
/// to the next node, 0 = nil), reachable only through the returned head pointer.
#[inline(never)]
fn build_chain(n: i64) -> i64 {
    let mut node: i64 = 0; // nil sentinel (not a pointer)
    for i in 1..=n {
        let r = zutai_rt::record_new(2);
        zutai_rt::record_set(r, 0, i);
        zutai_rt::record_set(r, 1, node);
        node = r;
    }
    node
}

/// Allocate and discard `n` records so the stress collector runs `n` times with
/// real garbage to reclaim. Returns a folded value so nothing is optimised away.
#[inline(never)]
fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0;
    for i in 0..n {
        let g = zutai_rt::record_new(1);
        zutai_rt::record_set(g, 0, i);
        acc = acc.wrapping_add(zutai_rt::record_get(g, 0));
    }
    acc
}

#[test]
fn collector_retains_live_objects_through_stress() {
    if std::env::var(CHILD_ENV).is_ok() {
        // A single live record held only on the stack.
        let live = zutai_rt::record_new(1);
        zutai_rt::record_set(live, 0, 0xCAFE);

        // A 100-node live chain, reachable only transitively through `head`.
        let head = build_chain(100);

        // Heavy churn: under ZUTAI_GC_STRESS this collects before each of the
        // ~2000 allocations. If the scan missed `live` or any chain node, the
        // freed memory would be handed back out below and corrupt the values.
        let folded = churn(2000);
        std::hint::black_box(folded);

        // The standalone live record survived untouched.
        assert_eq!(
            zutai_rt::record_get(live, 0),
            0xCAFE,
            "live record was reclaimed or overwritten during stress GC"
        );

        // The whole chain survived: 1 + 2 + ... + 100 = 5050.
        let mut sum = 0i64;
        let mut cur = head;
        while cur != 0 {
            sum += zutai_rt::record_get(cur, 0);
            cur = zutai_rt::record_get(cur, 1);
        }
        assert_eq!(sum, 5050, "live chain lost nodes during stress GC");

        std::process::exit(0);
    }

    let exe = std::env::current_exe().expect("current test binary path");
    let output = Command::new(exe)
        .args([
            "--exact",
            "--nocapture",
            "collector_retains_live_objects_through_stress",
        ])
        .env(CHILD_ENV, "1")
        .env("ZUTAI_GC_STRESS", "1")
        .output()
        .expect("spawn child test process");

    assert!(
        output.status.success(),
        "child should retain live objects under stress GC; status {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
}
