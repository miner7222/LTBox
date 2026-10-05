# Vendored `qdl`

> **Local change: `bincode` removed.** Upstream `qdl` (de)serializes the Sahara
> packet structs and the cpio `newc` header with `bincode` 1.x, which is
> unmaintained (RUSTSEC-2025-0141; its 3.0.0 release is only a `compile_error!`).
> LTBox replaced it with explicit little-endian encode/decode in `src/wire.rs`
> and `src/sahara.rs` / `src/lib.rs` (`serde` and `serde_repr` went with it), and
> bumped `xmltree` 0.11 to 0.12. Golden-byte tests in `src/sahara.rs` (`golden`)
> and `src/lib.rs` (`cpio_golden`) pin the wire format. When re-vendoring, do
> not reintroduce `bincode`; re-apply this change instead.

This is a vendored copy of the `qdl` crate from
[`qualcomm/qdlrs`](https://github.com/qualcomm/qdlrs) (Sahara / Firehose
EDL transport), licensed BSD-3-Clause (see `LICENSE`).

## Why vendored

The EDL flash path needs fixes that are not yet in an upstream release,
and `qualcomm/qdlrs` is not pushable by us. Vendoring keeps the build
reproducible (no external fork repo, no submodule fetch in CI) while
carrying the minimal patches we need.

## Source

- Upstream: `qualcomm/qdlrs` `main` at `75fe2f7`. Everything under `src/`
  matches that revision except the local patches listed below.
- Only the `qdl` library crate files under `qdl/src/` and the crate
  `Cargo.toml` were vendored. Upstream CLI files under `cli/` were not
  copied into this vendor tree.
- The standalone vendored `Cargo.toml` omits upstream's `README.md`
  pointer and placeholder description comment because that README is not
  copied, and keeps `publish = false` without the upstream TODO comment.
- Intervening repository-only files outside the library crate (for
  example workflows and agent docs) were not copied.

## Local patches

- **Structured GUI operation logs** (`src/operation_log.rs`, `src/lib.rs`,
  `src/parsers.rs`, `src/sahara.rs`). An optional observer receives throttled
  storage/Sahara-dump byte counts and protocol diagnostics. When installed, it
  replaces native progress bars; standalone consumers keep terminal
  output. Transfer counts never imply a successful final ACK. Short transfers
  emit no progress rows, and longer transfers preserve their last byte count
  on both success and error. Sahara completion and Firehose version messages
  become neutral diagnostics. Existing transport and write-start callbacks
  are unchanged.

- **Preserve every byte when dumping storage** (`src/lib.rs`).
  `firehose_read_storage` uses `write_all` for each received chunk before
  advancing its byte count. A short output write is retried, while a zero-length
  write or output error aborts the dump instead of silently discarding bytes.
  Regression tests live in `crates/ltbox-device/tests/firehose_read.rs` so they
  run with the LTBox workspace tests.
- **Drop the redundant explicit ZLP in `firehose_program_storage`**
  (`src/lib.rs`). The USB `Write` impl already terminates every transfer
  via `EndpointWrite::submit_end()` — a zero-length packet when the
  payload is a multiple of the bulk max-packet size, a short packet
  otherwise. The extra explicit `channel.write(&[])` put a second, stray
  zero-length OUT transfer on the wire; after a packet-aligned partition
  Firehose has already byte-counted all its sectors and stops reading the
  OUT endpoint, so that stray ZLP stalls the next `<program>` write
  indefinitely (the endpoint write timeout does not cancel the queued
  transfer). Symptom: a multi-partition flash hung on the partition after
  the first packet-aligned one (e.g. `xbl_config_a`, 245760 B = exact
  512-multiple). Upstream still sends this ZLP, so this is a deliberate
  behavioural divergence that must be re-applied on every re-sync.
- **Make the serial backend tolerant enough for Qualcomm kernel-driver mode**
  (`src/serial.rs`). LTBox opens the port with an identity configuration,
  applies raw mode + 115200 baud best-effort, and sets explicit read/write
  timeouts. This keeps the serial path usable when the user selects Qualcomm's
  kernel driver family while avoiding hard failure on hosts whose serial
  driver rejects one of the advisory termios settings.
- **Add `firehose_program_storage_with_progress`** (`src/lib.rs`). Additive
  API that accepts a `FnMut(u64, u64)` callback `(completed_bytes,
  total_bytes)`. Existing `firehose_program_storage` delegates to it with a
  no-op callback, preserving behavior and terminal `pbr` output. Reports `0`
  after the device ACKs `<program>`, then again after each successful chunk
  write. LTBox needs this for structured, cross-platform per-partition flash
  progress in the GUI without scraping terminal progress-bar text.

- **Terminate every `pbr` progress bar with a newline** (`src/lib.rs`,
  `src/sahara.rs`). `pbr` redraws its single row with `\r` and never emits a
  trailing newline, so upstream leaves the finished 100% bar as an unterminated
  line: the next log message lands on the same row. In the GUI that also broke
  log dedup — the stdout tap flushed `<bar><our line>` as one entry while the
  in-process live sink emitted `<our line>` alone, and the two no longer
  compared equal. `firehose_program_storage_with_progress`,
  `firehose_read_storage`, and `sahara_dump_region` now call
  `pb.finish_println("")` when their transfer loop ends.

- **Reject ambiguous USB selection** (`src/usb.rs`). Enumerate every matching
  EDL candidate before opening a handle, including duplicate serial matches.
  Multiple candidates require the user to disconnect other devices. Propagate
  enumeration errors instead of panicking.

## Updating

To re-sync with upstream: re-copy `src/` + `Cargo.toml` from the desired
`qualcomm/qdlrs` revision (library crate only; skip CLI files), then
re-apply the patches above. Update the revision recorded here.

After re-syncing, compare the complete local diff against the recorded upstream
revision. Check the callback and final-response handling, USB/serial selection,
and programmer-loading support against LTBox callers; this list alone is not a
complete inventory of every local change.

Run `cargo test -p qdl --lib --locked` for the vendored unit tests (excluded
from `--workspace`), then `cargo test -p ltbox-device --locked` from the LTBox workspace, including
`firehose_faults`, `firehose_read`, and `edl::partition_flash` tests. Confirm
that preflight rejects later invalid images before any write/erase, a missing
or delayed final ACK prevents the next write, and an error stops the batch.
Do not treat the untracked standalone vendor build directory as workspace test
evidence. Real-device serial/USB checks remain a separate release check.
