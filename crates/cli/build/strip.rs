//! Types stripped from TypeScript by blanking them: each byte of a type
//! becomes a space, and line breaks stay. What is left is the JavaScript
//! the source holds, at the same byte offsets, so an error in the browser
//! points at the line of the `.ts` file.
//!
//! Only syntax that erases takes part: annotations, type parameters and
//! arguments, `as`, `satisfies`, `!`, `type` and `interface`, type-only
//! imports and exports, and `declare` class fields. Anything that would
//! need code written in its place, such as an `enum`, a namespace or a
//! parameter property, is refused (`erasableSyntaxOnly` in tsconfig.json
//! refuses the same). The result is parsed again as JavaScript, so nothing
//! of TypeScript's own is left in what the browser runs.
//!
//! Every statement that a line break could end, and every class field,
//! must end with its own `;`. Where the source leaves one out, blanking a
//! type could join two statements into one that still parses: `a = b`, an
//! `interface`, then `(c)()` would read as `a = b(c)()`, and so would a
//! line ending `as T` before a line starting `(`. With every statement
//! ended by a `;`, what is blanked cannot move where one ends. A statement
//! that is all type, such as an `interface` or an `import type`, also
//! starts with a `;` in place of its first byte.
//!
//! build.rs strips the inbox's modules with it, and `tests/strip.rs` tests
//! it.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Declaration, ExportDeclaration, ExportDefaultDeclarationKind, ExportFromDeclaration,
    ExportNamedDeclaration, ExportSpecifier, FormalParameter, ImportDeclaration,
    ImportDeclarationSpecifier, PropertyDefinition, Statement, TSAsExpression, TSNonNullExpression,
    TSSatisfiesExpression, TSTypeAnnotation, TSTypeAssertion, TSTypeParameterDeclaration,
    TSTypeParameterInstantiation,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

/// The JavaScript of `source`, TypeScript, with its types blanked: a plain
/// script when `script`, else a module. Each problem is a line of text that
/// names the line of `source` it is on.
pub(crate) fn strip(source: &str, script: bool) -> Result<String, Vec<String>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(
        &allocator,
        source,
        SourceType::ts().with_script(script).with_module(!script),
    )
    .parse();
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .iter()
            .map(|problem| format!("not TypeScript: {problem}"))
            .collect());
    }
    let mut types = Types {
        source,
        blanks: Vec::new(),
        statements: Vec::new(),
        problems: Vec::new(),
    };
    types.visit_program(&parsed.program);
    if !types.problems.is_empty() {
        return Err(types.problems);
    }
    let mut js = source.as_bytes().to_vec();
    for span in &types.blanks {
        for byte in &mut js[span.start as usize..span.end as usize] {
            if !matches!(byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
    }
    for span in &types.statements {
        js[span.start as usize] = b';';
    }
    // Every byte blanked became ASCII, so the text is whole.
    let js = String::from_utf8(js).map_err(|error| vec![error.to_string()])?;
    let problems = not_javascript(&js, script);
    if problems.is_empty() {
        Ok(js)
    } else {
        Err(problems)
    }
}

/// What keeps `js` from parsing as JavaScript: a plain script when
/// `script`, else a module. Empty when it parses.
pub(crate) fn not_javascript(js: &str, script: bool) -> Vec<String> {
    let allocator = Allocator::default();
    let kind = if script {
        SourceType::script()
    } else {
        SourceType::mjs()
    };
    let parsed = Parser::new(&allocator, js, kind).parse();
    parsed
        .diagnostics
        .iter()
        .map(|problem| format!("not JavaScript once stripped: {problem}"))
        .collect()
}

/// The spans of a program's types, to blank, and what it holds that does
/// not erase.
struct Types<'s> {
    source: &'s str,
    blanks: Vec<Span>,
    /// The spans of whole statements of types, each also in `blanks`, which
    /// start with a `;` once blanked.
    statements: Vec<Span>,
    problems: Vec<String>,
}

impl Types<'_> {
    fn refuse(&mut self, span: Span, what: &str) {
        let line = self.line(span.start);
        self.problems
            .push(format!("line {line}: {what} does not erase"));
    }

    /// The line of `source`, from 1, that the byte at `at` is on.
    fn line(&self, at: u32) -> usize {
        self.source.as_bytes()[..at as usize]
            .split(|byte| *byte == b'\n')
            .count()
    }

    /// Refuses `span`, a statement or class field that a line break could
    /// end, unless it ends with its own `;`.
    fn ended(&mut self, span: Span) {
        if span.end == 0 || self.source.as_bytes()[span.end as usize - 1] != b';' {
            let line = self.line(span.end.saturating_sub(1));
            self.problems.push(format!(
                "line {line}: a statement that does not end with `;`"
            ));
        }
    }

    /// A statement, or class member, that is all type: blanked, to a `;`.
    fn statement(&mut self, span: Span) {
        self.blanks.push(span);
        self.statements.push(span);
    }

    /// `export { ... }` at `span`: blanked when it is `export type`.
    fn exports(&mut self, span: Span, types: bool, specifiers: &[ExportSpecifier<'_>]) {
        if types {
            self.statement(span);
        } else if specifiers.iter().any(|s| s.export_kind.is_type()) {
            self.refuse(span, "a `type` inside braces (write `export type`)");
        }
    }

    /// The span from the end of `inner`, an expression, to the end of
    /// `outer`, the expression with the TypeScript after it.
    fn tail(&mut self, inner: Span, outer: Span) {
        self.blanks.push(Span::new(inner.end, outer.end));
    }
}

