# cstat
Terminal CLI tool to examine a codebase through a statistical lens

## helpful commands
```bash
cstat --help # cli options
cstat scorecard --path . --json # machine-readable code complexity cost
cstat loc --path . -v # size-shape analysis plus explanations
cstat test-reachability --path . --json # structured static test/bench reachability
```

## Score changed files

```bash
cstat scorecard --path ../zebra --changed         # uncommitted changes
cstat scorecard --path ../zebra --changed HEAD~2  # changes since an older commit
```

`--changed` defaults to `HEAD`; the base can also be a commit, tag, or branch.
The scorecard reads current Rust files that differ from the base, plus untracked
Rust files not ignored by Git. It skips deleted files and does not stage anything
or check out the base. It scores whole files, not just the changed lines.

## C++ source analysis

Rust remains the default. Select C++ explicitly for a source directory or file;
no build, manifest, or compilation database is needed.

```sh
cstat scorecard --language cpp --path ../project --include-dir include --exclude 'generated/**' --top 10
cstat scorecard --language cpp --path ../project --changed HEAD~1 --json
cstat scorecard --language cpp --path ../project --changed HEAD~1 --diff-hunks --json
cstat branching --language cpp --path ../project/src/widget.cpp --json
```

Supported C++ reports: `loc`, `symbols`, `branching`, `signature`, `span`,
and `scorecard`, in human and JSON formats. C++ `loc` always counts physical
source lines, not reachability. `deps`, `call-trace`, `test-reachability`,
`coverage`, and `cluster` are Rust-only.

C++ discovery includes `.cpp`, `.cc`, `.cxx`, `.C`, `.h`, `.hpp`, `.hh`, `.hxx`,
`.ipp`, and `.tpp`. It honors Git ignores for untracked files and repeatable
root-relative `--exclude` globs. `--changed` scores whole existing changed and
nonignored untracked files, including headers once; it does not score their
dependents. `--include-dir` configures only literal include lookup and does
not add files to the selection.
Reported C++ path identities percent-encode `%`, ASCII control bytes, and
invalid UTF-8 bytes so file rows can be joined to include-graph paths.

Compare `cpp_code_complexity_cost_v1` only across complete C++ runs with
equivalent selection rules. The score measures structural function complexity,
file concentration, declaration surface, and directed include coupling.
Physical LOC does not affect it, and `--top` changes only ranked contributors.
Analysis is syntax-only: no macro expansion, active build configuration,
template instantiation, or overload resolution. Unreadable files or syntax
errors make the run incomplete and nonzero; the scorecard omits the scalar.
A valid empty `--changed` exits zero without a score.

Use `--changed [BASE] --diff-hunks` to measure only syntax anchored to Git's
new-side lines. It reports `cpp_diff_hunk_complexity_cost_v1`, not a whole-file
score or a score delta. New files are covered in full; deletions never subtract.
JSON lists the paths, added line ranges, and each charged construct.

See the [C++ score specification](notes/CPP_SCORE_SPEC.md) for metrics, include
resolution rules, and the JSON contract.