# C++ source analysis and score v1

This is the contract for `--language cpp`. It does not change the Rust commands,
JSON schemas, or `code_complexity_cost_v2`. The C++ comparison scalar is
`cpp_code_complexity_cost_v1`; compare only complete runs with equivalent selection
rules. Lower is less structural cost, not proof of quality or behavior.

## Source and identity

- Parse the original bytes once per selected input using Tree-sitter 0.25.10 and
  the vendored Tree-sitter C++ 0.23.4-cstat.1 grammar (ABI 14). It patches
  explicit class template instantiations, braced parameter defaults, CRLF line
  splices, nested `offsetof` member designators, member operator expressions,
  C++20 designated braced initializers, and comma expressions in `decltype`.
  See `vendor/tree-sitter-cpp/regenerate.sh` for pinned sources and regeneration.
  No compiler, build, manifest, macro expansion,
  preprocessing configuration, template instantiation, or semantic lookup is
  required or implied. Include both conditional-compilation alternatives where
  the grammar exposes them; do not count directive conditions or scan macro
  replacement text for synthetic declarations or includes.
  A bare uppercase annotation token after a function or lambda's `noexcept`
  specifier is accepted as opaque syntax. It does not add a declaration or
  body metric. Macro definitions are not consulted; an annotation that
  expands to meaningful code is not analyzed as expanded code.
- Use relative path, zero-based half-open byte span, and qualified display name
  as the source identity. Also report one-based inclusive line spans (ending on
  the line of the last occupied byte) and one-based byte columns for diagnostics.
  Preserve repeated declarations, overloads, and explicit specializations as
  distinct occurrences. Normalize formatting between name tokens for display;
  a name alone is not an identity. Compose lexical namespace/class scopes
  with spelled qualifiers. An initial `::` anchors the name at global scope;
  repeated relative segments are preserved unless an earlier declaration in
  the same file identifies the shorter descendant owner and no nearer owner
  is observed. A compact inline namespace uses its identifiers, not `inline`,
  in the qualified name. An observed same-file owner takes precedence over
  owners in other inputs; conflicting cross-file owner categories are unknown.
  No include-driven or semantic name lookup is performed.
  Lambdas get a synthetic name qualified by their enclosing scope/callable,
  with a source location to distinguish siblings.
  Path identities use the same spelling across human and JSON reports and
  the include graph: percent-encode literal `%`, ASCII control bytes, and
  bytes outside valid UTF-8 as uppercase `%HH`.
- A function definition must have a body (including a function-try-block), a
  lambda must have its own body. A prototype, pure virtual, defaulted or deleted
  callable is not an implementation. Use declarator binding, not the presence
  of any parameter list: `int (*select())(int);` declares a function with zero
  parameters, whereas `int (*callback)(int);` declares an object. Keep explicit
  C variadics and C++ packs distinct from empty lists. Record named entities in
  their C++ terms, including constructor, destructor, conversion and other
  operators, method, and free function. Store template heads as metadata on the
  entity, not duplicate implementations for the wrapper.
  Explicit class instantiation statements record only the spelled class
  occurrence: `extern template` is a declaration, and the form without `extern`
  is an explicit instantiation definition with no source body.
  Count a comma-free C ellipsis after a built-in parameter type as a separate
  slot when the grammar exposes it as an unnamed pack-shaped declarator.
  Type-dependent spellings are not disambiguated by semantic lookup.
- Keep declaration-level and implementation-level data distinct. `symbols`
  reports source occurrences with kind and role (declaration or definition),
  including bodyless callables and lambdas. `branching` and `span` rank bodies.
  An uninitialized variable directly wrapped by a language-linkage specifier
  (such as `extern "C" int x;`) is a declaration; a linkage block alone does
  not make its variables declarations.
  `extern` order does not affect the role. Static data members defined inline
  (including C++17 `static constexpr`) are definitions; a non-inline static
  member in the class remains a declaration even with a constant initializer.
  `signature` shows declarator metrics for declarations, definitions, and
  lambdas with an explicit role. Constructor initialization and the function
  try/catch parts are in the implementation region. Lexical bodies nested in
  another callable never contribute their decisions, nesting, or statements to
  that outer callable. For a lambda with an init-capture, evaluating the
  capture initializer remains part of the enclosing expression; the lambda
  body is independent. Local class/enum definitions are not executable regions;
  body metrics do not inspect their members on behalf of the enclosing function.
