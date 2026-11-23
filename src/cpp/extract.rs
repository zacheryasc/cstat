use std::collections::HashMap;

use tree_sitter::Node;

use super::hunks::FileScore;
use super::metrics::body_metrics;
use super::signature::{declarator, name_text, signature, template_parameter_count, Declarator};
use super::{Access, Body, Kind, OwnerKind, Role, Span, Symbol};

#[derive(Clone, Copy, Default)]
struct Context<'a> {
    prefix: &'a str,
    key: &'a str,
    namespace: &'a str,
    local: bool,
    access: Option<Access>,
}

pub(super) fn symbols(
    root: Node<'_>,
    source: &[u8],
    scopes: &mut HashMap<String, OwnerKind>,
    hunks: Option<&mut FileScore<'_>>,
) -> Vec<Symbol> {
    let mut walker = Walker { source, scopes, hunks, symbols: Vec::new() };
    walker.visit(root, &mut Context::default());
    for symbol in &mut walker.symbols {
        if let Some(kind) = symbol.qualified_owner.as_ref().and_then(|name| walker.scopes.get(name).copied()) {
            // Even an ambiguous observation in this file takes precedence over other files.
            symbol.qualified_owner = None;
            symbol.resolve_owner(kind);
        }
    }
    walker.symbols.sort_unstable_by(|a, b| {
        (a.span.byte_start, a.span.byte_end, &a.name, a.kind)
            .cmp(&(b.span.byte_start, b.span.byte_end, &b.name, b.kind))
    });
    walker.symbols
}

struct Walker<'a, 'ranges> {
    source: &'a [u8],
    scopes: &'a mut HashMap<String, OwnerKind>,
    hunks: Option<&'a mut FileScore<'ranges>>,
    symbols: Vec<Symbol>,
}

impl Walker<'_, '_> {
    fn visit(&mut self, node: Node<'_>, ctx: &mut Context<'_>) {
        match node.kind() {
            "namespace_definition" => self.scope(node, *ctx, Kind::Namespace, None),
            "class_specifier" => self.scope(node, *ctx, Kind::Class, Some(Access::Private)),
            "struct_specifier" => self.scope(node, *ctx, Kind::Struct, Some(Access::Public)),
            "union_specifier" => self.scope(node, *ctx, Kind::Union, Some(Access::Public)),
            "enum_specifier" => self.scope(node, *ctx, Kind::Enum, None),
            "template_instantiation" if node.child_by_field_name("class_key").is_some() => {
                self.class_instantiation(node, *ctx);
            }
            "function_definition" | "declaration" | "field_declaration" | "template_instantiation" => {
                self.declaration(node, *ctx);
            }
            "lambda_expression" => self.lambda(node, *ctx),
            "alias_declaration" | "namespace_alias_definition" | "concept_definition" | "enumerator" => {
                let kind = match node.kind() {
                    "concept_definition" => Kind::Concept,
                    "enumerator" => Kind::Enumerator,
                    _ => Kind::Alias,
                };
                if let Some(name) = node.child_by_field_name("name") {
                    let templates = templates(node);
                    let span = Span::through(templates.last().copied().unwrap_or(node), node);
                    self.named(*ctx, name, kind, Role::Definition, span, &templates);
                }
                self.children(node, *ctx);
            }
            "type_definition" => {
                let templates = templates(node);
                let mut cursor = node.walk();
                for child in node.children_by_field_name("declarator", &mut cursor) {
                    if let Some(decl) = declarator(child) {
                        self.named(*ctx, decl.name, Kind::Alias, Role::Definition, Span::of(child), &templates);
                    }
                }
                self.children(node, *ctx);
            }
            "compound_statement" | "parameter_list" | "template_parameter_list" | "requires_expression" => {
                self.children(node, Context { local: true, access: None, ..*ctx });
            }
            "friend_declaration" => self.children(node, Context {
                prefix: ctx.namespace, key: ctx.namespace, access: None, ..*ctx
            }),
            "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef" => self.conditional(node, ctx),
            "access_specifier" if ctx.access.is_some() => {
                ctx.access = match &self.source[node.byte_range()] {
                    b"public" => Some(Access::Public),
                    b"protected" => Some(Access::Protected),
                    b"private" => Some(Access::Private),
                    _ => Some(Access::Unknown),
                };
            }
            _ => self.children(node, *ctx),
        }
    }

