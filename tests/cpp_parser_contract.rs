use tree_sitter::{Node, Parser, Tree};

const HEADER: &str = include_str!("fixtures/cpp_contract/api.hpp");
const SOURCE: &str = include_str!("fixtures/cpp_contract/api.cpp");
const CYCLE: &str = include_str!("fixtures/cpp_contract/cycle.hpp");
const BROKEN: &str = include_str!("fixtures/cpp_malformed/broken.cpp");

fn parse(source: &str) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("pinned runtime supports grammar ABI");
    parser.parse(source, None).unwrap()
}

fn descendants(root: Node<'_>) -> Vec<Node<'_>> {
    let mut nodes = vec![];
    let mut cursor = root.walk();
    loop {
        nodes.push(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return nodes;
            }
        }
    }
}

fn of_kind<'tree>(nodes: &[Node<'tree>], kind: &str) -> Vec<Node<'tree>> {
    nodes.iter().copied().filter(|n| n.kind() == kind).collect()
}

fn text<'src>(node: Node<'_>, source: &'src str) -> &'src str {
    &source[node.byte_range()]
}

fn assert_clean(tree: &Tree, source: &str) {
    let root = tree.root_node();
    assert!(
        !root.has_error(),
        "{:?}\n{}",
        descendants(root)
            .into_iter()
            .filter(|n| n.is_missing() || n.is_error())
            .map(|n| (n.start_position(), n.kind(), text(n, source)))
            .collect::<Vec<_>>(),
        root.to_sexp(),
    );
}

