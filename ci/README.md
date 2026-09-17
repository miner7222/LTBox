# CI caching

## Downloads and compilation

The local `cargo-sources` action stores Cargo registry archives, registry index
data, and Git objects. Its key uses the OS and root `Cargo.lock`, not the Rust
compiler, CPU architecture, or flags. Cargo reconstructs extracted sources and
Git checkouts. A designated main-branch writer fetches the complete lockfile on
a miss; other jobs and pull requests restore without saving another copy.

Linux x86_64 and macOS release-build jobs populate their OS caches. The weekly
root build can also populate Linux sources, and the weekly non-root Windows job
populates Windows sources. Compiler output remains in sccache. Do not share the
target-specific xwin SDK caches across Windows target architectures.

Weekly root checks compile one lib test executable and share it through a
one-day workflow artifact. The executable is selected from Cargo JSON and its
exact ignored test is checked before upload, so renamed tests cannot silently
produce a zero-test success. All eight providers still run independently. This
artifact belongs to the current workflow run; it is not a cross-run binary cache.
Downloaded provider APKs, modules, drivers, and application assets are never
included in these caches.

## Windows dependency cache experiment

The native Windows test job additionally uses rust-cache for dependency build
artifacts. Workspace artifacts and installed Cargo tools are excluded. This
job retains rust-cache's own compiler/environment/lockfile key and source cache;
it does not also restore the separate source-only cache. Only main writes it.
Other build jobs continue to use sccache without a target archive.

`windows-cache-metrics` artifacts (14-day retention) and the job summary record:

- Dependency cache restoration time, including rust-cache setup.
- Workspace and demo test command duration and exit status.
- Exact cache-hit status, raw sccache statistics, and repository cache usage
  during the run (before the new target archive is saved).
- Target size before rust-cache prunes it. This is **not** the compressed archive
  size; post-job cache logs contain actual archive size and save time.

Compare repeated runs of the same commit and runner image after the cache is
warm. Compare total job time, not just compilation, against the prior workflow.
The baseline run `34700445610` had a 62.87% Rust sccache hit rate for Windows
tests, 105 non-cacheable `crate-type` calls, and a 4m54s test build. That run used
an older commit, so it is context rather than a controlled benchmark.

If restore/save overhead outweighs the build savings, set `cache-targets` back
to false and retain the measurements. Avoid enabling target archives on all
matrix entries without measuring their impact on repository cache pressure.
Existing caches are not deleted by this change.

## sccache write diagnostics

All compiler-cache jobs use `.github/actions/sccache-setup`, pinning sccache to
0.18.0. The GHA cache namespace includes the sccache version, even when
`SCCACHE_GHA_VERSION` is set; review version upgrades as cold-cache events.

The setup action enables only `sccache::server` debug logging in a runner-temporary
file. The report action runs after build/test steps, including failures, and
publishes numeric statistics plus allowlisted HTTP error codes and OpenDAL error
kinds in the job summary and a 14-day JSON artifact. Raw logs, error messages,
headers, URLs, and paths are not uploaded. An unknown category is deliberately
retained rather than publishing unrecognized text. Missing logs/statistics are
reported as unavailable, not zero errors. Diagnostic failures do not fail builds.

Compare `cache_write_errors` with HTTP 429 (rate limiting), 403 (access), and
409 / AlreadyExists (competing writes). Log-entry counts need not match final
statistics exactly; the snapshot precedes action post steps. Existing cache
behavior remains unchanged so the next run can diagnose the current backend.
Use workflow_dispatch on the diagnostic branch to run Rust CI; pushing a branch
other than main/dev does not trigger it automatically. The external-download
workflow uses the same setup but need not be run for this investigation.
