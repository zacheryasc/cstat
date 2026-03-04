# Plan: Compact dump output + judge cstat integration

## Overview

Two changes to make cstat work as a practical agent tool in the `.loop/` workflow:

1. **Rework `cstat dump`** from a 119KB full diagnostic blob into a compact ~3-5KB summary with drilldown hints — making it consumable in a single agent context read
2. **Update `judge.md`** so the judge uses cstat scores as part of code quality evaluation

---

## Stage 1: Rework `cstat dump` — compact summary format

The current `dump` output includes every diagnostic (314 items, 119KB for cstat itself). This is too large for an agent to consume in one read. Rework it into a compact machine summary that acts as an **index** — enough to understand the landscape, with pointers to drill deeper.

### New output structure:

```json
{
  "cstat_version": "0.1.0",
  "project": {
    "files": 18,
    "total_loc": 8768,
    "functions": 202,
    "modules": 18
  },
  "scores": {
    "modularity": 0.95,
    "complexity": 0.81,
    "maintainability": 0.78
  },
  "diagnostic_counts": {
    "total": 314,
    "alerts": 61,
    "warns": 253,
    "by_category": {
      "bloated_function": { "alerts": 14, "warns": 45 },
      "redundant_code": { "alerts": 23, "warns": 87 },
      "high_cognitive": { "alerts": 14, "warns": 34 },
      "high_complexity": { "alerts": 4, "warns": 26 },
      "deep_nesting": { "alerts": 2, "warns": 25 },
      "dead_code": { "alerts": 0, "warns": 20 },
      "bloated_file": { "alerts": 0, "warns": 10 },
      "low_cohesion": { "alerts": 3, "warns": 2 },
      "too_many_params": { "alerts": 0, "warns": 3 },
      "god_module": { "alerts": 1, "warns": 1 },
      "high_coupling": { "alerts": 0, "warns": 0 },
      "cyclic_deps": { "alerts": 0, "warns": 0 }
    }
  },
  "worst_items": [
    {
      "severity": "alert",
      "category": "bloated_function",
      "location": { "file": "src/diagnostics.rs", "function": "run_diagnostics", "lines": [30, 520] },
      "metric": 490.0,
      "threshold": 100.0,
      "context": { "...cross-referenced data..." },
      "message": "src/diagnostics.rs::run_diagnostics is 490 lines with composite complexity 77.0"
    }
  ],
  "suggested_drilldowns": [
    { "command": "cstat complexity --json --path .", "reason": "14 alert-level bloated functions, 14 alert-level high cognitive complexity" },
    { "command": "cstat redundancy --json --path .", "reason": "23 alert-level redundant code pairs" },
    { "command": "cstat deps --json --path .", "reason": "3 alert-level low cohesion modules, 1 alert-level god module" }
  ]
}
```

### Design intent:

- **`project`** and **`scores`**: kept as-is from current dump — these are already compact and useful
- **`diagnostic_counts`**: replaces the full `diagnostics` array. Shows counts by severity and category so the consumer knows the shape of the problem space without seeing every item
- **`worst_items`**: top 5 diagnostics by severity (alerts first) then by metric magnitude. These keep full detail including cross-referenced context. The consumer sees the most critical items immediately
- **`suggested_drilldowns`**: ranked list of `cstat <cmd> --json` commands, computed by mapping the worst diagnostic categories to their relevant subcommands. Each entry includes the command to run and a human-readable reason. This tells the consumer *where to look next*
- The full diagnostic data remains accessible via individual `--json` commands (`cstat complexity --json`, `cstat deps --json`, `cstat redundancy --json`, etc.)
- **`hotspots`** and **`topology`** sections from the current dump are removed — they overlap with the drilldown commands and contribute to the bloat

### Drilldown mapping logic:

The `suggested_drilldowns` field is computed by:
1. Counting alerts per category
2. Mapping categories to subcommands:
   - `bloated_function`, `high_complexity`, `high_cognitive`, `deep_nesting`, `too_many_params` → `cstat complexity --json`
   - `redundant_code` → `cstat redundancy --json`
   - `bloated_file` → `cstat loc --json`
   - `low_cohesion`, `high_coupling`, `god_module` → `cstat deps --json`
   - `dead_code` → `cstat datapaths --json`
   - `cyclic_deps` → `cstat graph --json`
3. Merging categories that map to the same command into one entry with a combined reason
4. Sorting by total alert count descending
5. Only including commands where there's at least 1 alert (warn-only categories don't generate drilldown suggestions)

### What stays the same:

- All individual `--json` commands continue to emit full detailed output (unchanged from current implementation)
- `cstat dump` still always emits JSON (no `--json` flag needed)
- `cstat summary` (the human command) is completely untouched

**Guard**: `cstat dump --path .` produces valid JSON under 10KB. It includes all 5 top-level keys: `cstat_version`, `project`, `scores`, `diagnostic_counts`, `worst_items`, `suggested_drilldowns`. The `worst_items` array has at most 5 entries. Each `suggested_drilldowns` entry includes both `command` and `reason` fields.

---

## Stage 2: Update judge.md with cstat evaluation

Add a section to `judge.md` that tells the judge to use cstat as part of its code quality evaluation. The judge already evaluates naming, readability, and API ergonomics — cstat provides quantitative backing for structural quality.

### Addition to judge.md:

After the existing "Judge it for the human" section, add a new section:

**3. Measure it**

Run `cstat dump --path .` and `cstat summary --path .` as part of evaluation. Use the metrics as evidence, not as automatic pass/fail criteria:

- **Scores**: Note the modularity, complexity, and maintainability scores. These provide a quantitative baseline. Significant degradation from reasonable levels is worth calling out.
- **Alerts**: Review the `worst_items` from dump. Alert-level diagnostics represent areas where metrics significantly exceed typical thresholds — these warrant inspection.
- **Structure**: Run `cstat summary` to visually inspect the codebase architecture. Do the dependency patterns, file sizes, and complexity distributions look reasonable for the project's scope?

The judge uses cstat output as **evidence** to support observations, not as a mechanical pass/fail gate. A project can have warn-level diagnostics and still PASS if the overall structure is sound. Conversely, clean metrics don't guarantee PASS if the code has other problems (bad naming, poor error handling, etc.).

**Guard**: `judge.md` contains a "Measure it" section that references `cstat dump` and `cstat summary`. The section frames metrics as evidence, not automatic criteria.
