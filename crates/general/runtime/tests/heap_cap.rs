//! End-to-end check that the arena's `ZUTAI_HEAP_MAX` ceiling aborts a runaway
//! allocation with a diagnostic instead of leaking until the OS OOM-kills the
//! process (D-0008 release valve).
//!
//! The test re-execs its own binary: the child branch drives the real
//! `zutai.alloc` ABI past a tiny cap and must die nonzero with the diagnostic on
//! stderr. This exercises the production abort path without the LLVM toolchain.

use std::process::Command;

const CHILD_ENV: &str = "ZUTAI_RT_HEAP_CAP_CHILD";

#[test]
fn alloc_aborts_when_heap_cap_exceeded() {
    if std::env::var(CHILD_ENV).is_ok() {
        // Child: allocate far past the 1 MiB cap through the exported ABI. The
        // runtime must abort before this loop finishes; reaching the explicit
        // exit means the cap failed to fire, which fails the parent's assertion.
        for _ in 0..1_000_000 {
            let _ = zutai_rt::alloc(64 * 1024);
        }
        std::process::exit(0);
    }

    let exe = std::env::current_exe().expect("current test binary path");
    let output = Command::new(exe)
        .args([
            "--exact",
            "--nocapture",
            "alloc_aborts_when_heap_cap_exceeded",
        ])
        .env(CHILD_ENV, "1")
        .env("ZUTAI_HEAP_MAX", "1M")
        .output()
        .expect("spawn child test process");

    assert!(
        !output.status.success(),
        "child should abort nonzero when the heap cap is exceeded; got {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("heap limit exceeded"),
        "child stderr should explain the heap-cap abort; got: {stderr}"
    );
}