#[test]
fn grammar_exposes_body_boundaries_overloads_and_specializations() {
    let header = parse(HEADER);
    let source = parse(SOURCE);
    assert_clean(&header, HEADER);
    assert_clean(&source, SOURCE);
    let h = descendants(header.root_node());
    let s = descendants(source.root_node());

    let header_bodies = of_kind(&h, "function_definition")
        .into_iter()
        .filter_map(|n| n.child_by_field_name("body"))
        .map(|n| text(n, HEADER))
        .collect::<Vec<_>>();
    assert_eq!(header_bodies, ["{ return value + value; }", "{ return value * 2; }"]);
    assert_eq!(of_kind(&h, "delete_method_clause").len(), 1);
    assert_eq!(of_kind(&s, "default_method_clause").len(), 1);
    assert_eq!(of_kind(&h, "operator_cast").len(), 1);
    assert_eq!(of_kind(&h, "requires_clause").len(), 1);
    assert_eq!(of_kind(&h, "requires_expression").len(), 1);
    assert_eq!(
        of_kind(&h, "template_declaration")
            .into_iter()
            .map(|n| text(n.child_by_field_name("parameters").unwrap(), HEADER))
            .collect::<Vec<_>>(),
        ["<typename T>", "<typename T>", "<>"]
    );

    let operators = |nodes: &[Node<'_>], input: &str| {
        of_kind(nodes, "function_declarator")
            .into_iter()
            .filter(|n| text(*n, input).contains("operator()"))
            .map(|n| text(n.child_by_field_name("parameters").unwrap(), input).to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(operators(&h, HEADER), ["(int value)", "(double value)"]);
    assert_eq!(operators(&s, SOURCE), ["(int value)", "(double value)"]);
    assert_eq!(of_kind(&s, "field_initializer_list").len(), 1);
    assert_eq!(
        of_kind(&s, "function_definition")
            .into_iter()
            .filter(|n| text(*n, SOURCE).starts_with("int configured("))
            .count(),
        2,
        "both conditional-compilation alternatives remain visible"
    );
}

#[test]
fn grammar_separates_lambdas_branch_forms_and_opaque_source() {
    let tree = parse(SOURCE);
    assert_clean(&tree, SOURCE);
    let nodes = descendants(tree.root_node());
    let lambdas = of_kind(&nodes, "lambda_expression");
    assert_eq!(lambdas.len(), 2);
    let outer = descendants(lambdas[0].child_by_field_name("body").unwrap());
    let inner = descendants(lambdas[1].child_by_field_name("body").unwrap());
    assert_eq!(of_kind(&outer, "lambda_expression"), [lambdas[1]]);
    assert_eq!(of_kind(&outer, "if_statement").len(), 1);
    assert_eq!(of_kind(&inner, "conditional_expression").len(), 1);
    assert_eq!(of_kind(&nodes, "conditional_expression").len(), 2);
    assert_eq!(of_kind(&nodes, "for_statement").len(), 1);
    assert_eq!(of_kind(&nodes, "catch_clause").len(), 1);
    assert_eq!(
        of_kind(&nodes, "case_statement")
            .into_iter()
            .map(|n| n.child_by_field_name("value").map(|v| text(v, SOURCE)))
            .collect::<Vec<_>>(),
        [Some("0"), Some("1"), None]
    );
    assert_eq!(
        of_kind(&nodes, "binary_expression")
            .into_iter()
            .filter_map(|n| n.child_by_field_name("operator"))
            .filter(|n| matches!(n.kind(), "&&" | "||"))
            .map(|n| n.kind())
            .collect::<Vec<_>>(),
        ["&&", "||"]
    );
    assert_eq!(of_kind(&nodes, "raw_string_literal").len(), 1);
    assert!(of_kind(&nodes, "comment").is_empty());
    assert_eq!(of_kind(&nodes, "preproc_def").len(), 1);

    let includes = |nodes: &[Node<'_>]| {
        of_kind(nodes, "preproc_include")
            .into_iter()
            .map(|n| n.child_by_field_name("path").unwrap().kind())
            .collect::<Vec<_>>()
    };
    assert_eq!(includes(&nodes), ["string_literal", "system_lib_string", "identifier"]);
    let cycle = parse(CYCLE);
    assert_clean(&cycle, CYCLE);
    let header = parse(HEADER);
    let cycle_edges = [(&header, HEADER), (&cycle, CYCLE)].map(|(tree, source)| {
        let includes = of_kind(&descendants(tree.root_node()), "preproc_include");
        assert_eq!(includes.len(), 1);
        text(includes[0].child_by_field_name("path").unwrap(), source)
    });
    assert_eq!(cycle_edges, ["\"cycle.hpp\"", "\"api.hpp\""]);
}

#[test]
fn grammar_exposes_braced_parameter_defaults() {
    let source = "\
struct Options { int value; };
int f(Options opts = {}) { return opts.value; }
struct Owner { explicit Owner(Options = {42}); };
int g(Options opts = Options{});
";
    let tree = parse(source);
    assert_clean(&tree, source);
    let nodes = descendants(tree.root_node());
    let defaults = of_kind(&nodes, "optional_parameter_declaration")
        .into_iter()
        .map(|node| {
            let value = node.child_by_field_name("default_value").unwrap();
            (value.kind(), text(value, source))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        defaults,
        [
            ("initializer_list", "{}"),
            ("initializer_list", "{42}"),
            ("compound_literal_expression", "Options{}"),
        ]
    );
    assert_eq!(of_kind(&nodes, "function_definition").len(), 1);
    assert!(parse("int f(int value = {);").root_node().has_error());
}

#[test]
fn grammar_exposes_class_and_function_template_instantiations() {
    let source = "\
namespace n { template<bool Enabled> struct C {}; }
template struct n::C<true>;
extern template struct n::C<false>;
template<class T> class D {};
template class D<char>;
extern template class D<long>;
template<class T> void call(T) {}
extern template void call(int);
template void call(double);
";
    let tree = parse(source);
    assert_clean(&tree, source);
    let nodes = descendants(tree.root_node());
    let instantiations = of_kind(&nodes, "template_instantiation");
    let types = instantiations
        .iter()
        .filter_map(|node| {
            let key = node.child_by_field_name("class_key")?;
            let name = node.child_by_field_name("name").unwrap();
            assert!(node.child_by_field_name("declarator").is_none());
            Some((key.kind(), text(name, source), node.child(0).unwrap().kind() == "extern"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        types,
        [
            ("struct", "n::C<true>", false),
            ("struct", "n::C<false>", true),
            ("class", "D<char>", false),
            ("class", "D<long>", true),
        ]
    );
    let functions = instantiations
        .iter()
        .filter_map(|node| node.child_by_field_name("declarator").map(|decl| text(decl, source)))
        .collect::<Vec<_>>();
    assert_eq!(functions, ["call(int)", "call(double)"]);
    assert!(parse("extern template struct ;").root_node().has_error());
}

#[test]
fn grammar_preserves_quoted_include_splice_bytes() {
    let source = "#include \"fi\\\r\nrst.hpp\"\r\n#include \"fi\\\nrst.hpp\"\n";
    let tree = parse(source);
    assert_clean(&tree, source);
    let nodes = descendants(tree.root_node());
    let paths = of_kind(&nodes, "preproc_include")
        .into_iter()
        .map(|node| text(node.child_by_field_name("path").unwrap(), source))
        .collect::<Vec<_>>();
    assert_eq!(paths, ["\"fi\\\r\nrst.hpp\"", "\"fi\\\nrst.hpp\""]);
}

#[test]
fn grammar_recovers_later_functions_without_hiding_error_or_missing_nodes() {
    let tree = parse(BROKEN);
    let root = tree.root_node();
    assert!(root.has_error());
    let nodes = descendants(root);
    assert!(nodes.iter().any(|n| n.is_missing() && n.start_position().row == 1));
    assert!(nodes.iter().any(|n| n.is_error() && n.start_position().row == 2));
    assert_eq!(
        of_kind(&nodes, "function_definition")
            .into_iter()
            .filter(|n| !n.has_error())
            .map(|n| text(n, BROKEN))
            .collect::<Vec<_>>(),
        ["int before() { return 1; }", "int after() { return 3; }"]
    );
}

#[test]
fn member_operators_keep_their_call_and_expression_precedence() {
    let source = "\
struct Value {
    int item;
    int operator[](int) const;
    int run(Value* other) {
        auto member = &Value::item;
        this->*member = 1;
        return 2 * other->*member + this->operator[](other->item);
    }
};
";
    let tree = parse(source);
    assert_clean(&tree, source);
    let nodes = descendants(tree.root_node());
    let member_operator = of_kind(&nodes, "field_expression").into_iter()
        .filter_map(|node| node.child_by_field_name("field"))
        .find(|field| field.kind() == "operator_name").unwrap();
    assert_eq!(text(member_operator, source), "operator[]");
    let multiplication = of_kind(&nodes, "binary_expression").into_iter()
        .find(|node| node.child_by_field_name("operator").is_some_and(|op| op.kind() == "*"))
        .unwrap();
    let member = multiplication.child_by_field_name("right").unwrap();
    assert_eq!(member.child_by_field_name("operator").unwrap().kind(), "->*");
    assert_eq!(text(member, source), "other->*member");
}
