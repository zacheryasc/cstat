use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const CPP_SCORE_VERSION: &str = "cpp_code_complexity_cost_v1";
const API_SOURCE: &str = include_str!("fixtures/cpp_contract/api.cpp");
const API_HEADER: &str = include_str!("fixtures/cpp_contract/api.hpp");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("cstat-cpp-reports-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => panic!("create {}: {err}", path.display()),
            }
        }
    }

    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, source).unwrap();
        path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_cpp(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cstat"))
        .args(["--no-color", "--language", "cpp", "--path"])
        .arg(path)
        .args(args)
        .output()
        .expect("invoke cstat binary")
}

fn stdout(output: &Output, success: bool) -> &str {
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "status={}\nstderr={}\nstdout={text}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    text
}

fn report(path: &Path, args: &[&str], success: bool) -> Value {
    let mut json_args = vec!["--json"];
    json_args.extend_from_slice(args);
    let output = run_cpp(path, &json_args);
    serde_json::from_str(stdout(&output, success)).expect("parse C++ JSON")
}

fn complete(path: &Path, args: &[&str]) -> Value {
    let result = report(path, args, true);
    assert_eq!(result["language"], "cpp");
    assert_eq!(result["analysis_mode"], "source_syntax");
    assert_eq!(result["status"], "complete");
    assert_eq!(result["diagnostics"], json!([]));
    result
}

fn rows<'a>(document: &'a Value, key: &str) -> &'a [Value] {
    document[key]
        .as_array()
        .unwrap_or_else(|| panic!("missing array {key}: {document}"))
}

fn named<'a>(rows: &'a [Value], name: &str) -> &'a Value {
    let mut matches = rows.iter().filter(|row| row["name"] == name);
    let first = matches.next().unwrap_or_else(|| panic!("missing {name}"));
    assert!(matches.next().is_none(), "expected only one occurrence of {name}");
    first
}

fn assert_body(row: &Value, decisions: u64, nesting: u64, statements: u64) {
    assert_eq!(row["decisions"], decisions, "{row}");
    assert_eq!(row["path_count"], decisions + 1, "{row}");
    assert_eq!(row["max_nesting"], nesting, "{row}");
    assert_eq!(row["statements"], statements, "{row}");
}

#[test]
fn declarations_are_surface_and_lambdas_own_their_bodies() {
    let project = fixture("cpp_contract");
    let symbols = complete(&project, &["symbols"]);
    let symbols = rows(&symbols, "symbols");
    for (name, declarations, definitions) in [
        ("demo::Widget::Widget", 2, 1),
        ("demo::Widget::~Widget", 2, 0),
        ("demo::Widget::operator()", 2, 2),
        ("demo::Widget::operator bool", 1, 0),
    ] {
        let mut counts = [0, 0];
        for row in symbols.iter().filter(|row| row["name"] == name) {
            assert_eq!(row["named_surface"], true);
            match row["role"].as_str().unwrap() {
                "declaration" => counts[0] += 1,
                "definition" => counts[1] += 1,
                other => panic!("unexpected role {other} for {row}"),
            }
        }
        assert_eq!(counts, [declarations, definitions], "{name}");
    }
    let lambdas: Vec<_> = symbols.iter().filter(|row| row["kind"] == "lambda").collect();
    assert_eq!(lambdas.len(), 2);
    assert!(lambdas.iter().all(|row| row["role"] == "definition" && row["named_surface"] == false));
    let outer = lambdas[0]["name"].as_str().unwrap();
    let inner = lambdas[1]["name"].as_str().unwrap();
    assert!(outer.starts_with("demo::Widget::operator()::"));
    assert!(inner.strip_prefix(outer).is_some_and(|tail| tail.starts_with("::")));

    let branching = complete(&project, &["branching"]);
    let bodies = rows(&branching, "functions");
    assert_eq!(bodies.len(), 9);
    assert!(bodies.iter().all(|row| row["role"] == "definition"));
    let ctor = named(bodies, "demo::Widget::Widget");
    assert_body(ctor, 1, 1, 0);
    assert_eq!(ctor["categories"]["conditional_expressions"], 1);
    assert_body(named(bodies, outer), 1, 1, 4);
    assert_eq!(named(bodies, outer)["categories"]["if_statements"], 1);
    assert_body(named(bodies, inner), 1, 1, 1);
    assert_eq!(named(bodies, inner)["categories"]["conditional_expressions"], 1);

    let operators: Vec<_> = bodies.iter().filter(|row| row["name"] == "demo::Widget::operator()").collect();
    assert_eq!(operators.len(), 2);
    assert_eq!(operators[0]["span"]["line_start"], 16);
    assert_body(operators[0], 9, 3, 13);
    assert_eq!(operators[0]["categories"], json!({
        "if_statements": 2, "for_loops": 1, "range_for_loops": 0,
        "while_loops": 0, "do_while_loops": 0, "case_labels": 2,
        "default_labels": 1, "catch_clauses": 1, "conditional_expressions": 0,
        "logical_and": 1, "logical_or": 1,
    }));
    assert_eq!(operators[1]["span"]["line_start"], 38);
    assert_body(operators[1], 0, 0, 1);

    let spans = complete(&project, &["span"]);
    let ctor = named(rows(&spans, "functions"), "demo::Widget::Widget");
    assert_eq!(ctor["body_span"], json!({
        "byte_start": API_SOURCE.find(": bias_").unwrap(),
        "byte_end": API_SOURCE.find("\nWidget::~Widget").unwrap(),
        "line_start": 7, "line_end": 7,
    }));

    let score = complete(&project, &["scorecard"]);
    assert_eq!(score["inputs"]["bodies"], 9);
    assert_eq!(score["inputs"]["decisions"], 12);
    assert_eq!(score["inputs"]["statements"], 23);
}

