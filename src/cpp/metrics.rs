use serde::Serialize;
use tree_sitter::Node;

#[derive(Debug, Default, Serialize)]
pub(super) struct BodyMetrics {
    pub decisions: usize,
    pub path_count: usize,
    pub max_nesting: usize,
    pub statements: usize,
    pub categories: BranchCategories,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct BranchCategories {
    pub if_statements: usize,
    pub for_loops: usize,
    pub range_for_loops: usize,
    pub while_loops: usize,
    pub do_while_loops: usize,
    pub case_labels: usize,
    pub default_labels: usize,
    pub catch_clauses: usize,
    pub conditional_expressions: usize,
    pub logical_and: usize,
    pub logical_or: usize,
}

pub(super) fn body_metrics(owner: Node<'_>, body: Node<'_>) -> BodyMetrics {
    let mut metrics = BodyMetrics::default();
    if owner.kind() == "function_definition" {
        let mut cursor = owner.walk();
        for child in owner.named_children(&mut cursor) {
            if child.kind() == "field_initializer_list" {
                metrics.add_region(child);
            }
        }
    }
    // A constructor's try-block may be an unfielded child of the definition.
    let region = body
        .parent()
        .filter(|parent| parent.kind() == "try_statement" && parent.parent() == Some(owner))
        .unwrap_or(body);
    metrics.add_region(region);
    metrics.path_count = 1 + metrics.decisions;
    metrics
}

impl BodyMetrics {
    fn add_region(&mut self, root: Node<'_>) {
        let mut depth = 0;
        let mut directive_condition = None;
        walk(root, |node, entering| {
            let kind = node.kind();
            let nests = increases_depth(kind);
            if !entering {
                depth -= usize::from(nests);
                return true;
            }
            if directive_condition == Some(node) {
                directive_condition = None;
                return false;
            }
            if !node.is_named() || node.is_missing() {
                return false;
            }

            match kind {
                "function_definition" => return false,
                // The remaining lambda child, `lambda_capture_specifier`, belongs
                // to the enclosing expression. The lambda's own region is its body.
                "compound_statement"
                | "template_parameter_list"
                | "requires_clause"
                | "abstract_function_declarator"
                | "ERROR"
                    if node != root && parent_kind(node) == Some("lambda_expression") =>
                {
                    return false;
                }
                "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                    // The grammar exposes a declaration without declarators as
                    // the specifier itself, rather than wrapping it in `declaration`.
                    if matches!(
                        parent_kind(node),
                        Some(
                            "compound_statement"
                                | "declaration_list"
                                | "preproc_if"
                                | "preproc_ifdef"
                                | "preproc_elif"
                                | "preproc_elifdef"
                                | "preproc_else"
                        )
                    ) {
                        self.statements += 1;
                    }
                    return false;
                }
                "preproc_if" | "preproc_elif" => {
                    directive_condition = node.child_by_field_name("condition");
                }
                "preproc_def" | "preproc_function_def" | "preproc_call" | "preproc_include" => {
                    return false;
                }
                _ => {}
            }

            let category = match kind {
                "if_statement" => Some(&mut self.categories.if_statements),
                "for_statement" => Some(&mut self.categories.for_loops),
                "for_range_loop" => Some(&mut self.categories.range_for_loops),
                "while_statement" => Some(&mut self.categories.while_loops),
                "do_statement" => Some(&mut self.categories.do_while_loops),
                "case_statement" if node.child(0).is_some_and(|token| token.kind() == "default") => {
                    Some(&mut self.categories.default_labels)
                }
                "case_statement" => Some(&mut self.categories.case_labels),
                "catch_clause" => Some(&mut self.categories.catch_clauses),
                "conditional_expression" => Some(&mut self.categories.conditional_expressions),
                "binary_expression" => match node.child_by_field_name("operator").map(|op| op.kind()) {
                    Some("&&" | "and") => Some(&mut self.categories.logical_and),
                    Some("||" | "or") => Some(&mut self.categories.logical_or),
                    _ => None,
                },
                _ => None,
            };
            if let Some(category) = category {
                *category += 1;
                self.decisions += 1;
            }

            if nests {
                depth += 1;
                self.max_nesting = self.max_nesting.max(depth);
            }
            self.statements += usize::from(is_statement(node, kind));
            true
        });
    }
}

