//! Canonical semantic source printing over the shared indentation engine.

use crate::ast::{
    BindingTime, Decl, File, ImportClause, ModuleItem, Name, PropertyMap, QualifiedName, RefKind,
    Value, Visibility,
};

pub fn print_file(file: &File) -> String {
    let mut printer = Printer::default();
    printer.file(file);
    printer.output
}

/// Render one semantic value using the same canonical spelling as file
/// printing. Resolution uses this to reinterpret call-shaped values in slots
/// whose native schema declares SQL-expression semantics.
pub(crate) fn print_value(value: &Value) -> String {
    let mut printer = Printer::default();
    printer.value(value);
    printer.output
}

#[derive(Default)]
struct Printer {
    output: String,
    indent: usize,
    line_start: bool,
}

impl Printer {
    fn file(&mut self, file: &File) {
        self.text("avenger ");
        self.text(&file.version.to_string());
        self.line(";");
        for import in &file.imports {
            match &import.clause {
                ImportClause::Named(specifiers) => {
                    let inline = specifiers
                        .iter()
                        .map(|specifier| {
                            if specifier.imported == specifier.local {
                                specifier.imported.to_string()
                            } else {
                                format!("{} as {}", specifier.imported, specifier.local)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let source_width = import.source.chars().count() + 16;
                    if inline.chars().count() + source_width <= 88 {
                        self.text("import { ");
                        self.text(&inline);
                        self.text(" } from ");
                    } else {
                        self.line("import {");
                        self.indent += 1;
                        for specifier in specifiers {
                            self.text(specifier.imported.as_str());
                            if specifier.imported != specifier.local {
                                self.text(" as ");
                                self.text(specifier.local.as_str());
                            }
                            self.line(",");
                        }
                        self.indent -= 1;
                        self.text("} from ");
                    }
                }
                ImportClause::Namespace(local) => {
                    self.text("import * as ");
                    self.text(local.as_str());
                    self.text(" from ");
                }
            }
            self.string(&import.source);
            if let Some(hash) = &import.sha256 {
                self.text(" sha256 ");
                self.string(hash);
            }
            self.line(";");
        }
        if !file.imports.is_empty() {
            self.line("");
        }
        for (index, item) in file.items.iter().enumerate() {
            if index > 0 {
                self.line("");
            }
            self.module_item(item);
        }
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn module_item(&mut self, item: &ModuleItem) {
        self.doc(&item.declaration);
        if item.exported {
            self.text("export ");
        }
        match item.declaration.keyword.as_str() {
            "chart" => self.chart(&item.declaration),
            "define" => self.definition(&item.declaration),
            _ => self.decl_without_doc(&item.declaration),
        }
    }

    fn chart(&mut self, decl: &Decl) {
        self.text("chart ");
        self.text(kind_or(&decl.kind, "chart-kind"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn definition(&mut self, decl: &Decl) {
        self.text("define ");
        self.text(kind_or(&decl.kind, "definition-kind"));
        self.text(" ");
        self.text(name_or(&decl.name, "definition"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn decl(&mut self, decl: &Decl) {
        self.doc(decl);
        self.decl_without_doc(decl);
    }

    fn doc(&mut self, decl: &Decl) {
        if let Some(doc) = &decl.doc {
            for line in doc.lines() {
                self.text("-- | ");
                self.line(line);
            }
        }
    }

    fn decl_without_doc(&mut self, decl: &Decl) {
        match decl.visibility {
            Visibility::Default => {}
            Visibility::Private => self.text("private "),
            Visibility::Public => self.text("public "),
        }
        match decl.keyword.as_str() {
            "catalog" | "schema" | "table" | "mark" | "transform" | "view" | "widget"
            | "resource" | "derive" | "tool" => self.kind_bind_decl(decl),
            "variable" => self.variable(decl),
            "param" => self.param(decl),
            "store" | "selection" => self.state_binding(decl),
            "dimension" => self.predicate_entry(decl),
            "on" => self.event(decl),
            "cell" => self.cell(decl),
            "plot" => self.kind_body_decl(decl),
            "part" | "layer" => self.named_body_decl(decl),
            "level" => self.level(decl),
            "adjust" => self.adjust(decl),
            "row" | "when" | "key" | "fields" | "scale_edit" | "scale_hint" => {
                self.plain_body_decl(decl)
            }
            "field" => self.field(decl),
            "slot" => self.slot(decl),
            "channel" => self.channel(decl),
            "output" => self.output(decl),
            "export" => self.export(decl),
            "match" => self.match_block(decl),
            "splice" => self.splice(decl),
            "set" => self.action(decl),
            "theme" => self.theme(decl),
            _ => self.generic_decl(decl),
        }
    }

    fn kind_bind_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.text(" ");
        self.text(kind_or(&decl.kind, "kind"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn variable(&mut self, decl: &Decl) {
        self.text("variable ");
        self.text(kind_or(&decl.kind, "role"));
        self.text(" ");
        self.text(name_or(&decl.name, "variable"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn param(&mut self, decl: &Decl) {
        self.text("param ");
        if let Some(initializer) = decl.props.get("value") {
            self.value(initializer);
        } else {
            self.text("NULL");
        }
        self.text(" as ");
        self.text(name_or(&decl.name, "binding"));
        if decl.props.len() == 1 && decl.children.is_empty() {
            self.line(";");
        } else {
            self.body(&decl.props, &decl.children, &["value"]);
        }
    }

    fn state_binding(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.text(" as ");
        self.text(name_or(&decl.name, "binding"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn event(&mut self, decl: &Decl) {
        self.text("on ");
        self.text(kind_or(&decl.kind, "event"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn cell(&mut self, decl: &Decl) {
        self.text("cell ");
        self.text(kind_or(&decl.kind, "coordinate"));
        self.binder(&decl.name);
        if let Some(Value::Block { head: None, body }) = decl.props.get("at") {
            self.text(" at");
            self.body(&body.props, &body.children, &[]);
        }
        self.body(&decl.props, &decl.children, &["at"]);
    }

    fn kind_body_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.text(" ");
        self.text(kind_or(&decl.kind, "kind"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn named_body_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.text(" ");
        self.text(name_or(&decl.name, "name"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn level(&mut self, decl: &Decl) {
        self.text("level ");
        match decl.props.get("index") {
            Some(Value::Num(value)) => self.text(value.as_str()),
            _ => self.text("0"),
        }
        self.body(&decl.props, &decl.children, &["index"]);
    }

    fn adjust(&mut self, decl: &Decl) {
        self.text("adjust");
        if let Some(kind) = &decl.kind {
            self.text(" ");
            self.text(kind.as_str());
            self.binder(&decl.name);
        } else {
            self.text(" expr");
        }
        self.body(&decl.props, &decl.children, &[]);
    }

    fn plain_body_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        if matches!(decl.keyword.as_str(), "equality" | "interval") {
            self.body_with_parent(
                &decl.props,
                &decl.children,
                &[],
                Some(decl.keyword.as_str()),
            );
        } else {
            self.body(&decl.props, &decl.children, &[]);
        }
    }

    fn predicate_entry(&mut self, decl: &Decl) {
        self.text(name_or(&decl.name, "dimension"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn field(&mut self, decl: &Decl) {
        self.text("field ");
        if let Some(value) = decl.props.get("type") {
            self.value(value);
        } else {
            self.text("null");
        }
        self.text(" ");
        self.text(name_or(&decl.name, "field"));
        if matches!(decl.props.get("nullable"), Some(Value::Bool(true))) {
            self.text(" nullable");
        }
        self.line(";");
    }

    fn slot(&mut self, decl: &Decl) {
        self.text("slot ");
        self.text(kind_or(&decl.kind, "expr"));
        self.text(" ");
        self.text(name_or(&decl.name, "slot"));
        if decl.props.is_empty() && decl.children.is_empty() {
            self.line(";");
        } else {
            self.body(&decl.props, &decl.children, &[]);
        }
    }

    fn channel(&mut self, decl: &Decl) {
        self.text("slot channel ");
        self.text(name_or(&decl.name, "channel"));
        if let Some(kind) = &decl.kind {
            self.line(" {");
            self.indent += 1;
            self.text("default: ");
            self.text(kind.as_str());
            self.line(";");
            self.indent -= 1;
            self.line("}");
        } else {
            self.line(";");
        }
    }

    fn output(&mut self, decl: &Decl) {
        self.text("output ");
        if let Some(value) = decl.props.get("value") {
            self.value(value);
            self.text(" as ");
        }
        self.text(name_or(&decl.name, "output"));
        self.line(";");
    }

    fn export(&mut self, decl: &Decl) {
        self.text("export ");
        self.path_property(&decl.props, "source");
        if let Some(alias) = &decl.name {
            self.text(" as ");
            self.text(alias.as_str());
        }
        self.line(";");
    }

    fn match_block(&mut self, decl: &Decl) {
        self.text("match ");
        self.text(name_or(&decl.name, "slot"));
        self.line(" {");
        self.indent += 1;
        for arm in &decl.children {
            if let Some(doc) = &arm.doc {
                for line in doc.lines() {
                    self.text("-- | ");
                    self.line(line);
                }
            }
            self.text(name_or(&arm.name, "arm"));
            self.body(&arm.props, &arm.children, &[]);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn splice(&mut self, decl: &Decl) {
        self.text(name_or(&decl.name, "slot"));
        self.line(";");
    }

    fn action(&mut self, decl: &Decl) {
        self.text("set ");
        if decl
            .kind
            .as_ref()
            .is_some_and(|kind| kind.as_str() == "cursor")
        {
            self.text("cursor");
            self.text(" = ");
        } else {
            self.path_property(&decl.props, "target");
            if let Some(Value::Atom(at)) = decl.props.get("at") {
                self.text(" at ");
                self.text(at.as_str());
            }
            if matches!(decl.props.get("replacing_scopes"), Some(Value::Bool(true))) {
                self.text(" replacing scopes");
            }
            self.text(" = ");
        }
        if let Some(value) = decl.props.get("value") {
            self.value(value);
            if !matches!(value, Value::Block { .. }) {
                self.line(";");
            }
        } else {
            self.line("null;");
        }
    }

    fn theme(&mut self, decl: &Decl) {
        self.text("theme css ");
        if let Some(Value::Str(value)) = decl.props.get("from") {
            self.text("from ");
            self.string(value);
            if let Some(Value::Str(hash)) = decl.props.get("sha256") {
                self.text(" sha256 ");
                self.string(hash);
            }
        } else if let Some(Value::Str(value)) = decl.props.get("css") {
            self.text(": ");
            self.string(value);
        } else {
            self.text(": ''");
        }
        self.line(";");
    }

    fn generic_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        if let Some(kind) = &decl.kind {
            self.text(" ");
            self.text(kind.as_str());
        }
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn body(&mut self, props: &PropertyMap, children: &[Decl], skip: &[&str]) {
        self.body_with_parent(props, children, skip, None);
    }

    fn body_with_parent(
        &mut self,
        props: &PropertyMap,
        children: &[Decl],
        skip: &[&str],
        parent: Option<&str>,
    ) {
        if self.output.ends_with(' ') {
            self.line("{");
        } else {
            self.line(" {");
        }
        self.indent += 1;
        for (key, value) in props.iter() {
            if skip.contains(&key.as_str()) {
                continue;
            }
            self.text(key.as_str());
            self.text(": ");
            self.property_value(key.as_str(), value);
        }
        for child in children {
            if matches!(parent, Some("equality" | "interval"))
                && child.keyword.as_str() == "dimension"
            {
                self.predicate_entry(child);
            } else {
                self.decl(child);
            }
        }
        self.indent -= 1;
        self.line("}");
    }

    fn property_value(&mut self, property: &str, value: &Value) {
        if property == "target"
            && let Value::Array(values) = value
            && values.iter().all(|value| {
                matches!(
                    value,
                    Value::Ref {
                        kind: RefKind::Mark,
                        ..
                    }
                )
            })
        {
            self.text("marks [");
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    self.text(", ");
                }
                if let Value::Ref { path, .. } = value {
                    self.path(path);
                }
            }
            self.line("];");
            return;
        }
        if property == "scope" {
            if let Some(path) = call_path(value, "subplot") {
                self.text("subplot ");
                self.path_refs(&path);
                self.line(";");
                return;
            }
            if let Value::Array(values) = value
                && values
                    .iter()
                    .all(|value| call_path(value, "subplot").is_some())
            {
                self.text("subplots [");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.text(", ");
                    }
                    self.path_refs(&call_path(value, "subplot").expect("checked call path"));
                }
                self.line("];");
                return;
            }
        }
        if property == "surface"
            && let Some(path) = call_path(value, "legend")
            && path.len() == 1
        {
            self.text("legend ");
            self.text(path[0].as_str());
            self.line(";");
            return;
        }
        match value {
            Value::Block { head, body } => {
                if let Some(head) = head {
                    self.value(head);
                }
                self.body(&body.props, &body.children, &[]);
            }
            Value::Ref { kind, path } => {
                self.text(ref_name(*kind));
                self.text(" ");
                self.path(path);
                self.line(";");
            }
            Value::Query(query) => {
                self.text(&query.canonical_sql());
                self.line(";");
            }
            Value::Relation(path) => {
                self.path(path);
                self.line(";");
            }
            Value::Projection(projection) => {
                let items = projection.canonical_items();
                let current_width = self
                    .output
                    .rsplit_once('\n')
                    .map_or(self.output.len(), |(_, line)| line.len());
                if items.len() == 1 && current_width + items[0].len() < 88 {
                    self.text(&items[0]);
                    self.line(";");
                } else {
                    if self.output.ends_with(' ') {
                        self.output.pop();
                    }
                    self.line("");
                    self.indent += 1;
                    for (index, item) in items.iter().enumerate() {
                        self.text(item);
                        self.line(if index + 1 == items.len() { ";" } else { "," });
                    }
                    self.indent -= 1;
                }
            }
            Value::Channel { mode, expression } => {
                self.text(mode.as_str());
                self.text(" ");
                self.value(expression);
                self.line(";");
            }
            Value::Dim(path) => {
                self.text("dim ");
                self.path(path);
                self.line(";");
            }
            Value::Pattern(value) => {
                self.text("pattern ");
                self.value(value);
                if !matches!(value.as_ref(), Value::Block { .. }) {
                    self.line(";");
                }
            }
            Value::Env(value) => {
                self.text("env ");
                self.string(value);
                self.line(";");
            }
            Value::None => self.line("none;"),
            _ => {
                self.value(value);
                self.line(";");
            }
        }
    }

    fn value(&mut self, value: &Value) {
        match value {
            Value::Str(value) => self.string(value),
            Value::Num(value) => self.text(value.as_str()),
            Value::Bool(value) => self.text(if *value { "true" } else { "false" }),
            Value::Null => self.text("null"),
            Value::Column(value) => {
                self.text("\"");
                self.text(&value.replace('"', "\"\""));
                self.text("\"");
            }
            Value::Atom(value) => self.text(value.as_str()),
            Value::Expr(value) => self.text(&value.canonical_sql()),
            Value::Projection(value) => self.text(&value.canonical_sql()),
            Value::Query(value) => self.text(&value.canonical_sql()),
            Value::Relation(path) => self.path(path),
            Value::Binding { path, time, .. } => {
                self.text("$");
                self.path(path);
                match time {
                    BindingTime::Current => {}
                    BindingTime::Start => self.text("@start"),
                    BindingTime::Previous => self.text("@previous"),
                }
            }
            Value::Ref { kind, path } => {
                self.text(ref_name(*kind));
                self.text(" ");
                self.path(path);
            }
            Value::Channel { mode, expression } => {
                self.text(mode.as_str());
                self.text(" ");
                self.value(expression);
            }
            Value::Dim(path) => {
                self.text("dim ");
                self.path(path);
            }
            Value::Pattern(value) => {
                self.text("pattern ");
                self.value(value);
            }
            Value::Env(value) => {
                self.text("env ");
                self.string(value);
            }
            Value::None => self.text("none"),
            Value::Array(values) => {
                self.text("[");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.text(", ");
                    }
                    self.value(value);
                }
                self.text("]");
            }
            Value::Block { head, body } => {
                if let Some(head) = head {
                    self.value(head);
                }
                self.body(&body.props, &body.children, &[]);
            }
            Value::Call { function, args } => {
                self.text(function.as_str());
                self.text("(");
                for (index, value) in args.iter().enumerate() {
                    if index > 0 {
                        self.text(", ");
                    }
                    self.value(value);
                }
                self.text(")");
            }
        }
    }

    fn path_property(&mut self, props: &PropertyMap, key: &str) {
        if let Some(Value::Array(values)) = props.get(key) {
            let names = values
                .iter()
                .filter_map(|value| match value {
                    Value::Atom(name) => Some(name),
                    _ => None,
                })
                .collect::<Vec<_>>();
            self.path_refs(&names);
        } else {
            self.text("missing");
        }
    }

    fn path(&mut self, path: &[Name]) {
        self.path_refs(&path.iter().collect::<Vec<_>>());
    }

    fn path_refs(&mut self, path: &[&Name]) {
        for (index, name) in path.iter().enumerate() {
            if index > 0 {
                self.text(".");
            }
            self.text(name.as_str());
        }
    }

    fn binder(&mut self, binder: &Option<Name>) {
        if let Some(binder) = binder {
            self.text(" as ");
            self.text(binder.as_str());
        }
    }

    fn string(&mut self, value: &str) {
        self.text("'");
        self.text(&value.replace('\'', "''"));
        self.text("'");
    }

    fn text(&mut self, value: &str) {
        if self.line_start {
            for _ in 0..self.indent {
                self.output.push_str("  ");
            }
            self.line_start = false;
        }
        self.output.push_str(value);
    }

    fn line(&mut self, value: &str) {
        self.text(value);
        self.output.push('\n');
        self.line_start = true;
    }
}

fn ref_name(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Mark => "mark",
        RefKind::Selection => "selection",
        RefKind::Tool => "tool",
        RefKind::Widget => "widget",
        RefKind::Resource => "resource",
    }
}

fn name_or<'a>(name: &'a Option<Name>, fallback: &'a str) -> &'a str {
    name.as_ref().map_or(fallback, Name::as_str)
}

fn kind_or<'a>(kind: &'a Option<QualifiedName>, fallback: &'a str) -> &'a str {
    kind.as_ref().map_or(fallback, QualifiedName::as_str)
}

fn call_path<'a>(value: &'a Value, function: &str) -> Option<Vec<&'a Name>> {
    let Value::Call {
        function: actual,
        args,
    } = value
    else {
        return None;
    };
    if actual.as_str() != function {
        return None;
    }
    args.iter()
        .map(|value| match value {
            Value::Atom(name) => Some(name),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{SourceFile, SourceId, SourceOrigin, syntax::parse_file};

    use super::print_file;

    fn round_trip(source: &str) -> String {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("print.avenger".into()),
            source,
        );
        let parsed = parse_file(&source).unwrap();
        let printed = print_file(&parsed.ast);
        let reparsed = parse_file(&SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("print-2.avenger".into()),
            printed.clone(),
        ))
        .unwrap();
        assert_eq!(parsed.ast, reparsed.ast);
        assert_eq!(printed, print_file(&reparsed.ast));
        printed
    }

    #[test]
    fn print_round_trip_chart_definition_and_data() {
        round_trip("avenger 1; chart cartesian { z: 2; a: 1; mark symbol as dots { x: \"x\"; } }");
        round_trip(
            "avenger 1; define tool brushing { slot number radius; slot channel x { default: x; } output span(0, 1) as domain; export brush.domain as domain; tool behavior as inner {} }",
        );
        round_trip(
            "avenger 1; catalog memory as local { schema tables as vega { table inline as movies { values: []; } } }",
        );
    }

    #[test]
    fn print_round_trips_mixed_modules_and_canonical_import_forms() {
        let printed = round_trip(
            "avenger 1;\
             import { first, second as local, third, fourth, fifth, sixth, seventh, eighth } from './a-very-long-library-module-name.avenger';\
             import * as acme from 'native:acme' sha256 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';\
             export define mark badge { mark symbol {} }\
             table inline as rows { values: []; }\
             export chart acme.cartesian as dashboard {}",
        );
        assert!(printed.contains(
            "import {\n  first,\n  second as local,\n  third,\n  fourth,\n  fifth,\n  sixth,\n  seventh,\n  eighth,\n} from './a-very-long-library-module-name.avenger';"
        ));
        assert!(printed.contains("import * as acme from 'native:acme' sha256 'aaaaaaaa"));
        assert!(printed.contains("export define mark badge"));
        assert!(printed.contains("export chart acme.cartesian as dashboard"));
        assert!(printed.find("define mark").unwrap() < printed.find("table inline").unwrap());
        assert!(printed.find("table inline").unwrap() < printed.find("chart acme").unwrap());
    }

    #[test]
    fn print_sorts_properties_but_preserves_children() {
        let printed = round_trip(
            "avenger 1; chart cartesian { z: 2; mark symbol as first {} a: 1; mark line as second {} }",
        );
        assert!(printed.find("a: 1;").unwrap() < printed.find("z: 2;").unwrap());
        assert!(printed.find("first").unwrap() < printed.find("second").unwrap());
    }

    #[test]
    fn print_round_trips_dedicated_event_value_shapes() {
        round_trip(
            "avenger 1; chart cartesian { on pointermove { target: marks [layers.a, b,]; scope: subplots [cells.left, cells.right]; surface: legend color; } }",
        );
    }
}
