# Crypto Standards Fuzz VPS testing process

Crypto Standards Fuzz heavy validation runs on the VPS:

- SSH: `demisuga01@35.185.176.47`
- Allowed root: `/home/demisuga01/wellpdf`
- Results: `/home/demisuga01/wellpdf/results/crypto_standards_fuzz-<timestamp>/`
- Temp/source/cargo target: `/home/demisuga01/wellpdf/tmp/crypto_standards_fuzz-<timestamp>/`
- External corpus root: `/home/demisuga01/wellpdf/corpus/`

Environment:

```bash
export WELLPDF_TEST_ROOT=/home/demisuga01/wellpdf
export WELLPDF_RESULT_DIR=/home/demisuga01/wellpdf/results/crypto_standards_fuzz-<timestamp>
export WELLPDF_TMP_DIR=/home/demisuga01/wellpdf/tmp/crypto_standards_fuzz-<timestamp>
export CARGO_BUILD_JOBS=1
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR=/home/demisuga01/wellpdf/tmp/crypto_standards_fuzz-<timestamp>/cargo-target
```

Budget:

- Wellfriend PDF SDK memory budget: 32 GiB.
- cargo-fuzz process cap: 16 GiB process-tree RSS per target for the user-approved
  Crypto Standards Fuzz run; ASan fuzz binaries are not constrained with virtual-address
  `RLIMIT_AS`.
- Full workspace gates use `--jobs 1`.
- No swap is available; memory failures are blockers.

The VPS is a test runner only. No deployment and no production services are touched.