fn parent_kind(node: Node<'_>) -> Option<&'static str> {
    node.parent().map(|parent| parent.kind())
}

fn increases_depth(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "for_statement"
            | "for_range_loop"
            | "while_statement"
            | "do_statement"
            | "switch_statement"
            | "catch_clause"
            | "conditional_expression"
    )
}

fn is_statement(node: Node<'_>, kind: &str) -> bool {
    match kind {
        "if_statement"
        | "for_statement"
        | "for_range_loop"
        | "while_statement"
        | "do_statement"
        | "switch_statement"
        | "try_statement"
        | "return_statement"
        | "co_return_statement"
        | "co_yield_statement"
        | "throw_statement"
        | "break_statement"
        | "continue_statement"
        | "goto_statement"
        | "expression_statement"
        | "seh_try_statement"
        | "seh_leave_statement" => true,
        "declaration"
        | "type_definition"
        | "alias_declaration"
        | "using_declaration"
        | "static_assert_declaration"
        | "namespace_alias_definition" => !matches!(
            parent_kind(node),
            Some("for_statement" | "condition_clause" | "init_statement")
        ),
        _ => false,
    }
}

#[derive(Debug, Default, Copy, Clone, Serialize)]
pub(super) struct Loc {
    pub total_lines: usize,
    pub code_lines: usize,
    pub comment_lines: usize,
    pub blank_lines: usize,
}

#[derive(Clone, Copy)]
enum LineKind {
    Blank,
    Comment,
    Code,
}

pub(super) fn physical_loc(source: &[u8], root: Node<'_>) -> Loc {
    let mut loc = Loc::default();
    let mut offset = 0;
    let mut line = LineKind::Blank;
    walk(root, |node, entering| {
        if !entering || node.kind() != "comment" {
            return true;
        }
        let range = node.byte_range();
        loc.scan(&source[offset..range.start], false, &mut line);
        offset = range.end;
        loc.scan(&source[range], true, &mut line);
        false
    });
    loc.scan(&source[offset..], false, &mut line);
    if source.last().is_some_and(|&byte| byte != b'\n') {
        loc.finish_line(line);
    }
    loc
}

impl Loc {
    pub(super) fn add(&mut self, other: &Self) {
        self.total_lines += other.total_lines;
        self.code_lines += other.code_lines;
        self.comment_lines += other.comment_lines;
        self.blank_lines += other.blank_lines;
    }

    fn scan(&mut self, bytes: &[u8], comment: bool, line: &mut LineKind) {
        for &byte in bytes {
            match byte {
                b'\n' => {
                    self.finish_line(*line);
                    *line = LineKind::Blank;
                }
                b'\t'..=b'\r' | b' ' => {}
                _ if !comment => *line = LineKind::Code,
                _ if matches!(line, LineKind::Blank) => *line = LineKind::Comment,
                _ => {}
            }
        }
    }

    fn finish_line(&mut self, line: LineKind) {
        self.total_lines += 1;
        match line {
            LineKind::Blank => self.blank_lines += 1,
            LineKind::Comment => self.comment_lines += 1,
            LineKind::Code => self.code_lines += 1,
        }
    }
}

// A false enter result prunes the subtree and omits the matching exit event.
fn walk<'tree>(root: Node<'tree>, mut visit: impl FnMut(Node<'tree>, bool) -> bool) {
    let mut cursor = root.walk();
    'enter: loop {
        let node = cursor.node();
        if visit(node, true) {
            if cursor.goto_first_child() {
                continue;
            }
            visit(node, false);
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'enter;
            }
            if !cursor.goto_parent() {
                return;
            }
            visit(cursor.node(), false);
        }
    }
}