- C++ `loc` reports physical lines, including on selected files; no reachability.
  Each LF- or CRLF-delimited line belongs to exactly one of blank, comment-only,
  or code. A trailing separator creates no extra line. Whitespace-only lines are
  blank; among nonblank lines, a line containing non-whitespace bytes outside
  Tree-sitter comment ranges is code (even if it also has a comment). A raw
  string or a preprocessor directive is source code, not a comment.

## Structural callable metrics

Measure each body locally, starting at decision depth zero. Walk implementation
nodes in source order. Skip nested callable bodies and local type-definition
regions as above; ignore preprocessor condition expressions but traverse the
branches the grammar parses. A lambda starts again at depth zero.

| Metric | Count |
| --- | --- |
| `decisions` (D) | One per `if` (including `if constexpr` when parsed); each ordinary/range `for`, `while`, and `do`; each `case` or `default` label (fallthrough still counts); each `catch`; each `?:`; and each binary `&&`/`and` or `\|\|`/`or`. `else` (without another `if`), `try`, and `switch` itself add no decision. Overloaded logical operators still count as spelled. Report categories as well as the total; the baseline path count is `1 + D`. |
| `max_nesting` (N) | Longest path through the syntax nesting of `if`, each loop, `switch`, `catch`, and `?:`, including the current node. Count these ancestors in conditions as well as bodies; an `else if` is a nested `if`. Blocks, `try`, switch labels, and short-circuit operators do not increase depth. |
| `statements` (S) | One per parsed control or executable statement (`if`, loop, `switch`, `try`, `return`/`co_return`, `co_yield`, `throw`, `break`, `continue`, `goto`, expression statement), plus block-scope declaration statements. Do not count compound blocks, labels, case clauses, catches, attribute wrappers, the declaration in a control initializer/condition, or the iteration/catch parameter as separate statements. A declaration with multiple declarators is one statement. Do not infer statements from semicolons or physical lines. |
| `parameters` (P) | Direct parameters of the callable declarator, including defaulted parameters and one slot per pack or C variadic ellipsis; sole unnamed, unqualified `void` is zero. Do not add again for a function-pointer parameter's own parameter list. A lambda with no list has zero. |
| `template_parameters` (T) | Direct parameter entries across heads that wrap this declarator (or the lambda's explicit template list). A template-template parameter is one entry, as is a pack; do not repeat a containing class's parameters for its inline methods. `template <>` is a head with zero entries. |
| `constraints` (R) | Count attached `requires` clauses in template heads and the declarator plus `requires` expressions inside them. A body `requires` expression is not a signature constraint; concept names used as types are not resolved. |
| `type_complexity` (Y) | Within spelled parameter and return types only, add one for each pointer, reference, array, or nested function declarator layer (including abstract forms), and one per argument of each template argument list, including nested lists. Exclude the callable's own function declarator, parameter defaults, template heads, constraints, and body. Base and qualified type names alone are zero. For a constructor/destructor no return type is invented. |

A report may also show the complete definition span, body span and physical body
lines, or lexical accessibility; these are not substitutes for structural counts.
For example, `fixtures/cpp_contract/api.cpp` has two `Widget::operator()` bodies.
The `int` overload owns nine decisions (two `if`, one `for`, three switch labels,
one `catch`, and two short-circuit operators), depth three, and 13 statements.
Its outer lambda owns one decision and four statements. The inner lambda owns
one ternary and one statement. The constructor owns a ternary in its initializer.
The header prototypes add no implementation cost. The constrained primary and
explicit specialization in the header each have one source body.

## Declaration and file metrics

- Named surface counts source occurrences at translation-unit, namespace and
  class scope: namespace blocks, class/struct/union/enum declarations and
  definitions, enumerators, aliases/typedefs, concepts, variables, named
  fields, and named callable declarations or definitions. A declarator list
  can have several named entries. At namespace scope a structured binding
  contributes one variable occurrence per bound identifier, at that
  identifier's span. Do not count anonymous entities, lambdas, parameters,
  block-local bindings, or macro replacement text as named surface.
  A named type nested inside a callable and its members are not surface;
  callable bodies there still count as implementations. No declaration-to-
  definition linking or de-duplication by name is needed. Count the header
  occurrence and source occurrence separately only in named surface when both
  exist; implementation cost belongs solely to the body.
- Report lexical member access (`public`, `protected`, `private`, or `unknown`
  when a qualified member cannot be established lexically), plus whether the
  occurrence is in a header. Apply the C++ defaults (class private; struct/union
  public) before explicitly spelled access specifiers. Nonmembers have no
  member access. A qualified out-of-class type definition does not inherit
  access from an earlier member declaration. Its access is unknown unless the
  qualifier names an observed namespace. Header surface is a syntactic review
  proxy, not ABI, export visibility, API stability, or proof of semantic
  accessibility. Eligible header occurrences get one surface point in addition
  to their base point; private or unknown members are ineligible. Header
  extensions are `.h`,
  `.hpp`, `.hh`, `.hxx`, `.ipp`, and `.tpp`.
- For each file let F be the number of bodies (including lambdas), S the sum of
  their own statements, and A the number of named surface occurrences. Report
  physical input and LOC counts separately. Score files once, not by includers.
- Label cost by nearest matching ancestor directory component relative to the
  analysis root: `test`/`tests` → `tests`; `bench`/`benches`/`benchmark`/
  `benchmarks` → `benchmarks`; otherwise `production`. Case-sensitive; a file
  name or a test framework does not imply scope. All scopes have equal weight.

## Include inputs and weights

Report the sorted selected files and effective selection rules (analysis root,
selected-file/directory/changed mode, base if changed, extensions, Git ignores,
excludes, configured include roots). Recognize existing eligible C++ files under
that root as possible internal targets even if not selected by `--changed` or
selected-file mode. A file excluded by selection rules is not an internal target.

Directory mode uses the requested directory as the root. Selected-file mode
uses the enclosing Git worktree root, or the file's containing directory
outside Git. In a worktree, tracked sources remain eligible even if an ignore
pattern matches them; standard Git ignores filter untracked sources. Other
worktrees, such as submodules, are not walked by the parent worktree's selection.
For an explicitly selected file, match the extension, excludes, Git ignores,
and changed paths against the requested leaf name, not the target of a symlink.
`--exclude` globs are relative to the analysis root; `*` does not cross path
separators, `**` can, and matching a directory excludes its descendants.
If Git reports a changed path as deleted but its existence cannot be checked
due to permissions, the result is incomplete rather than empty. Permission
errors in explicitly excluded directories do not invalidate the selection.
`--include-dir` must name an existing directory and is relative to the analysis
root unless absolute. Include roots do not add inputs.

For a quoted literal, try the including file's directory first. If there is no
file there, search all configured `--include-dir` roots. For an angle literal,
search only those roots; do not implicitly add compiler system paths. Search each
tier by spelled relative path, never by a basename index. Remove line splices
from literal names for lookup only, retaining the source spelling in reports.
Resolve `..` through the filesystem before applying target exclusions; do not
lexically collapse a directory symlink to an unrelated path. Within a tier dedupe
candidates identifying the same physical file; multiple distinct hits are
`ambiguous`, not an edge with an arbitrarily chosen target. A unique existing hit
outside the eligible input universe is `external`; no hit is `unresolved` (do
not assume every `<...>` is external). Nonliteral includes are `macro`. Every
parsed directive gets a status and location, including duplicate directives;
none of the last four statuses creates an internal graph edge.

Only selected inputs contribute outgoing directives. Internally resolved
`(source, target)` pairs form a simple directed graph; report unique edges, each
node's fan-in and fan-out, and cyclic strongly connected components (including
self-edges). A selected subgraph need not show cycles through files it did not
read. Charge the source file for its unique outgoing edges, its fan-out excess,
and each of its outgoing edges inside a cyclic component. Do not invent a
cohesion term, infer calls, or score unresolved text as if it were a dependency.