    fn children(&mut self, node: Node<'_>, mut ctx: Context<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, &mut ctx);
        }
    }

    fn conditional(&mut self, node: Node<'_>, ctx: &mut Context<'_>) {
        let condition = node.child_by_field_name("condition").or_else(|| node.child_by_field_name("name"));
        let alternative = node.child_by_field_name("alternative");
        let mut yes = *ctx;
        let mut no = *ctx;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if Some(child) == condition {
                continue;
            }
            if Some(child) == alternative {
                if child.kind() == "preproc_else" {
                    let mut branch = child.walk();
                    for item in child.named_children(&mut branch) {
                        self.visit(item, &mut no);
                    }
                } else {
                    self.visit(child, &mut no);
                }
            } else {
                self.visit(child, &mut yes);
            }
        }
        if ctx.access.is_some() {
            ctx.access = if yes.access == no.access { yes.access } else { Some(Access::Unknown) };
        }
    }

    fn scope(&mut self, node: Node<'_>, ctx: Context<'_>, kind: Kind, access: Option<Access>) {
        let body = node.child_by_field_name("body");
        let name = node.child_by_field_name("name").filter(|n| !n.is_missing());
        let mut ctx = ctx;
        let role = if body.is_some() { Role::Definition } else { Role::Declaration };
        if let Some(name) = name.filter(|_| body.is_some() || standalone_type(node)) {
            let templates = templates(node);
            let span = Span::through(templates.last().copied().unwrap_or(node), node);
            ctx = self.named(ctx, name, kind, role, span, &templates);
            if !ctx.local && kind != Kind::Enum {
                let key = key_text(name, self.source);
                let owner = if kind == Kind::Namespace { OwnerKind::Namespace } else { OwnerKind::Record };
                if name.kind() == "nested_namespace_specifier" {
                    for (end, _) in key.match_indices("::").filter(|&(end, _)| end != 0) {
                        OwnerKind::observe(self.scopes, qualify(ctx.key, &key[..end]), owner);
                    }
                }
                OwnerKind::observe(self.scopes, qualify(ctx.key, &key), owner);
            }
        }
        let Some(body) = body else { return };
        let (prefix, key) = if let Some(name) = name {
            (qualify(ctx.prefix, &name_text(name, self.source)), qualify(ctx.key, &key_text(name, self.source)))
        } else {
            (qualify(ctx.prefix, "<anonymous>"), qualify(ctx.key, "<anonymous>"))
        };
        let namespace = if kind == Kind::Namespace { key.as_str() } else { ctx.namespace };
        self.children(body, Context { prefix: &prefix, key: &key, namespace, access: access.or(ctx.access), ..ctx });
    }

    fn named<'a>(
        &mut self,
        ctx: Context<'a>,
        name: Node<'_>,
        kind: Kind,
        role: Role,
        span: Span,
        templates: &[Node<'_>],
    ) -> Context<'a> {
        if name.is_missing() {
            return ctx;
        }
        let owner = scope_key(name, self.source);
        let ctx = self.placement(ctx, owner.as_deref());
        let qualified_owner = owner.filter(|_| ctx.access.is_none()).map(|key| qualify(ctx.key, &key));
        let access = ctx.access.or(qualified_owner.as_ref().map(|_| Access::Unknown));
        self.symbols.push(Symbol {
            kind,
            role,
            name: qualify(ctx.prefix, &name_text(name, self.source)),
            name_position: terminal(name).start_position(),
            span,
            access,
            named_surface: !ctx.local,
            template_heads: templates.len(),
            template_parameters: template_parameter_count(templates),
            signature: None,
            body: None,
            qualified_owner,
        });
        ctx
    }

    fn class_instantiation(&mut self, node: Node<'_>, ctx: Context<'_>) {
        if let (Some(key), Some(name)) = (node.child_by_field_name("class_key"), node.child_by_field_name("name")) {
            let kind = match key.kind() {
                "class" => Kind::Class,
                "struct" => Kind::Struct,
                "union" => Kind::Union,
                _ => return,
            };
            let mut cursor = node.walk();
            let role = if node.children(&mut cursor).any(|child| child.kind() == "extern") {
                Role::Declaration
            } else {
                Role::Definition
            };
            self.named(ctx, name, kind, role, Span::of(node), &[]);
        }
        self.children(node, ctx);
    }

    fn declaration(&mut self, node: Node<'_>, ctx: Context<'_>) {
        let templates = templates(node);
        let implementation = if node.kind() == "function_definition" {
            node.child_by_field_name("body").or_else(|| named_child(node, "try_statement"))
                .filter(|body| !body.is_missing())
        } else {
            None
        };
        let mut cursor = node.walk();
        let mut fields = node.children_by_field_name("declarator", &mut cursor).peekable();
        let mut implicit_name = None;
        if let Some(first) = fields.next() {
            let single = fields.peek().is_none();
            let variable_kind = if node.kind() == "field_declaration" { Kind::Field } else { Kind::Variable };
            let mut default_role = None;
            for field in std::iter::once(first).chain(fields) {
                let Some(decl) = declarator(field).filter(|decl| !decl.name.is_missing()) else { continue };
                let span = if single { Span::through(templates.last().copied().unwrap_or(node), node) }
                    else { Span::of(field) };
                if decl.function.is_some() {
                    if single {
                        implicit_name = Some(self.symbols.len());
                    }
                    self.callable(node, ctx, decl, span, &templates, implementation.filter(|_| single));
                } else if !ctx.local || ctx.access.is_some() {
                    let role = if variable_kind == Kind::Variable
                        && (field.kind() == "init_declarator" || node.child_by_field_name("value").is_some())
                    {
                        Role::Definition
                    } else {
                        *default_role.get_or_insert_with(|| self.default_variable_role(node, first, variable_kind))
                    };
                    if decl.name.kind() == "structured_binding_declarator" {
                        let mut cursor = decl.name.walk();
                        for binding in decl.name.named_children(&mut cursor).filter(|child| child.kind() == "identifier") {
                            self.named(ctx, binding, variable_kind, role, Span::of(binding), &templates);
                        }
                    } else {
                        self.named(ctx, decl.name, variable_kind, role, span, &templates);
                    }
                }
            }
        }
        if let Some(index) = implicit_name {
            // The vector can grow while visiting a body. Keep the prefix independent of it.
            let prefix = self.symbols[index].name.clone();
            self.children(node, Context { prefix: &prefix, key: &prefix, local: true, access: None, ..ctx });
        } else if implementation.is_some() {
            self.children(node, Context { local: true, access: None, ..ctx });
        } else {
            // Copying the context here leaves the enclosing class's access in force for siblings.
            self.children(node, ctx);
        }
    }

    fn default_variable_role(&self, owner: Node<'_>, first: Node<'_>, kind: Kind) -> Role {
        let mut cursor = owner.walk();
        let specifiers = owner.named_children(&mut cursor).take_while(|child| child.end_byte() <= first.start_byte());
        if kind == Kind::Field {
            let mut is_static = false;
            let mut is_inline = false;
            for specifier in specifiers {
                match (specifier.kind(), &self.source[specifier.byte_range()]) {
                    ("storage_class_specifier", b"static") => is_static = true,
                    // Since C++17, a constexpr static data member is implicitly inline.
                    ("storage_class_specifier", b"inline") | ("type_qualifier", b"constexpr") => is_inline = true,
                    _ => {}
                }
            }
            if is_static && is_inline { Role::Definition } else { Role::Declaration }
        } else {
            // A linkage block does not imply extern storage for its contents.
            let directly_wrapped = owner.parent().is_some_and(|node| node.kind() == "linkage_specification");
            let mut storage = specifiers.filter(|child| child.kind() == "storage_class_specifier");
            if directly_wrapped || storage.any(|child| &self.source[child.byte_range()] == b"extern") {
                Role::Declaration
            } else {
                Role::Definition
            }
        }
    }

    fn callable(
        &mut self,
        owner: Node<'_>,
        ctx: Context<'_>,
        decl: Declarator<'_>,
        span: Span,
        templates: &[Node<'_>],
        implementation: Option<Node<'_>>,
    ) {
        let unqualified = terminal(decl.name);
        let enclosing = scope_key(decl.name, self.source);
        let ctx = self.placement(ctx, enclosing.as_deref());
        let external_qualifier = ctx.access.is_none() && enclosing.is_some();
        let qualified_owner = enclosing.as_ref().filter(|_| external_qualifier).map(|key| qualify(ctx.key, key));
        let name = name_text(decl.name, self.source);
        let direct_name = name_text(simple_name(unqualified), self.source);
        let enclosing_name = enclosing.as_deref().unwrap_or(ctx.key).rsplit("::").next().unwrap_or("");
        let kind = match unqualified.kind() {
            "destructor_name" => Kind::Destructor,
            "operator_cast" => Kind::ConversionOperator,
            "operator_name" => Kind::Operator,
            _ if direct_name == enclosing_name && (ctx.access.is_some() || external_qualifier)
                && owner.child_by_field_name("type").is_none() => Kind::Constructor,
            _ if ctx.access.is_some() || (external_qualifier && decl.function.is_some_and(|n| {
                named_child(n, "type_qualifier").is_some() || named_child(n, "ref_qualifier").is_some()
            })) => Kind::Method,
            _ => Kind::Function,
        };
        let access = ctx.access.or(qualified_owner.as_ref().map(|_| Access::Unknown));
        let (body, signature) = self.callable_metrics(owner, Some(decl), templates, implementation);
        self.symbols.push(Symbol {
            kind,
            role: if implementation.is_some() { Role::Definition } else { Role::Declaration },
            name: qualify(ctx.prefix, &name),
            name_position: unqualified.start_position(),
            span,
            access,
            named_surface: !ctx.local,
            template_heads: templates.len(),
            template_parameters: signature.as_ref().map_or(0, |s| s.template_parameters),
            signature,
            body,
            qualified_owner,
        });
    }

    fn callable_metrics<'tree>(
        &mut self,
        owner: Node<'tree>,
        decl: Option<Declarator<'tree>>,
        templates: &[Node<'tree>],
        implementation: Option<Node<'tree>>,
    ) -> (Option<Body>, Option<super::Signature>) {
        if let Some(hunks) = self.hunks.as_deref_mut() {
            if let Some(body) = implementation {
                hunks.body(owner, body);
                signature(owner, decl, templates, self.source, |node, part| hunks.signature(node, part));
            }
            (None, None)
        } else {
            let body = implementation.map(|body| Body {
                span: Span::through(named_child(owner, "field_initializer_list").unwrap_or(body), body),
                metrics: body_metrics(owner, body),
            });
            (body, Some(signature(owner, decl, templates, self.source, |_, _| 1)))
        }
    }

    // A relative qualifier can name a nested scope even when it repeats the
    // enclosing namespace's spelling. Only shorten when the alternate owner
    // is a known descendant and the nearer owner was not observed in this file.
    fn placement<'a>(&self, ctx: Context<'a>, owner: Option<&str>) -> Context<'a> {
        let Some(owner) = owner.filter(|name| !name.starts_with("::")) else { return ctx };
        if ctx.key.is_empty() || ctx.key != ctx.prefix || ctx.local || ctx.access.is_some() {
            return ctx;
        }
        let alternate = ctx.key.rmatch_indices("::")
            .map(|(split, _)| (&ctx.key[..split], &ctx.key[split + 2..]))
            .chain(std::iter::once(("", ctx.key)))
            .find(|&(prefix, suffix)| {
                owner.strip_prefix(suffix).is_some_and(|rest| rest.starts_with("::"))
                    && self.scopes.contains_key(&qualify(prefix, owner))
            });
        if let Some((prefix, _)) = alternate {
            if !self.scopes.contains_key(&qualify(ctx.key, owner)) {
                return Context { prefix, key: prefix, ..ctx };
            }
        }
        ctx
    }

    fn lambda(&mut self, node: Node<'_>, ctx: Context<'_>) {
        let Some(body) = node.child_by_field_name("body").filter(|body| !body.is_missing()) else {
            self.children(node, ctx);
            return;
        };
        let position = node.start_position();
        let name = qualify(ctx.prefix, &format!("<lambda@{}:{}>", position.row + 1, position.column + 1));
        let (body_metrics, signature) = self.callable_metrics(node, None, &[], Some(body));
        self.symbols.push(Symbol {
            kind: Kind::Lambda,
            role: Role::Definition,
            name: name.clone(),
            name_position: position,
            span: Span::of(node),
            access: None,
            named_surface: false,
            template_heads: usize::from(node.child_by_field_name("template_parameters").is_some()),
            template_parameters: signature.as_ref().map_or(0, |s| s.template_parameters),
            signature,
            body: body_metrics,
            qualified_owner: None,
        });
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child == body {
                self.children(child, Context { prefix: &name, key: &name, local: true, access: None, ..ctx });
            } else {
                self.visit(child, &mut Context { local: true, access: None, ..ctx });
            }
        }
    }
}