#[test]
fn branching_counts_loop_kinds_and_keeps_init_captures_with_the_owner() {
    let project = TempProject::new();
    let source = project.write("walk.cpp", r#"
int walk(int n, const int (&xs)[2]) {
    struct Local { int run(int x) { return x ? x : 0; } };
    auto f = [x = n ? n : 1] { return x || 0; };
    if (n and f()) {
        for (int x : xs) {
            while (x or n) {
                do { --n; } while (n > 0);
                break;
            }
        }
    } else if (n) {
        goto done;
    } else {
        ++n;
    }
done:
    return n;
}
"#);
    let branching = complete(&source, &["branching"]);
    let bodies = rows(&branching, "functions");
    assert_eq!(bodies.len(), 3);
    let walk = named(bodies, "walk");
    assert_body(walk, 8, 4, 12);
    assert_eq!(walk["categories"], json!({
        "if_statements": 2, "for_loops": 0, "range_for_loops": 1,
        "while_loops": 1, "do_while_loops": 1, "case_labels": 0,
        "default_labels": 0, "catch_clauses": 0, "conditional_expressions": 1,
        "logical_and": 1, "logical_or": 1,
    }));
    let method = named(bodies, "walk::Local::run");
    assert_body(method, 1, 1, 1);
    assert_eq!(method["categories"]["conditional_expressions"], 1);
    let lambda = bodies.iter().find(|row| row["kind"] == "lambda").unwrap();
    assert_body(lambda, 1, 0, 1);
    assert_eq!(lambda["categories"]["logical_or"], 1);
}

#[test]
fn signatures_preserve_qualified_occurrences_and_count_spelled_types() {
    let project = fixture("cpp_contract");
    let signatures = complete(&project, &["signature"]);
    let signatures = rows(&signatures, "functions");
    let mut operator_identities = Vec::new();
    for row in signatures.iter().filter(|row| row["name"] == "demo::Widget::operator()") {
        let file = row["file"].as_str().unwrap();
        let span = &row["span"];
        let start = span["byte_start"].as_u64().unwrap() as usize;
        let end = span["byte_end"].as_u64().unwrap() as usize;
        let text = match file {
            "api.cpp" => API_SOURCE,
            "api.hpp" => API_HEADER,
            other => panic!("unexpected file for Widget overload: {other}"),
        };
        let spelling = &text[start..end];
        let overload = if spelling.contains("(int value)") {
            "int"
        } else {
            assert!(spelling.contains("(double value)"), "{spelling}");
            "double"
        };
        operator_identities.push((
            file,
            span["line_start"].as_u64().unwrap(),
            row["role"].as_str().unwrap(),
            overload,
        ));
        assert_eq!(row["parameters"], 1);
        assert_eq!(row["type_complexity"], 0);
    }
    assert_eq!(operator_identities, [
        ("api.cpp", 16, "definition", "int"),
        ("api.cpp", 38, "definition", "double"),
        ("api.hpp", 12, "declaration", "int"),
        ("api.hpp", 13, "declaration", "double"),
    ]);

    let select = named(signatures, "demo::select");
    assert_eq!(select["role"], "declaration");
    assert_eq!(select["parameters"], 0);
    assert_eq!(select["type_complexity"], 2); // Returning a function pointer.
    assert!(signatures.iter().all(|row| row["name"] != "demo::callback"));
    for (name, parameters, constraints) in [("demo::twice", 1, 1), ("demo::twice<int>", 0, 0)] {
        let row = named(signatures, name);
        assert_eq!(row["role"], "definition");
        assert_eq!(row["parameters"], 1);
        assert_eq!(row["template_heads"], 1);
        assert_eq!(row["template_parameters"], parameters);
        assert_eq!(row["constraints"], constraints);
    }

    let temp = TempProject::new();
    let source = temp.write("types.hpp", r#"
template <typename T> struct Box;
void zero(void);
void legacy(int n, ...);
template <typename... Ts> void forward_all(Ts&&... args);
auto adapt(int (*callback)(int), const Box<Box<int>>& value) -> int*;
"#);
    let details = complete(&source, &["signature"]);
    let details = rows(&details, "functions");
    for (name, parameters, variadic, packs, types) in [
        ("zero", 0, false, 0, 0),
        ("legacy", 2, true, 0, 0),
        ("forward_all", 1, false, 1, 1),
        ("adapt", 2, false, 0, 6),
    ] {
        let row = named(details, name);
        assert_eq!(row["role"], "declaration");
        assert_eq!(row["parameters"], parameters, "{row}");
        assert_eq!(row["variadic"], variadic, "{row}");
        assert_eq!(row["parameter_packs"], packs, "{row}");
        assert_eq!(row["type_complexity"], types, "{row}");
    }
}

#[test]
fn comma_free_c_variadics_cost_the_same_as_comma_spelling() {
    let project = TempProject::new();
    for parameters in ["int, int, int...", "int, int, int, ..."] {
        let source = project.write("count.cpp", &format!("int count({parameters}) {{ return 0; }}\n"));
        let signatures = complete(&source, &["signature"]);
        let count = named(rows(&signatures, "functions"), "count");
        assert_eq!(count["parameters"], 4, "{count}");
        assert_eq!(count["variadic"], true, "{count}");
        assert_eq!(count["parameter_packs"], 0, "{count}");

        let score = complete(&source, &["scorecard"]);
        assert_eq!(score["components"]["function_complexity_cost"], 3);
        assert_eq!(score["code_complexity_cost"], 4);
    }

    let source = project.write("packs.hpp", "\
template<class... Ts> void unnamed(Ts...);
void abbreviated(auto&&...);
void referenced(int&...);
");
    let signatures = complete(&source, &["signature"]);
    let functions = rows(&signatures, "functions");
    for name in ["unnamed", "abbreviated"] {
        let pack = named(functions, name);
        assert_eq!(pack["parameters"], 1, "{pack}");
        assert_eq!(pack["variadic"], false, "{pack}");
        assert_eq!(pack["parameter_packs"], 1, "{pack}");
    }
    let reference = named(functions, "referenced");
    assert_eq!(reference["parameters"], 2, "{reference}");
    assert_eq!(reference["variadic"], true, "{reference}");
    assert_eq!(reference["parameter_packs"], 0, "{reference}");
    assert_eq!(reference["type_complexity"], 1, "{reference}");
}

#[test]
fn qualified_type_definitions_do_not_assume_public_member_access() {
    let project = TempProject::new();
    let source = project.write("hidden.hpp", "class Outer { struct Hidden; };\nstruct Outer::Hidden {};\n");
    let symbols = complete(&source, &["symbols"]);
    let hidden: Vec<_> = rows(&symbols, "symbols").iter()
        .filter(|row| row["name"] == "Outer::Hidden")
        .map(|row| (row["role"].as_str().unwrap(), row["access"].as_str().unwrap()))
        .collect();
    assert_eq!(hidden, [("declaration", "private"), ("definition", "unknown")]);
    let score = complete(&source, &["scorecard"]);
    assert_eq!(score["files"][0]["inputs"]["eligible_header_occurrences"], 1);
    assert_eq!(score["code_complexity_cost"], 4);

    let source = project.write("namespace.hpp", "namespace api { struct Item; }\nstruct api::Item {};\n");
    let symbols = complete(&source, &["symbols"]);
    let item = rows(&symbols, "symbols").iter()
        .find(|row| row["name"] == "api::Item" && row["role"] == "definition")
        .expect("qualified namespace type definition");
    assert!(item["access"].is_null(), "{item}");
}

#[test]
fn only_uninitialized_variables_directly_wrapped_by_linkage_are_declarations() {
    let project = TempProject::new();
    let source = project.write("linkage.hpp", "\
extern \"C\" int ec;
extern \"C\" int initialized = 1, declared;
extern \"C\" { int ed; extern int eb; }
");
    let symbols = complete(&source, &["symbols"]);
    let variables = rows(&symbols, "symbols");
    for (name, role) in [
        ("ec", "declaration"), ("initialized", "definition"), ("declared", "declaration"),
        ("ed", "definition"), ("eb", "declaration"),
    ] {
        let row = named(variables, name);
        assert_eq!(row["kind"], "variable", "{row}");
        assert_eq!(row["role"], role, "{row}");
    }
}

#[test]
fn variable_roles_use_all_storage_and_type_specifiers() {
    let project = TempProject::new();
    let source = project.write("variables.hpp", "\
struct S {
    static int decl;
    static const int c = 1;
    static constexpr int k = 2;
    inline static int a, b = 3;
    static inline int reversed;
    int member = 4;
};
thread_local extern int ext;
extern thread_local int ext2;
thread_local extern int initialized = 5, declared;
thread_local int local_storage;
");
    let document = complete(&source, &["symbols"]);
    let symbols = rows(&document, "symbols");
    for (name, kind, role) in [
        ("S::decl", "field", "declaration"), ("S::c", "field", "declaration"),
        ("S::k", "field", "definition"), ("S::a", "field", "definition"),
        ("S::b", "field", "definition"), ("S::reversed", "field", "definition"),
        ("S::member", "field", "declaration"),
        ("ext", "variable", "declaration"), ("ext2", "variable", "declaration"),
        ("initialized", "variable", "definition"), ("declared", "variable", "declaration"),
        ("local_storage", "variable", "definition"),
    ] {
        let row = named(symbols, name);
        assert_eq!(row["kind"], kind, "{row}");
        assert_eq!(row["role"], role, "{row}");
    }
}

#[test]
fn explicit_class_instantiations_are_occurrences_without_callable_bodies() {
    let project = TempProject::new();
    let source = "\
template<class T> struct C {};
template struct C<int>;
extern template struct C<long>;
";
    let file = project.write("instantiation.cpp", source);
    let document = complete(&file, &["symbols"]);
    let symbols = rows(&document, "symbols");
    for (name, role, spelling) in [
        ("C<int>", "definition", "template struct C<int>;"),
        ("C<long>", "declaration", "extern template struct C<long>;"),
    ] {
        let row = named(symbols, name);
        assert_eq!(row["kind"], "struct", "{row}");
        assert_eq!(row["role"], role, "{row}");
        let start = row["span"]["byte_start"].as_u64().unwrap() as usize;
        let end = row["span"]["byte_end"].as_u64().unwrap() as usize;
        assert_eq!(&source[start..end], spelling);
    }
    let score = complete(&file, &["scorecard"]);
    assert_eq!(score["files"][0]["inputs"]["bodies"], 0);
}

#[test]
fn selected_file_loc_and_spans_use_physical_crlf_source() {
    const SOURCE: &str = concat!(
        "// lead\r\n",
        " \t\r\n",
        "int measure()\r\n",
        "{ // mixed\r\n",
        "    /* first\r\n",
        "       second */\r\n",
        "    const char* text = R\"x(// raw\r\n",
        "/* also raw */\r\n",
        ")x\";\r\n",
        "    return text[0]; /* mixed */\r\n",
        "}\r\n",
    );
    let project = TempProject::new();
    let source = project.write("physical.cpp", SOURCE);
    let loc = complete(&source, &["loc", "--top", "0"]);
    assert_eq!(loc["loc_mode"], "physical");
    assert_eq!(loc["selected_files"], json!(["physical.cpp"]));
    assert_eq!(loc["totals"], json!({
        "total_lines": 11, "code_lines": 7, "comment_lines": 3, "blank_lines": 1,
    }));
    assert_eq!(loc["files"], json!([{
        "path": "physical.cpp", "total_lines": 11, "code_lines": 7,
        "comment_lines": 3, "blank_lines": 1,
    }]));
    assert!(stdout(&run_cpp(&source, &["loc"]), true).lines().any(|line| {
        line.contains("physical.cpp:") && line.contains("11 physical lines")
    }));

    let span = complete(&source, &["span"]);
    let function = named(rows(&span, "functions"), "measure");
    let end = SOURCE.rfind('}').unwrap() + 1;
    assert_eq!(function["span"], json!({
        "byte_start": SOURCE.find("int measure").unwrap(), "byte_end": end,
        "line_start": 3, "line_end": 11,
    }));
    assert_eq!(function["body_span"], json!({
        "byte_start": SOURCE.find('{').unwrap(), "byte_end": end,
        "line_start": 4, "line_end": 11,
    }));
    assert_eq!(function["body_lines"], 8);
    assert_eq!(function["statements"], 2);
}

fn assert_not_comparable(score: &Value, status: &str) {
    assert_eq!(score["language"], "cpp");
    assert_eq!(score["analysis_mode"], "source_syntax");
    assert_eq!(score["score_version"], CPP_SCORE_VERSION);
    assert_eq!(score["status"], status);
    for key in ["code_complexity_cost", "components", "scopes"] {
        assert!(score.get(key).is_none(), "unexpected comparable field {key}: {score}");
    }
}

#[test]
fn malformed_and_ordinary_empty_selections_do_not_produce_comparable_scores() {
    // Unreadable inputs were smoke-tested in Stage 4. chmod-based tests would
    // depend on the privileges of the test runner.
    let broken = fixture("cpp_malformed");
    let score = report(&broken, &["scorecard"], false);
    assert_not_comparable(&score, "incomplete");
    assert_eq!(score["selected_files"], json!(["broken.cpp"]));
    assert_eq!(score["file_counts"], json!({"selected": 1, "parsed": 0, "failed": 1}));
    let diagnostics = rows(&score, "diagnostics");
    for (line, column, kind) in [(2, 25, "missing"), (3, 30, "syntax_error")] {
        assert!(diagnostics.iter().any(|row| row["file"] == "broken.cpp"
            && row["line"] == line && row["column"] == column && row["kind"] == kind), "{score}");
    }

    let symbols = report(&broken, &["symbols"], false);
    assert_eq!(symbols["status"], "incomplete");
    assert_eq!(symbols["diagnostics"], score["diagnostics"]);
    let recovered = named(rows(&symbols, "symbols"), "after");
    assert_eq!(recovered["role"], "definition");
    assert_eq!(recovered["span"]["line_start"], 4);
    let human = run_cpp(&broken, &["scorecard"]);
    let human = stdout(&human, false);
    assert!(human.contains("incomplete") && human.contains("broken.cpp:2:25"));
    assert!(!human.contains("Code complexity cost:"));

    let empty = TempProject::new();
    let score = report(&empty.0, &["scorecard"], false);
    assert_not_comparable(&score, "empty");
    assert_eq!(score["selected_files"], json!([]));
    assert_eq!(score["file_counts"], json!({"selected": 0, "parsed": 0, "failed": 0}));
    assert_eq!(score["diagnostics"], json!([]));
}

#[test]
fn packing_source_lines_does_not_reduce_structural_score() {
    let project = TempProject::new();
    project.write("choose.cpp", "int choose(int x, int y) {\n    if (x && y) {\n        return x ? y : 0;\n    }\n    return 0;\n}\n");
    let mut expanded = complete(&project.0, &["scorecard"]);
    project.write("choose.cpp", "int choose(int x,int y){if(x&&y){return x?y:0;}return 0;}\n");
    let mut packed = complete(&project.0, &["scorecard"]);
    assert_eq!(expanded["score_version"], CPP_SCORE_VERSION);
    assert_eq!(packed["score_version"], CPP_SCORE_VERSION);
    assert_eq!(expanded["code_complexity_cost"], 15);
    assert_eq!(packed["code_complexity_cost"], expanded["code_complexity_cost"]);
    assert_eq!(packed["components"], expanded["components"]);
    let expanded_loc = expanded["inputs"].as_object_mut().unwrap().remove("loc").unwrap();
    let packed_loc = packed["inputs"].as_object_mut().unwrap().remove("loc").unwrap();
    assert_eq!(expanded_loc["code_lines"], 6);
    assert_eq!(packed_loc["code_lines"], 1);
    assert_eq!(packed["inputs"], expanded["inputs"]);
}

#[test]
fn scorecard_is_deterministic_and_top_only_truncates_ranked_contributors() {
    let project = TempProject::new();
    for name in ["api.cpp", "api.hpp", "cycle.hpp"] {
        fs::copy(fixture("cpp_contract").join(name), project.0.join(name)).unwrap();
    }
    // Two nonzero concentration contributors so each ranking is actually truncated.
    let many_bodies: String = (0..13).map(|i| format!("static int f{i}() {{ return {i}; }}\n")).collect();
    project.write("many_a.cpp", &many_bodies);
    project.write("many_b.cpp", &many_bodies);

    let full = run_cpp(&project.0, &["--json", "scorecard"]);
    let repeat = run_cpp(&project.0, &["--json", "scorecard"]);
    assert_eq!(stdout(&full, true), stdout(&repeat, true));
    let mut full: Value = serde_json::from_slice(&full.stdout).unwrap();
    assert_eq!(full["status"], "complete");
    assert_eq!(full["score_version"], CPP_SCORE_VERSION);
    assert_eq!(rows(&full["includes"], "edges").len(), 3);
    assert_eq!(rows(&full["includes"], "cyclic_components").len(), 1);

    let full_contributors = full.as_object_mut().unwrap().remove("contributors").unwrap();
    for limit in ["0", "1"] {
        let mut limited = complete(&project.0, &["scorecard", "--top", limit]);
        let limited_contributors = limited.as_object_mut().unwrap().remove("contributors").unwrap();
        assert_eq!(limited, full, "--top {limit} changed more than contributor lists");
        let limit: usize = limit.parse().unwrap();
        for name in ["function_complexity_cost", "file_concentration_cost", "declaration_surface_cost", "include_coupling_cost"] {
            let ranking = rows(&full_contributors, name);
            assert!(ranking.len() > 1, "fixture must exercise truncating {name}");
            assert_eq!(rows(&limited_contributors, name), &ranking[..limit], "{name}");
        }
    }
}

#[test]
fn repeated_namespace_qualifiers_keep_their_lexical_owner() {
    let project = TempProject::new();
    let file = project.write("qualified.hpp", "\
namespace n {
    int f() { return 0; }
    namespace n { int f(); }
    int n::f() { return 1; }
    namespace api { int g(); int h(); }
    namespace n::api { int g(); }
    int n::api::g() { return [] { return 2; }(); }
    int ::n::api::g() { return 3; }
    namespace alternate { int h(); }
    int n::alternate::h() { return 4; }
}
");
    let document = complete(&file, &["symbols"]);
    let symbols = rows(&document, "symbols");
    for (name, expected) in [
        ("n::f", vec![(2, "definition")]),
        ("n::n::f", vec![(3, "declaration"), (4, "definition")]),
        ("n::api::g", vec![(5, "declaration"), (8, "definition")]),
        ("n::n::api::g", vec![(6, "declaration"), (7, "definition")]),
        ("n::alternate::h", vec![(9, "declaration"), (10, "definition")]),
    ] {
        let occurrences: Vec<_> = symbols.iter().filter(|row| row["name"] == name).map(|row| {
            assert_eq!(row["kind"], "function", "{row}");
            assert!(row["access"].is_null(), "{row}");
            (row["span"]["line_start"].as_u64().unwrap(), row["role"].as_str().unwrap())
        }).collect();
        assert_eq!(occurrences, expected, "{name}");
    }
    assert!(symbols.iter().any(|row| {
        row["kind"] == "lambda" && row["name"].as_str().unwrap().starts_with("n::n::api::g::<lambda@")
    }));
}

#[test]
fn compact_inline_namespace_qualifiers_name_the_same_nonmember_scope() {
    let project = TempProject::new();
    let file = project.write("inline.hpp", "\
namespace outer::inline middle::inline v1 { void f(); }
void outer::middle::v1::f() {}
");
    let document = complete(&file, &["symbols"]);
    let symbols = rows(&document, "symbols");
    assert_eq!(named(symbols, "outer::middle::v1")["kind"], "namespace");
    let functions: Vec<_> = symbols.iter().filter(|row| row["kind"] == "function").collect();
    assert_eq!(functions.len(), 2);
    for (row, role) in functions.into_iter().zip(["declaration", "definition"]) {
        assert_eq!(row["name"], "outer::middle::v1::f");
        assert_eq!(row["role"], role);
        assert!(row["access"].is_null(), "{row}");
    }
    let score = complete(&file, &["scorecard"]);
    assert_eq!(score["components"]["declaration_surface_cost"], 6);
    assert_eq!(score["code_complexity_cost"], 7);
}

#[test]
fn qualified_owners_prefer_same_file_and_do_not_pick_a_cross_file_conflict() {
    let project = TempProject::new();
    project.write("z.hpp", "namespace Lib { int f(); void g(); }\ninline int Lib::f() { return 0; }\n");
    project.write("use.cpp", "#include \"z.hpp\"\nvoid Lib::g() {}\n");
    let document = complete(&project.0, &["symbols"]);
    let fallback = rows(&document, "symbols").iter()
        .find(|row| row["name"] == "Lib::g" && row["role"] == "definition").unwrap();
    assert_eq!(fallback["kind"], "function");
    assert!(fallback["access"].is_null(), "{fallback}");

    let record = project.write("a.cpp", "struct Lib { static int f(); };\nint Lib::f() { return 0; }\n");
    for record_file in ["a.cpp", "zz.cpp"] {
        if record_file == "zz.cpp" {
            fs::rename(&record, project.0.join(record_file)).unwrap();
        }
        let document = complete(&project.0, &["symbols"]);
        let symbols = rows(&document, "symbols");
        for (file, name, kind, access) in [
            ("z.hpp", "Lib::f", "function", Value::Null),
            (record_file, "Lib::f", "method", json!("unknown")),
            ("use.cpp", "Lib::g", "function", json!("unknown")),
        ] {
            let row = symbols.iter().find(|row| row["file"] == file
                && row["name"] == name && row["role"] == "definition").unwrap();
            assert_eq!(row["kind"], kind, "{row}");
            assert_eq!(row["access"], access, "{row}");
        }
        let score = complete(&project.0, &["scorecard"]);
        let header = rows(&score, "files").iter().find(|row| row["path"] == "z.hpp").unwrap();
        assert_eq!(header["components"]["declaration_surface_cost"], 8);
    }
}

#[test]
fn namespace_structured_bindings_contribute_each_name_at_its_own_span() {
    let project = TempProject::new();
    let source = "struct Pair { int a; int b; };\nauto [x, /* binding */ y] = Pair{};\n";
    let file = project.write("bindings.hpp", source);
    let document = complete(&file, &["symbols"]);
    let variables: Vec<_> = rows(&document, "symbols").iter()
        .filter(|row| row["kind"] == "variable")
        .map(|row| {
            assert_eq!(row["role"], "definition");
            assert_eq!(row["named_surface"], true);
            let span = &row["span"];
            let start = span["byte_start"].as_u64().unwrap() as usize;
            let end = span["byte_end"].as_u64().unwrap() as usize;
            assert_eq!(row["name"], &source[start..end]);
            row["name"].as_str().unwrap()
        }).collect();
    assert_eq!(variables, ["x", "y"]);

    let score = complete(&file, &["scorecard"]);
    assert_eq!(score["files"][0]["inputs"]["eligible_header_occurrences"], 5);
    assert_eq!(score["components"]["declaration_surface_cost"], 10);
    assert_eq!(score["code_complexity_cost"], 10);
}

#[cfg(unix)]
#[test]
fn raw_path_identities_join_across_cpp_reports() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let project = TempProject::new();
    let headers = ["%25FE.hpp", "%FE.hpp", "%FF.hpp"];
    for raw in [b"%FE.hpp".as_slice(), b"\xfe.hpp", b"\xff.hpp"] {
        fs::write(project.0.join(OsStr::from_bytes(raw)), "int same() { return 0; }\n").unwrap();
    }
    let source = project.0.join(OsStr::from_bytes(b"src-\xfd.cpp"));
    fs::write(&source, b"#include \"%FE.hpp\"\n#include \"\xfe.hpp\"\n#include \"\xff.hpp\"\n").unwrap();
    let source_id = "src-%FD.cpp";
    let files = [headers[0], source_id, headers[1], headers[2]];
    let score = complete(&project.0, &["scorecard"]);
    assert_eq!(score["selected_files"], json!(files));
    for (container, key) in [(&score, "files"), (&score["includes"], "nodes")] {
        let paths: Vec<_> = rows(container, key).iter().map(|row| &row["path"]).collect();
        assert_eq!(json!(paths), json!(files), "{key}");
    }
    let functions = rows(&score["contributors"], "function_complexity_cost");
    assert_eq!(functions.iter().map(|row| row["file"].as_str().unwrap()).collect::<Vec<_>>(), headers);
    let edges = rows(&score["includes"], "edges");
    assert_eq!(edges.iter().map(|row| row["target"].as_str().unwrap()).collect::<Vec<_>>(), headers);
    assert!(edges.iter().all(|row| row["source"] == source_id));
    let human = run_cpp(&project.0, &["scorecard"]);
    let human = stdout(&human, true);
    for header in headers {
        assert!(human.contains(&format!("{header} [")));
        assert!(human.contains(&format!("{header}:1–1 same")));
        assert!(human.contains(&format!("Edge {source_id:?} -> {header:?}")));
    }

    let loc = complete(&source, &["loc"]);
    assert_eq!(loc["selected_files"], json!([source_id]));
    assert_eq!(loc["files"][0]["path"], source_id);
    fs::write(&source, b"int malformed( {\n").unwrap();
    let partial = report(&source, &["symbols"], false);
    assert_eq!(partial["status"], "incomplete");
    assert_eq!(partial["diagnostics"][0]["file"], source_id);
}

#[test]
fn opaque_noexcept_annotations_preserve_function_and_lambda_metrics() {
    let project = TempProject::new();
    let annotated = "\
#define EXAMPLE_ANNOTATION
int declared() noexcept EXAMPLE_ANNOTATION;
auto probe(int n) noexcept(true) EXAMPLE_ANNOTATION -> int {
    auto nested = [n]() noexcept EXAMPLE_ANNOTATION -> int { return n ? 1 : 0; };
    return nested();
}
";
    let file = project.write("annotated.cpp", annotated);
    let score = complete(&file, &["scorecard"]);
    let (_, declarations) = annotated.split_once('\n').unwrap();
    let plain = project.write("plain.cpp", &declarations.replace(" EXAMPLE_ANNOTATION", ""));
    let baseline = complete(&plain, &["scorecard"]);
    assert_eq!(score["components"], baseline["components"]);
    assert_eq!(score["inputs"]["bodies"], 2);
    assert_eq!(score["inputs"]["decisions"], 1);
    assert_eq!(score["inputs"]["named_surface"], 2);

    let malformed = project.write("malformed.cpp", "int bad() noexcept stray_identifier { return 0; }\n");
    let partial = report(&malformed, &["scorecard"], false);
    assert_eq!(partial["status"], "incomplete");
    assert!(partial.get("code_complexity_cost").is_none());
}

#[test]
fn offsetof_accepts_nested_member_paths_but_not_arbitrary_expressions() {
    let project = TempProject::new();
    let source = "\
struct Item { int value; };
struct Table { Item items[2]; };
int offset() { return offsetof(Table, items[1].value); }
";
    let file = project.write("offset.cpp", source);
    let score = complete(&file, &["scorecard"]);
    assert_eq!(score["inputs"]["bodies"], 1);
    let function = named(rows(&score["contributors"], "function_complexity_cost"), "offset");
    assert_body(function, 0, 0, 1);

    let malformed = project.write("malformed.cpp", &source.replace("items[1].value", "items + 1"));
    let partial = report(&malformed, &["scorecard"], false);
    assert_eq!(partial["status"], "incomplete");
    assert!(partial.get("code_complexity_cost").is_none());
}

#[test]
fn designated_braces_and_comma_decltype_keep_the_enclosing_body() {
    let project = TempProject::new();
    let file = project.write("designated.cpp", "\
struct Item { int value; };
struct Table { Item item; };
auto read(bool choose) -> decltype(void(), 0) {
    Table table{.item{.value = choose ? 1 : 0}};
    return table.item.value;
}
");
    let score = complete(&file, &["scorecard"]);
    let function = named(rows(&score["contributors"], "function_complexity_cost"), "read");
    assert_body(function, 1, 1, 2);
    assert_eq!(function["parameters"], 1);
}