## Formula and comparability

All components are nonnegative integers. Define `excess(x, k) = max(0, x - k)`.

```text
function_complexity_cost = sum_bodies(1 + 2D + N² + S + excess(P, 3)² + T + 2R + Y)
file_concentration_cost  = sum_files(excess(F, 12)² + excess(S, 80)² + excess(A, 24)²)
declaration_surface_cost = sum_named_occurrences(1 + eligible_header_occurrence)
include_coupling_cost    = 2 * unique_internal_edges
                         + sum_files(excess(internal_fan_out, 4)²)
                         + 3 * unique_internal_edges_in_cycles
code_complexity_cost     = sum of the four component totals
```

Report component input metrics, absolute costs, and scope costs attributable to
the owning file; no physical-LOC normalization. Blank lines, reindentation,
comments and packing statements together do not reduce cost if parsed structure
and selection are unchanged. Keep sorted files, diagnostics, and edges; order
ranked contributors by descending cost, then relative path, byte span and name
(or path pairs for edges). `--top` truncates ranked contributors only, never
changes score, scope, input, include, or diagnostic totals.

## Result and failure contract

- Report `language: "cpp"`, `analysis_mode: "source_syntax"`, selection,
  input counts (selected, parsed cleanly, failed), diagnostics and the
  syntax-only limitations in JSON and their equivalent in human output. A
  complete result has `status: "complete"`. Scorecard JSON additionally reports
  `score_version: "cpp_code_complexity_cost_v1"`, the scalar and components,
  include outcomes/graph, contributors, and scope breakdown. No Rust-only keys
  or zero-valued Rust placeholders.
