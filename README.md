# cstat
Terminal CLI tool to examine a codebase through a statistical lens

## helpful commands
```bash
cstat --help # cli options
cstat scorecard --path . --json # machine-readable code complexity cost
cstat loc --path . -v # size-shape analysis plus explanations
cstat test-reachability --path . --json # structured static test/bench reachability
```