impl<'a> Visit<'a> for Types<'_> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        let needs_semicolon = match it {
            Statement::ExpressionStatement(_)
            | Statement::VariableDeclaration(_)
            | Statement::ReturnStatement(_)
            | Statement::ThrowStatement(_)
            | Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
            | Statement::DoWhileStatement(_)
            | Statement::DebuggerStatement(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_) => true,
            Statement::ExportDeclaration(e) => matches!(
                e.declaration,
                Declaration::VariableDeclaration(_) | Declaration::TSTypeAliasDeclaration(_)
            ),
            Statement::ExportDefaultDeclaration(e) => !matches!(
                e.declaration,
                ExportDefaultDeclarationKind::FunctionDeclaration(_)
                    | ExportDefaultDeclarationKind::ClassDeclaration(_)
                    | ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
            ),
            _ => false,
        };
        if needs_semicolon {
            self.ended(it.span());
        }
        match it {
            Statement::TSTypeAliasDeclaration(_) | Statement::TSInterfaceDeclaration(_) => {
                self.statement(it.span());
            }
            Statement::TSImportEqualsDeclaration(_)
            | Statement::TSExportAssignment(_)
            | Statement::TSNamespaceExportDeclaration(_) => {
                self.refuse(it.span(), "an import =, export = or export as namespace");
            }
            _ => walk::walk_statement(self, it),
        }
    }

    fn visit_declaration(&mut self, it: &Declaration<'a>) {
        match it {
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {
                self.statement(it.span());
            }
            Declaration::TSEnumDeclaration(_) => self.refuse(it.span(), "an enum"),
            Declaration::TSNamespaceDeclaration(_) | Declaration::TSGlobalDeclaration(_) => {
                self.refuse(it.span(), "a namespace");
            }
            Declaration::VariableDeclaration(v) if v.declare => {
                self.refuse(it.span(), "a declare");
            }
            Declaration::FunctionDeclaration(f) if f.declare || f.body.is_none() => {
                self.refuse(it.span(), "a declare or an overload");
            }
            Declaration::ClassDeclaration(c) if c.declare || c.r#abstract => {
                self.refuse(it.span(), "a declare or abstract class");
            }
            _ => walk::walk_declaration(self, it),
        }
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if it.import_kind.is_type() {
            self.statement(it.span);
            return;
        }
        for specifier in it.specifiers.iter().flatten() {
            if let ImportDeclarationSpecifier::ImportSpecifier(s) = specifier
                && s.import_kind.is_type()
            {
                self.refuse(s.span, "a `type` inside braces (write `import type`)");
            }
        }
    }

    fn visit_export_declaration(&mut self, it: &ExportDeclaration<'a>) {
        if matches!(
            it.declaration,
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_)
        ) {
            self.statement(it.span);
            return;
        }
        walk::walk_export_declaration(self, it);
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        self.exports(it.span, it.export_kind.is_type(), &it.specifiers);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        self.exports(it.span, it.export_kind.is_type(), &it.specifiers);
    }

    fn visit_ts_type_annotation(&mut self, it: &TSTypeAnnotation<'a>) {
        self.blanks.push(it.span);
    }

    fn visit_ts_type_parameter_declaration(&mut self, it: &TSTypeParameterDeclaration<'a>) {
        self.blanks.push(it.span);
    }

    fn visit_ts_type_parameter_instantiation(&mut self, it: &TSTypeParameterInstantiation<'a>) {
        self.blanks.push(it.span);
    }

    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        self.tail(it.expression.span(), it.span);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_satisfies_expression(&mut self, it: &TSSatisfiesExpression<'a>) {
        self.tail(it.expression.span(), it.span);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_non_null_expression(&mut self, it: &TSNonNullExpression<'a>) {
        self.tail(it.expression.span(), it.span);
        self.visit_expression(&it.expression);
    }

    fn visit_ts_type_assertion(&mut self, it: &TSTypeAssertion<'a>) {
        self.refuse(it.span, "a `<T>x` assertion (write `x as T`)");
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.accessibility.is_some() || it.readonly || it.r#override {
            self.refuse(it.span, "a parameter property");
        }
        if it.optional {
            // The `?` after the name, which the annotation follows.
            let end = it.pattern.span().end;
            if self.source.as_bytes().get(end as usize) == Some(&b'?') {
                self.blanks.push(Span::new(end, end + 1));
            } else {
                self.refuse(it.span, "an optional parameter not written `name?`");
            }
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        self.ended(it.span);
        if it.declare {
            self.statement(it.span);
            return;
        }
        if it.accessibility.is_some()
            || it.readonly
            || it.optional
            || it.definite
            || it.r#override
            || it.r#type.is_abstract()
        {
            self.refuse(it.span, "a class field's modifier");
        }
        walk::walk_property_definition(self, it);
    }
}
