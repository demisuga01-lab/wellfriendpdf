# Release gate checklist

- Clean, pushed roadmap baseline and recorded start state.
- VPS-only heavy validation within 32 GiB Wellfriend budget.
- Formatting, diff checks, workspace check/clippy/tests, and package/binding gates.
- Public corpus provenance, stress outcomes, and no unclassified crash/hang/OOM.
- Dependency/license/vulnerability posture, SBOM, secret scan, and unsafe-boundary audit.
- API inventory and policy review for every supported binding.
- External-tool scorecard with unavailable tools explicitly marked.
- Final report, exact closure commit, and a clean worktree.

Passing the checklist means the released scope is evidence-backed. It does not
promise universal PDF compatibility or replace deployment-specific controls.
