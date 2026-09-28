# Python retained story sessions — implemented, qualification bounded

Added over the existing uncommitted candidate on `main`, based on
`27e62db3a1b84804339e65b6025273fd003b3736`. The Python Rust extension compiles,
installs into the local Python 3.14 environment, and all five retained-session
tests pass. Its broader wheel/platform matrix and real encrypted-PDF binding workflow
remain unqualified; this is not completion of the full roadmap.

## Canonical routing and ownership

`wellfriendpdf.StoryEditSession` delegates all native commands to
`engine::story_session_protocol` and retains `LinkedStorySession`. Python does
not implement its own PDF tokenizer, font chooser, pagination or transaction.
Registration is in the existing PyO3 extension; no package/dependency was added.
The complete [shared command set](native_story_sessions.md) is available through
`command_json`. Named preview/checkpoint methods preserve raw request fields and
duplicate keys for the same typed parser; they validate a single JSON value
before wrapping it so trailing fields cannot escape into the command envelope.

The constructor accepts immutable bytes, validates the 256 MiB cap before native
copying, and retains independent PDF storage. JSON has the common 32 MiB limit.
Keyword-only `password=` accepts exact permissions/owner password Python
`bytes`; it is copied into
zeroizing Rust storage only for synchronous open. Encrypted input becomes the
explicitly reported unencrypted working revision, and the session retains no
password. Status distinguishes its original input hash from the working revision.
Output PDFs and explicit PNG renders become independent Python bytes. JSON
remains a UTF-8 string, avoiding a separate Python-specific document model.
The shared engine supplies exact-revision receipts, table synchronization,
source inventories, conservative merges, and bounded byte-exact undo/redo.
No receipt is minted automatically when a stale checkpoint is rejected.

## Concurrency and lifecycle

The frozen PyO3 class owns a mutex over open/closed/poisoned native state. Work
detaches from the interpreter before obtaining the mutex and releases it before
reattachment. `try_lock` gives a deterministic busy error instead of waiting on
another command or holding an interpreter lock while waiting for native work.
Sequential cross-thread calls are supported by Rust-owned state; concurrent
session use and close are rejected. The existing shared cancellation token may
be signalled from another Python thread. No Python callback is invoked natively.
This follows [PyO3's detachment guidance](https://pyo3.rs/v0.29.0/parallelism.html)
and [class thread-safety rules](https://pyo3.rs/v0.29.0/class.html); reading those
contracts is not evidence of binding execution or free-threaded wheel support.

Native unwinding panics discard the session and poison the handle; only close is
then useful. Ordinary input/cancellation failures do not poison it. Close is
idempotent, and context-manager exit does not suppress the caller's exception.
Outputs/commands cannot be used after close. No post-publication cancellation
check changes a successful checkpoint's result. This does not recover aborts,
allocation failures or provide immediate interruption of every native codec.

## Added regression source (not executed)

Four Python cases cover a real source operand through preview, changed-request
rejection, checkpoint, exact history, save/reopen and a second edit; cancellation
with receipt/revision preservation; lifecycle and sequential cross-thread use;
invalid commands, duplicate keys, wrapper injection, DPI/input-type bounds and
context-manager exception behavior. The fixture is generated in memory by the
test, not downloaded or dependent on system fonts. It uses approved bundled
fallback in the story request.

Two Rust regression sources exercise nonblocking busy errors, panic poisoning,
idempotent close, ordinary cancellation without poisoning and thread transfer.
These do not establish GIL release, native symbols or runtime success until the
exact revision is built and executed. Independent rendering and extraction,
real cross-thread cancellation timing, wheel/ABI/platform checks, complete
binding parity and production host integration remain pending. Credentialed
Standard-handler owner-password open is source/type integrated but still needs
Python execution against representative encrypted files.

## Checks performed

`cargo fmt --all`, `cargo check -p wellfriendpdf-py --lib --jobs 1`, and
`maturin develop` succeeded for the current source. The five focused
`test_story_session.py` cases pass on Python 3.14. Wheel ABI/platform matrices,
GIL behavior under broader concurrency, and encrypted binding execution remain
separate qualification work.
