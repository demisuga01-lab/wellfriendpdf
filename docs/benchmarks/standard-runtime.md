# Standard runtime validation

The Standard runtime campaign validates:

- 2 vCPU / 6 GB minimum behavior;
- 4 vCPU / 8 GB recommended behavior;
- larger-host scaling within the 32 GiB validation budget.

Required evidence is stored in:

- `benchmarks/results/standard-runtime/minimum-2vcpu-6gb.json`
- `benchmarks/results/standard-runtime/recommended-4vcpu-8gb.json`
- `benchmarks/results/standard-runtime/scaling.json`

The VPS campaign under `/home/demisuga01/wellpdf/results/runtime-optimization-20260729T231614Z`
passed for all three profiles. The reports show Standard effective mode, no GPU
dependency, admitted governor and memory requests, unchanged correctness semantics,
and unchanged reopen contracts. These probes validate the runtime architecture
contract; they are not a market-performance comparison.
