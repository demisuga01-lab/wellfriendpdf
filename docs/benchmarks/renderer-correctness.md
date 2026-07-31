# Renderer correctness evidence

The final renderer campaign verifies operational completion on real PDFs: every selected file opened, every selected page rendered, and every per-page render respected the configured timeout/cancellation policy.

Final Wellfriend result: 5044/5044 files, 116975/116975 pages, 0 failures.

Correctness boundaries remain explicit:

- Compat mode uses bounded fallbacks for pathological Type3 and nested Form XObject tiling-pattern cases so rendering terminates under the corpus timeout.
- High-quality mode keeps the more exact pattern and image paths where supported.
- This run records raw pixel hashes, not full visual equivalence against every comparator.
- Independent comparator failures are not treated as Wellfriend wins unless the operation is equivalent and the artifact records the failure.
