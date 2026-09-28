//! CLI snapshot tests (trycmd). Snapshots live in `tests/cmd/*.trycmd`.
//! Regenerate after intentional CLI changes with `TRYCMD=overwrite cargo test`.

#[test]
fn cli_tests() {
    // Pin env the snapshots are sensitive to, so they don't depend on the
    // host: a set RUST_BACKTRACE/RUST_LIB_BACKTRACE makes anyhow append a
    // "Stack backtrace:" block to every error.
    trycmd::TestCases::new()
        .env("RUST_BACKTRACE", "0")
        .env("RUST_LIB_BACKTRACE", "0")
        .case("tests/cmd/*.trycmd");
}
