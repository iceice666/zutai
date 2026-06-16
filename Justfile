# Zutai workspace recipes

# Run tests with LLVM coverage (terminal summary)
cov:
    cargo llvm-cov nextest --workspace

# Run tests with LLVM coverage and open HTML report
cov-html:
    cargo llvm-cov nextest --workspace --html --open

# Standard checks (fmt + test + clippy)
check:
    cargo fmt --check
    cargo test --workspace
    cargo clippy --workspace --all-targets
