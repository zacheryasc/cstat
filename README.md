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