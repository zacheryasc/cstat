use serde::Serialize;
use tree_sitter::Node;

#[derive(Clone, Copy)]
pub(super) struct Declarator<'tree> {
    pub name: Node<'tree>,
    pub function: Option<Node<'tree>>,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Signature {
    pub parameters: usize,
    pub template_parameters: usize,
    pub constraints: usize,
    pub type_complexity: usize,
    pub variadic: bool,
    pub parameter_packs: usize,
}

pub(super) fn declarator(mut node: Node<'_>) -> Option<Declarator<'_>> {
    let mut nearest = None;
    loop {
        match node.kind() {
            "qualified_identifier" => {
                let local = local_name(node)?;
                // In `C::*p`, the scope belongs to the pointer type, not to p.
                if local.kind() == "pointer_type_declarator" {
                    node = local;
                    continue;
                }
                return Some(Declarator {
                    name: node,
                    function: if local.kind() == "operator_cast" {
                        conversion_function(local)
                    } else {
                        nearest.filter(|layer: &Node<'_>| is_function(*layer))
                    },
                });
            }
            "operator_cast" => {
                return Some(Declarator {
                    name: node,
                    function: conversion_function(node),
                });
            }
            "identifier" | "field_identifier" | "type_identifier" | "namespace_identifier"
            | "primitive_type" | "destructor_name" | "operator_name" | "template_function"
            | "template_method" | "template_type" | "dependent_name"
            | "structured_binding_declarator" => {
                return (!node.is_missing()).then_some(Declarator {
                    name: node,
                    function: nearest.filter(|layer| is_function(*layer)),
                });
            }
            _ => {}
        }
        if is_type_layer(node) {
            nearest = Some(node);
        }
        node = inner_declarator(node)?;
    }
}

fn qualified_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    // A dependent qualified name can also have an anonymous `template` in this field.
    let name = node.children_by_field_name("name", &mut cursor)
        .find(|child| child.is_named());
    name
}

fn local_name(mut node: Node<'_>) -> Option<Node<'_>> {
    while node.kind() == "qualified_identifier" {
        node = qualified_child(node)?;
    }
    (!node.is_missing()).then_some(node)
}

fn inner_declarator(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("declarator").or_else(|| {
        // These wrappers have unfielded inner declarators in the pinned grammar.
        if matches!(
            node.kind(),
            "attributed_declarator"
                | "parenthesized_declarator"
                | "abstract_parenthesized_declarator"
                | "reference_declarator"
                | "abstract_reference_declarator"
                | "variadic_declarator"
        ) {
            let mut cursor = node.walk();
            let inner = node.named_children(&mut cursor).find(|child| {
                !child.is_extra()
                    && !child.is_error()
                    && !matches!(child.kind(), "ms_call_modifier" | "attribute_declaration")
            });
            inner
        } else {
            None
        }
    })
}

fn is_function(node: Node<'_>) -> bool {
    matches!(node.kind(), "function_declarator" | "abstract_function_declarator")
}

fn is_type_layer(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declarator"
            | "abstract_function_declarator"
            | "pointer_declarator"
            | "abstract_pointer_declarator"
            | "pointer_type_declarator"
            | "reference_declarator"
            | "abstract_reference_declarator"
            | "array_declarator"
            | "abstract_array_declarator"
    )
}

fn conversion_function(node: Node<'_>) -> Option<Node<'_>> {
    // A conversion name owns an abstract declarator containing its callable suffix.
    let mut function = None;
    let mut current = inner_declarator(node);
    while let Some(layer) = current {
        if is_function(layer) {
            function = Some(layer);
        }
        current = inner_declarator(layer);
    }
    function
}

pub(super) fn name_text(node: Node<'_>, source: &[u8]) -> String {
    let end = local_name(node)
        .filter(|name| name.kind() == "operator_cast")
        .and_then(conversion_function)
        .and_then(|function| function.child_by_field_name("parameters"))
        .map_or(node.end_byte(), |parameters| parameters.start_byte());
    let mut text = String::new();
    append_name_tokens(node, source, end, &mut text, &mut false);
    text
}

fn append_name_tokens(
    node: Node<'_>,
    source: &[u8],
    end: usize,
    text: &mut String,
    previous_template_close: &mut bool,
) {
    if node.start_byte() >= end || node.is_missing() || node.kind() == "comment"
        || (node.kind() == "inline"
            && node.parent().is_some_and(|parent| parent.kind() == "nested_namespace_specifier"))
    {
        return;
    }
    if node.child_count() != 0
        && !matches!(node.kind(), "string_literal" | "raw_string_literal" | "char_literal")
    {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            append_name_tokens(child, source, end, text, previous_template_close);
        }
        return;
    }
    let Some(bytes) = source.get(node.byte_range()) else {
        return;
    };
    let Some(&first) = bytes.first() else {
        return;
    };
    let template_close = node.kind() == ">"
        && node.parent().is_some_and(|parent| parent.kind() == "template_argument_list");
    if let Some(&last) = text.as_bytes().last() {
        let joins_words = word_byte(last) && word_byte(first);
        // Keep separate spelling tokens separate even in non-type template arguments.
        let joins_punctuation = matches!(
            (last, first),
            (b'+', b'+' | b'=')
                | (b'-', b'-' | b'=' | b'>')
                | (b'<' , b'<' | b'=' | b':' | b'%')
                | (b'>', b'>' | b'=')
                | (b'!' | b'=' | b'*' | b'^', b'=')
                | (b'&', b'&' | b'=')
                | (b'|', b'|' | b'=')
                | (b'/', b'/' | b'*' | b'=')
                | (b'%', b'=' | b'>' | b':')
                | (b':', b':')
                | (b'.', b'.' | b'*')
        ) && !(*previous_template_close && template_close);
        if joins_words || joins_punctuation {
            text.push(' ');
        }
    }
    text.push_str(&String::from_utf8_lossy(bytes));
    *previous_template_close = template_close;
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
}

pub(super) fn template_parameter_count(templates: &[Node<'_>]) -> usize {
    templates
        .iter()
        .filter_map(|node| node.child_by_field_name("parameters"))
        .map(template_list_count)
        .sum()
}

fn template_list_count(list: Node<'_>) -> usize {
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter(|child| {
            !child.is_missing()
                && matches!(
                    child.kind(),
                    "parameter_declaration"
                        | "optional_parameter_declaration"
                        | "variadic_parameter_declaration"
                        | "type_parameter_declaration"
                        | "optional_type_parameter_declaration"
                        | "variadic_type_parameter_declaration"
                        | "template_template_parameter_declaration"
                )
        })
        .count()
}

pub(super) fn signature(
    owner: Node<'_>,
    decl: Option<Declarator<'_>>,
    templates: &[Node<'_>],
    source: &[u8],
) -> Signature {
    let mut result = Signature {
        template_parameters: template_parameter_count(templates),
        constraints: templates.iter().copied().map(attached_constraints).sum(),
        ..Signature::default()
    };
    let function = if owner.kind() == "lambda_expression" {
        if let Some(list) = owner.child_by_field_name("template_parameters") {
            result.template_parameters += template_list_count(list);
        }
        result.constraints += attached_constraints(owner);
        owner.child_by_field_name("declarator")
    } else {
        decl.and_then(|decl| decl.function)
    };
    let Some(function) = function else {
        return result;
    };
    if let Some(parameters) = function.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        let mut sole_void = false;
        for parameter in parameters.children(&mut cursor).filter(|child| !child.is_missing()) {
            match parameter.kind() {
                "parameter_declaration" | "optional_parameter_declaration"
                | "variadic_parameter_declaration" => {
                    result.parameters += 1;
                    result.type_complexity += spelled_type_cost(parameter);
                    if parameter.kind() == "variadic_parameter_declaration" {
                        if has_comma_free_ellipsis(parameter) {
                            result.parameters += 1;
                            result.variadic = true;
                        } else {
                            result.parameter_packs += 1;
                        }
                    } else if result.parameters == 1 {
                        sole_void = is_unqualified_void(parameter, source);
                    }
                }
                "..." => {
                    result.parameters += 1;
                    result.variadic = true;
                }
                _ => {}
            }
        }
        if result.parameters == 1 && sole_void {
            result.parameters = 0;
        }
    }
    result.type_complexity += field_type_cost(owner, "type") + trailing_return_cost(function);
    result.constraints += attached_constraints(function);
    // The derived layers outside the callable are its return type, not its parameters.
    // Walking ancestors also isolates this declarator in a declaration with multiple names.
    let mut current = function.parent();
    while let Some(layer) = current.filter(|layer| *layer != owner) {
        result.constraints += attached_constraints(layer);
        result.type_complexity += layer_type_cost(layer);
        if layer.kind() == "operator_cast" {
            result.type_complexity += field_type_cost(layer, "type");
            result.type_complexity += inner_declarator(function).map_or(0, declarator_type_cost);
        }
        current = layer.parent();
    }
    result
}

fn has_comma_free_ellipsis(parameter: Node<'_>) -> bool {
    // The grammar also uses a pack node for `int...` and `int&...`.
    // A nameless ellipsis after a built-in type cannot expand a type pack.
    if !parameter.child_by_field_name("type").is_some_and(|ty| {
        matches!(ty.kind(), "primitive_type" | "sized_type_specifier")
    }) {
        return false;
    }
    let mut declarator = inner_declarator(parameter);
    while let Some(node) = declarator {
        match node.kind() {
            "variadic_declarator" => return inner_declarator(node).is_none(),
            "reference_declarator" => declarator = inner_declarator(node),
            _ => break,
        }
    }
    false
}

fn is_unqualified_void(parameter: Node<'_>, source: &[u8]) -> bool {
    if parameter.kind() != "parameter_declaration"
        || parameter.child_by_field_name("declarator").is_some()
    {
        return false;
    }
    let mut cursor = parameter.walk();
    parameter.child_by_field_name("type").is_some_and(|ty| {
        // The grammar represents `void`, `char`, `bool`, etc. by the same leaf kind.
        ty.kind() == "primitive_type" && source.get(ty.byte_range()) == Some(b"void")
    }) && !parameter.named_children(&mut cursor).any(|node| node.kind() == "type_qualifier")
}

fn attached_constraints(node: Node<'_>) -> usize {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "requires_clause" && !child.is_missing())
        .map(|clause| 1 + requires_expressions(clause))
        .sum()
}

fn requires_expressions(node: Node<'_>) -> usize {
    if matches!(node.kind(), "compound_statement" | "field_declaration_list") {
        return 0;
    }
    let mut cursor = node.walk();
    usize::from(node.kind() == "requires_expression" && !node.is_missing())
        + node.named_children(&mut cursor).map(requires_expressions).sum::<usize>()
}

fn spelled_type_cost(node: Node<'_>) -> usize {
    field_type_cost(node, "type") + inner_declarator(node).map_or(0, declarator_type_cost)
}

fn field_type_cost(node: Node<'_>, field: &str) -> usize {
    node.child_by_field_name(field).map_or(0, type_cost)
}

fn trailing_return_cost(node: Node<'_>) -> usize {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "trailing_return_type")
        .map(type_cost)
        .sum()
}

fn layer_type_cost(node: Node<'_>) -> usize {
    let layers = usize::from(is_type_layer(node));
    layers
        + match node.kind() {
            "function_declarator" | "abstract_function_declarator" => {
                field_type_cost(node, "parameters") + trailing_return_cost(node)
            }
            "array_declarator" | "abstract_array_declarator" => field_type_cost(node, "size"),
            "qualified_identifier" if is_member_pointer(node) => field_type_cost(node, "scope"),
            _ => 0,
        }
}

fn is_member_pointer(node: Node<'_>) -> bool {
    local_name(node).is_some_and(|name| name.kind() == "pointer_type_declarator")
}

fn declarator_type_cost(node: Node<'_>) -> usize {
    let inner = if node.kind() == "qualified_identifier" && is_member_pointer(node) {
        qualified_child(node)
    } else {
        inner_declarator(node)
    };
    layer_type_cost(node) + inner.map_or(0, declarator_type_cost)
}

fn type_cost(node: Node<'_>) -> usize {
    if node.is_missing() || node.is_extra() {
        return 0;
    }
    match node.kind() {
        "type_descriptor" | "parameter_declaration" | "optional_parameter_declaration"
        | "variadic_parameter_declaration" => return spelled_type_cost(node),
        "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
            return field_type_cost(node, "name");
        }
        "attribute_declaration" | "attribute_specifier" | "compound_statement"
        | "field_declaration_list" | "requires_clause" | "requires_expression"
        | "template_parameter_list" => return 0,
        _ if is_type_layer(node) => return declarator_type_cost(node),
        _ => {}
    }
    let arguments = node.kind() == "template_argument_list";
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| !child.is_missing() && !child.is_extra() && !child.is_error())
        .map(|child| usize::from(arguments) + type_cost(child))
        .sum()
}
