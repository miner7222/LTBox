# CI caching

## Rust CI: dependency artifacts

Build, test, and Clippy jobs use Swatinem/rust-cache with dependency target
artifacts and Cargo sources. Workspace artifacts and installed Cargo tools are
excluded. Each job clears RUSTC_WRAPPER: Rust CI no longer uses sccache for these
compilation jobs. Do not add the separate cargo-sources action to these jobs;
that would restore the same downloads twice.

Keys keep release, test, and cross-Clippy outputs separate, including each
architecture. rust-cache also keys on Rust/compiler environment and lockfiles.
Apple jobs include an Xcode/SDK/clang fingerprint; Windows cross jobs include
clang/lld and retain the separate target-specific cargo-xwin SDK cache. Avoid
broad restore prefixes for xwin SDKs.

Main and the diagnostic branch ci/sccache-diagnostics may populate dependency
caches. Other branches restore only. GitHub cache visibility is ref-scoped;
merging these changes does not transfer the diagnostic branch's caches to main.
Expect a new main cache warm-up. Do not cache downloaded firmware, provider
payloads, drivers, or other external-download test payloads.

## Measurements and results

Per-job cache-metrics artifacts (14-day retention) and job summaries include:

- Exact target cache hit and restore duration.
- Build, Clippy, workspace test, and demo phase durations where applicable.
- Target bytes before pruning, which are NOT the compressed cache size.

Post-job logs provide compressed archive size and cleanup/save duration.
Windows additionally snapshots repository cache usage. sccache.json records
`enabled: false` when no compiler wrapper is used. Compare whole job duration,
including restore and post steps, rather than compilation time alone.

The sequential experiment began with run 35189880119: direct sccache GHA writes
had 808 successes and 2,574 failures, all HTTP 429. Run 35218455625 completed
successfully with exact dependency cache hits in all ten compilation jobs.
Representative warm measurements from that final run:

| Job | Restore | Main measured phase |
|---|---:|---:|
| Windows tests | 26.5s | workspace 121.7s; demo 65.1s |
| Linux tests | 29.4s | Clippy 11.0s; tests 49.6s |
| macOS arm64 tests | 19.8s | Clippy 20.2s; tests 66.1s |
| macOS Intel tests | 45.1s | tests 111.8s |
| macOS universal | 16.9s | build/package 331.6s |
| Linux x64 / arm64 release | 6.8s / 9.8s | 138.5s / 123.0s |
| Windows x64 / arm64 cross release | 8.1s / 5.9s | 123.3s / 139.6s |
| Windows cross Clippy | 11.6s | default 9.6s; demo 5.4s |

The ten initial compressed dependency archives total approximately 6.93 GiB.
The repository cache usage snapshot remained close to 10 GiB, including older
and other caches. Multiple lockfile versions and refs can cause eviction;
these results do not guarantee long-term hit rates. No existing cache was
manually deleted. Hosted-runner load and image differences limit comparisons
between runs; each new configuration had a cold run and a same-commit warm run.

## Weekly download contracts

The external-downloads workflow was not executed or migrated in this experiment.
It still uses the cargo-sources action (OS/lockfile-keyed Cargo archives, index,
and Git objects), plus pinned sccache 0.18.0. Main-branch writers populate Linux
sources from the root-contract build and Windows sources from the Windows job.

Root contract checks compile one lib test executable and distribute it through
a one-day artifact. ci/stage_root_test.py selects the exact Cargo JSON executable
and verifies the ignored test exists before upload. Provider payloads are not
cached, so the weekly checks continue to exercise real downloads.

The sccache setup/report actions remain for these jobs. Private server logs are
reduced to numeric counters and allowlisted HTTP/error categories in summaries
and 14-day JSON artifacts. Raw logs, headers, URLs, and paths are not uploaded.
Unknown categories and unavailable statistics are explicit. The GHA namespace
includes sccache's version even when SCCACHE_GHA_VERSION is set, so version
upgrades should be treated as cold-cache events.
