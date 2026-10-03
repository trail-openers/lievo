# Benchmarks

These figures were measured on a pre-release build (`ff1ade0`, 2026-09-24) and are not expected to reproduce exactly on current releases; see the caveats below.

> Figures from benchmark run set b3, build `ff1ade0` (2026-09-24).

We benchmarked lievo against an agent's built-in tools on an anonymous 11,000-file commercial monorepo (Ruby on Rails backend, React frontend — lievo parses about 20% of the tracked files, so all tasks were scoped to the JavaScript portion). 435 runs: change-impact analysis n=30/arm, bug localization n=39/arm, multi-turn sustained work n=18/arm. Headless agents (Claude Sonnet), one pinned index, lievo build `ff1ade0` (2026-09-24); the benchmarked index was summarized via a generic OpenAI-compatible backend. Arms: baseline (Bash/Read/Grep/Glob, no index); lievo — those four tools plus lievo's MCP server, the shipping configuration; and lievo_only (lievo's MCP server only). Scoring: set-F1 of returned file paths against hand-authored gold derived from git history or hand-derived dependency closures. Comparisons are bootstrap 95% CIs on the difference; "tied" means the interval spans zero.

**Headline — the shipping configuration, on multi-turn sustained work:** built-ins + lievo vs built-ins alone scores F1 0.657 vs 0.550, delta +0.106 [+0.007, +0.206], significant, n=18/arm, at the highest recall of any arm, 0.980 vs baseline 0.958.

| Arm | Multi-turn F1 | Multi-turn resident context | Change-impact F1 |
| --- | --- | --- | --- |
| baseline (built-ins only) | 0.550 | 337,297 tokens | 0.951 |
| lievo (built-ins + lievo, shipping config) | **0.657** (+0.106 [+0.007, +0.206], significant) | −92,667 tokens [−205,049, +20,833] (tied) | 0.926 (−0.025 [−0.054, +0.002], tied) |
| lievo_only (lievo only) | 0.781 (baseline 0.550; +0.232 [+0.110, +0.328], significant) | 129,068 tokens vs baseline 337,297 (−207,353 [−304,133, −119,403], significant) | 0.765 (baseline 0.951; −0.186 [−0.224, −0.150], significant — worse than built-ins alone) |

The lievo_only row is a replacement configuration, not a recommendation: it wins multi-turn F1 and resident context, and it loses change-impact F1.

Across 18 multi-turn sessions the shipping configuration made 114 `lievo_explore` calls plus 477 built-in calls (Read 177, Grep 126, Bash 110, Glob 64), and its lievo call mix is near-identical to the lievo_only arm's: lievo is added to the built-ins rather than substituted for them.

The shipping configuration never loses significantly on any suite: change-impact F1 0.926 vs 0.951 (−0.025 [−0.054, +0.002], tied) and change-impact tokens −3,161 [−51,401, +47,631] (tied); bug localization F1 0.331 vs 0.371 (−0.041 [−0.193, +0.110], tied — a replicated null across three runs, n=26, n=39, n=39) and tokens +136,872 [−325,880, +612,437] (tied); multi-turn resident context −92,667 [−205,049, +20,833] (tied).

**Caveats, stated next to the numbers:**

- Single repository, single model (Claude Sonnet), JavaScript portion only.
- Bug localization is a replicated null.
- The multi-turn gold set is directory-shaped (it favours recall for every arm — the metric that matters there is resident context at equal recall).
- n=18–39 per arm; ballpark figures with stated intervals, not a peer-reviewed study.
- Measured on build `ff1ade0` (2026-09-24) — later builds changed the retrieval and response contract, so these figures are not expected to reproduce on v0.1.0.
- The raw data is not published.
