# Initial indexed-query performance evidence

These are recorded observations, not throughput targets or performance claims.
Captured from [GitHub Actions run 38618365838](https://github.com/moritzbrantner/search-engine/actions/runs/38618365838)
on exact source revision `ca9984b9e6ffb717f59cd18b56ad53f5db481687`
(2026-10-14), before this evidence-only documentation change.

## Method

- Command: `cargo run --locked --example bench_indexed` (dev/debug profile).
- Host reported Linux x86-64, GitHub-hosted `ubuntu-latest`.
- Workload: deterministic mixed keyword/path/numeric file projections;
  query requires a unique file path and minimum width.
- Scenarios: 100, 1,000 and 5,000 source records, one cold index build
  followed by one reference query and one indexed query per scenario.
- Times are monotonic elapsed wall-clock nanoseconds from `Instant`,
  including query collector/materialization costs. No warm-up,
  repeated samples, hardware pinning or controlled-runner baseline.
- Candidate counter counts index-posting candidate documents materialized,
  not all Tantivy internal CPU work, memory, or all traversed posting entries.

| Indexed records | Index build (ns) | Reference query (ns) | Tantivy query (ns) | Indexed candidates |
|---:|---:|---:|---:|---:|
| 100 | 67,584,045 | 29,696 | 487,494 | 1 |
| 1,000 | 181,023,743 | 286,177 | 1,788,114 | 1 |
| 5,000 | 552,703,045 | 1,745,134 | 7,469,062 | 1 |

## Interpretation and next checks

The indexed retrieval path produced the same result as the independent
O(n) oracle and only materialized one candidate on each selective query.
On these small, debug-build workloads, Tantivy query time nevertheless
exceeded the reference oracle. Do **not** claim a speedup or use these
shared-runner timings as a blocking latency gate. Candidate work is
guarded separately by the deterministic selective-query integration test.

To make latency comparisons meaningful, repeat cold and warm queries in
release mode on controlled hardware, record compiler/CPU/tool versions,
and sample larger and unselective workloads. Keep microsecond timing and
candidate-count metrics distinct.