- Emit sorted file/line/column diagnostics for actual Tree-sitter `ERROR` and
  `MISSING` nodes (including anonymous missing punctuation), a parse with no
  tree, and file read or discovery errors affecting selection. A recovered
  node is not a clean parse. I/O or syntax failure makes the result
  `status: "incomplete"`, exit nonzero; recoverable observations in non-score
  reports may be shown as partial. Scorecard omits `code_complexity_cost` and
  does not present ordinary component/scope totals for an incomplete run.
- Report a valid empty selection explicitly as `status: "empty"` with no
  comparable scalar. No matching ordinary inputs is an error; a valid Git
  `--changed` selection with no existing C++ inputs exits zero, without claiming
  a complete analysis. Invalid paths, bases, or arguments are errors, not empty.

### Basic reports

The five reports share the result metadata above. `selected_files` lists the
root-relative inputs, including failed inputs. On incomplete runs, rows and
totals describe only the observations that could be recovered.

- `loc`: `loc_mode: "physical"`, `totals`, and `files`, ranked by code lines
  then path. `--top` limits the directory ranking only; it leaves totals and
  `selected_files` unchanged and is ignored for selected-file LOC.
- `symbols`: `counts_by_kind` and `symbols`, in file and source order. Each
  occurrence indicates whether it belongs to named surface.
- `branching`, `signature`, and `span`: `functions` contains source occurrences
  ranked by decisions, the sum of signature counts (P+T+R+Y), and physical
  body lines, respectively. Ties are broken by path, byte span, and name.
  Branching reports branch categories, paths, nesting, and statements.
  Signature includes declarations and distinguishes C variadics from packs.
  Span reports the full occurrence span, implementation region, and statements.

### Scorecard output

Complete JSON reports aggregate `inputs` and `components`, and repeats those
fields under each selected `files` row and each `scopes` entry. `inputs.loc`
describes physical source lines and is not part of the formula. The four
`contributors` lists rank function bodies, concentrated files, named surface
occurrences, and the files owning include coupling. Only these lists are
limited by `--top`.

