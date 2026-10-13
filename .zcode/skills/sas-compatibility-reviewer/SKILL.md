---
name: sas-compatibility-reviewer
description: Verify and improve sasrs SAS 9.4-style behavior using independent reference oracles, minimal reproducers, conformance fixtures and precise regression tests.
---

# Review SAS semantics and conformance

1. Read `CONTRIBUTING.md`, `docs/support-contract.md`, `conformance/STATUS.md` and the relevant parser/execution/tests. Follow the active consolidation plan; do not use deprecated `sasrs-impl` or the frozen M1–M66 roadmap as a work queue.
2. For a specific DATA/PROC/SQL/macro/format/ODS behavior, write a **minimal SAS reproducer** and list expected dataset, types, missing values, order, log, listing, diagnostics and return code.
3. Obtain the expected behavior from an **independent source** (cited SAS documentation, actual SAS 9.4 output or separately implemented oracle). A self-generated snapshot proves stability, never SAS conformity.
4. Implement or repair the smallest behavior gap and add regression fixtures. Unsupported or recognized-but-ignored constructs must yield the documented ERROR/WARNING rather than a silent fallback. Never overwrite a known-divergence oracle to make tests pass.
5. Execute the appropriate `scripts/check.sh` suite, deterministic snapshots with `INSTA_UPDATE=no`, and corpus checks. Follow `CONTRIBUTING.md` snapshot-justification requirements.
6. Report verified vs merely implemented vs documented approximation vs unsupported, evidence and outstanding gaps in French.

Do not change generated plan views, publish releases or commit without explicit instruction.
