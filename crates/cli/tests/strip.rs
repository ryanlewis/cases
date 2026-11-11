//! The type strip build.rs runs on the inbox's TypeScript: types become
//! spaces, the JavaScript stays where it was, and syntax that does not
//! erase fails.

#[path = "../build/strip.rs"]
mod strip;

use strip::{not_javascript, strip};

/// `ts` stripped as a module.
fn js(ts: &str) -> String {
    strip(ts, false).unwrap_or_else(|problems| panic!("{ts}: {problems:?}"))
}

/// Why `ts` does not strip as a module.
fn refused(ts: &str) -> Vec<String> {
    match strip(ts, false) {
        Ok(js) => panic!("{ts} stripped to {js}"),
        Err(problems) => problems,
    }
}

#[test]
fn annotations_become_spaces() {
    assert_eq!(js("let a: number = 1;"), "let a         = 1;");
    assert_eq!(
        js("function f(a: string, b?: number): void {}"),
        "function f(a        , b         )       {}"
    );
    assert_eq!(js("const f = (a: T): U => a;"), "const f = (a   )    => a;");
}

#[test]
fn type_parameters_and_arguments_become_spaces() {
    assert_eq!(
        js("function f<T>(a: T): T { return a; }"),
        "function f   (a   )    { return a; }"
    );
    assert_eq!(js("g<string>(1);"), "g        (1);");
    assert_eq!(js("h.then(read<Case>);"), "h.then(read      );");
}

#[test]
fn casts_and_assertions_become_spaces() {
    assert_eq!(js("x = a as B;"), "x = a     ;");
    assert_eq!(js("x = (a as B).c;"), "x = (a     ).c;");
    assert_eq!(js("x = a satisfies B;"), "x = a            ;");
    assert_eq!(js("x = a!.b;"), "x = a .b;");
}

#[test]
fn type_only_statements_become_spaces_and_keep_their_lines() {
    let ts = "import type { A } from \"./a.js\";\n\
              export interface B {\n  a: A;\n}\n\
              type C = B;\n\
              export type { C };\n\
              let d = 1;\n";
    let stripped = js(ts);
    assert_eq!(stripped.len(), ts.len());
    assert_eq!(stripped.lines().count(), ts.lines().count());
    let lines: Vec<&str> = stripped.lines().map(str::trim).collect();
    assert_eq!(lines, [";", ";", "", "", ";", ";", "let d = 1;"]);
}

#[test]
fn a_statement_of_types_keeps_the_lines_around_it_apart() {
    // Blank, the interface would leave `b;(c)()` only for the `;` it starts
    // with.
    assert_eq!(
        js("a = b;\ninterface I {}\n(c)();"),
        "a = b;\n;             \n(c)();"
    );
}

/// The problem `ts` is refused with, which must be one.
fn refused_once(ts: &str) -> String {
    let problems = refused(ts);
    assert_eq!(problems.len(), 1, "{ts}: {problems:?}");
    problems.into_iter().next().unwrap_or_default()
}

#[test]
fn a_statement_without_its_semicolon_is_refused_by_line() {
    for (ts, line) in [
        // A type-only statement after a line without `;`, before a `(`.
        ("a = b\ninterface I {}\n(c)();", 1),
        ("let a = 1;\ntype T = number\n[a].map(f);", 2),
        ("import type { T } from \"./t.js\"\n`x`;", 1),
        ("export type T = number\n+a;", 1),
        // A tail of types at a line's end, before a line TypeScript starts
        // a new statement at.
        ("x = a as B\n(c);", 1),
        ("let a = 1;\nx = a satisfies B\n[c];", 2),
        ("x = a!\nc();", 1),
        ("type T = number\n-a;", 1),
        ("let a = 1\ninterface I {}\n/c/g.test(d);", 1),
        // Every other statement a line break could end, and a class field.
        ("let a = 1\nlet b = 2;", 1),
        ("function f() { return 1 }", 1),
        ("function f() { throw e }", 1),
        ("for (;;) { break }", 1),
        ("for (;;) { continue }", 1),
        ("do {} while (a)", 1),
        ("debugger", 1),
        ("import a from \"a\"", 1),
        ("export { a }", 1),
        ("export { a } from \"a\"", 1),
        ("export * from \"a\"", 1),
        ("export const a = 1", 1),
        ("export default a", 1),
        ("class A {\n  a = 1\n}", 2),
        ("class A {\n  declare a: number\n}", 2),
    ] {
        let problem = refused_once(ts);
        assert_eq!(
            problem,
            format!("line {line}: a statement that does not end with `;`"),
            "{ts}"
        );
    }
}

/// A line that goes on with the expression before it, as `a! - c` does in
/// TypeScript too, is one statement, and strips.
#[test]
fn a_line_that_goes_on_is_one_statement() {
    assert_eq!(js("x = a!\n-c;"), "x = a \n-c;");
    assert_eq!(js("x = a as B\n+ c;"), "x = a     \n+ c;");
}

#[test]
fn statements_that_end_in_a_block_need_no_semicolon() {
    js(
        "function f() {}\nclass A { m() {} }\nif (a) {}\nfor (const b of c) {}\ninterface I {}\nexport function g() {}\nexport default class {}\n",
    );
}

/// The inbox's own modules keep the rule, as build.rs strips them.
#[test]
fn the_inbox_s_modules_strip() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands/serve/web/static");
    let mut stripped = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if path.extension().is_none_or(|ext| ext != "ts") || name.ends_with(".d.ts") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        if let Err(problems) = strip(&source, name == "theme.ts") {
            panic!("{name}: {problems:?}");
        }
        stripped += 1;
    }
    assert_eq!(stripped, 24);
}

#[test]
fn declare_fields_become_spaces() {
    assert_eq!(
        js("class A extends B { declare s: number; }"),
        "class A extends B { ;                  }"
    );
}

#[test]
fn text_other_than_types_is_kept_byte_for_byte() {
    let ts = "// ünïcode ✓\nconst s = `a ${b} ✓`; /* c */ let t: \"✓\" = \"✓\";\n";
    assert_eq!(
        js(ts),
        "// ünïcode ✓\nconst s = `a ${b} ✓`; /* c */ let t        = \"✓\";\n"
    );
}

#[test]
fn a_plain_script_strips_as_a_script() {
    let ts = "(function () { var a: number = 1; })();";
    assert_eq!(
        strip(ts, true).unwrap(),
        "(function () { var a         = 1; })();"
    );
}

#[test]
fn syntax_that_does_not_erase_is_refused() {
    for ts in [
        "enum A { B }",
        "export enum A { B }",
        "namespace A { }",
        "declare const a: number;",
        "function f(a: number): void;\nfunction f(a) {}",
        "class A { constructor(private a: number) {} }",
        "class A { private a = 1; }",
        "class A { readonly a = 1; }",
        "abstract class A {}",
        "let a = <B>c;",
        "import { type A, b } from \"./a.js\";",
        "export { type A, b };",
        "import a = require(\"a\");",
        "export = a;",
    ] {
        refused(ts);
    }
}

#[test]
fn what_is_not_typescript_is_refused() {
    assert!(refused("let = ;")[0].starts_with("not TypeScript"));
}

#[test]
fn what_is_left_is_checked_as_javascript() {
    assert!(not_javascript("let a = 1;", false).is_empty());
    assert!(!not_javascript("let a: number = 1;", false).is_empty());
    // `class A implements B` strips nothing, and is not JavaScript.
    let problems = refused("interface B {}\nclass A implements B {}");
    assert!(problems[0].starts_with("not JavaScript"), "{problems:?}");
}