`includes.directives` preserves each occurrence and its status (`internal`,
`ambiguous`, `external`, `unresolved`, or `macro`), source span and column,
spelling, and effective-tier candidates. Only internal outcomes name a graph
target. `includes.nodes` reports fan-in, fan-out, and cyclic outgoing edges.
`includes.edges` lists unique directed edges and whether each belongs to a
cycle; `includes.cyclic_components` lists the member files of each cycle
component. These fields describe literal source dependencies, not compiler
resolution. An I/O error that prevents establishing a lookup outcome makes the
score incomplete rather than falsely unresolved.

### Scorecard examples

Complete scorecard JSON excerpt for a single header containing
`int increment(int n) { return n + 1; }` and no directives:

```json
{
  "language": "cpp",
  "analysis_mode": "source_syntax",
  "status": "complete",
  "score_version": "cpp_code_complexity_cost_v1",
  "file_counts": {"selected": 1, "parsed": 1, "failed": 0},
  "diagnostics": [],
  "code_complexity_cost": 4,
  "components": {
    "function_complexity_cost": 2,
    "file_concentration_cost": 0,
    "declaration_surface_cost": 2,
    "include_coupling_cost": 0
  }
}
```

Incomplete scorecard JSON excerpt (the absence of the scalar is intentional):

```json
{
  "language": "cpp",
  "analysis_mode": "source_syntax",
  "status": "incomplete",
  "score_version": "cpp_code_complexity_cost_v1",
  "file_counts": {"selected": 1, "parsed": 0, "failed": 1},
  "diagnostics": [
    {"file": "broken.cpp", "line": 2, "column": 25, "kind": "missing", "message": "missing ;"},
    {"file": "broken.cpp", "line": 3, "column": 30, "kind": "syntax_error", "message": "unexpected syntax"}
  ]
}
```

## Real-project validation

Release `cstat` on Linux x86-64, against a local [OCUDU](https://gitlab.com/ocudu/ocudu)
working tree at HEAD `c8519743731f4c2e57021fbeb79eeacac8c3ac87`. Timings are
single runs with output redirected to files; peak RSS and wall time measured
with `busybox time`.

```sh
cstat scorecard --language cpp --path ../ocudu --include-dir include \
  --exclude 'build/**' --exclude 'external/**' --top 3 --json
cstat scorecard --language cpp --path ../ocudu/lib/gateways \
  --include-dir ../../include --top 3 --json
```

| Selection | Clean before grammar patch | Clean with patch | Selected | Result with patch |
| --- | ---: | ---: | ---: | --- |
| OCUDU root, exclusions above | 5,445 | 5,596 | 5,760 | Incomplete |
| `lib/gateways` | 19 | 19 | 19 | Complete; cost 167,542 |
| `lib/rlc` | 26 | 26 | 31 | Incomplete |
| `lib/support` | 44 | 47 | 48 | Incomplete |
| `lib/scheduler` | 233 | 243 | 247 | Incomplete |
| `include/ocudu` | 1,484 | 1,535 | 1,562 | Incomplete |

All five basic reports also completed in both formats on `lib/gateways`.
The complete score included 4,003 physical lines; include reporting found
25 internal, 46 external, and 48 unresolved directive occurrences. The top
function contributor was `sctp_network_client_impl::connect` (cost 170).
The patched root run reported 1,907 parser diagnostics (previously 3,895) over
164 failed files, in 20.12 s with 158,036 KiB peak RSS. Incomplete runs still
withhold the scalar. Macro-dependent declarations and split preprocessor
branches can still cause parse errors; this is not a measure of compiler
validity or general C++ compatibility.

Accepting opaque `noexcept` annotations raised root coverage to 5,615 of
5,760 files (1,867 diagnostics), and `lib/rlc` coverage from 26 to 30 of 31
files. The gateways score stayed at 167,542.

The standard-syntax fixes raised root coverage to 5,669 of 5,760 files
(367 diagnostics in 91 files), with no new failing files. `lib/asn1` parsed
74 of 75 files; the one remaining failure uses declaration annotation macros.
Remaining root failures include declaration/coroutine/test macros, directives
splitting syntactic constructs, and unsupported C++ syntax such as some
pointer-to-member type declarators. The gateways score was unchanged.