fn named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|child| child.kind() == kind);
    found
}

fn templates(mut node: Node<'_>) -> Vec<Node<'_>> {
    let mut heads = Vec::new();
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "template_declaration" => heads.push(parent),
            "friend_declaration" => {}
            _ => break,
        }
        node = parent;
    }
    heads
}

fn standalone_type(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| match parent.kind() {
        "declaration" | "field_declaration" => parent.child_by_field_name("declarator").is_none(),
        "translation_unit" | "declaration_list" | "field_declaration_list" | "compound_statement"
        | "template_declaration" | "preproc_if" | "preproc_ifdef" | "preproc_elif" | "preproc_elifdef"
        | "preproc_else" => true,
        _ => false,
    })
}

fn terminal(mut node: Node<'_>) -> Node<'_> {
    while node.kind() == "qualified_identifier" {
        let Some(name) = node.child_by_field_name("name") else { break };
        node = name;
    }
    node
}

fn simple_name(node: Node<'_>) -> Node<'_> {
    match node.kind() {
        "template_type" | "template_method" | "template_function" => node.child_by_field_name("name").unwrap_or(node),
        _ => node,
    }
}

fn key_text(node: Node<'_>, source: &[u8]) -> String {
    let mut key = scope_key(node, source).unwrap_or_default();
    if !key.is_empty() && key != "::" {
        key.push_str("::");
    }
    key.push_str(&name_text(simple_name(terminal(node)), source));
    key
}

fn scope_key(mut name: Node<'_>, source: &[u8]) -> Option<String> {
    if name.kind() != "qualified_identifier" {
        return None;
    }
    let mut path = String::new();
    while name.kind() == "qualified_identifier" {
        if let Some(scope) = name.child_by_field_name("scope") {
            if !path.is_empty() && path != "::" {
                path.push_str("::");
            }
            path.push_str(&name_text(simple_name(scope), source));
        } else if path.is_empty() {
            path.push_str("::");
        }
        let Some(next) = name.child_by_field_name("name") else { break };
        name = next;
    }
    Some(path)
}

fn qualify(prefix: &str, name: &str) -> String {
    if let Some(global) = name.strip_prefix("::") {
        return global.to_owned();
    }
    if prefix.is_empty() {
        return name.to_owned();
    }
    format!("{prefix}::{name}")
}
