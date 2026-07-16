//! Canonical semantic source printing over the shared indentation engine.

use crate::ast::{BindingTime, Decl, File, Name, PropertyMap, RefKind, Root, Value, Visibility};

pub fn print_file(file: &File) -> String {
    let mut printer = Printer::default();
    printer.file(file);
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
            self.text("import ");
            self.string(&import.source);
            if let Some(hash) = &import.sha256 {
                self.text(" sha256 ");
                self.string(hash);
            }
            if let Some(alias) = &import.alias {
                self.text(" as ");
                self.text(alias.as_str());
            }
            self.line(";");
        }
        if !file.imports.is_empty() {
            self.line("");
        }
        match &file.root {
            Root::Chart(decl) => self.chart(decl),
            Root::Define(decl) => self.definition(decl),
            Root::Data(declarations) => {
                for (index, decl) in declarations.iter().enumerate() {
                    if index > 0 {
                        self.line("");
                    }
                    self.decl(decl);
                }
            }
        }
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn chart(&mut self, decl: &Decl) {
        self.text("chart ");
        self.text(name_or(&decl.kind, "chart-kind"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn definition(&mut self, decl: &Decl) {
        self.text("define ");
        self.text(name_or(&decl.kind, "definition-kind"));
        self.text(" ");
        self.text(name_or(&decl.name, "definition"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn decl(&mut self, decl: &Decl) {
        if let Some(doc) = &decl.doc {
            for line in doc.lines() {
                self.text("-- | ");
                self.line(line);
            }
        }
        match decl.visibility {
            Visibility::Default => {}
            Visibility::Private => self.text("private "),
            Visibility::Public => self.text("public "),
        }
        match decl.keyword.as_str() {
            "catalog" | "schema" | "table" | "mark" | "transform" | "view" | "widget"
            | "resource" | "variable" | "derive" | "tool" => self.kind_bind_decl(decl),
            "param" | "store" | "selection" | "dimension" => self.bind_decl(decl),
            "group" | "overlay" => self.optional_bind_decl(decl),
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
        self.text(name_or(&decl.kind, "kind"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn bind_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.text(" as ");
        self.text(name_or(&decl.name, "binding"));
        self.body(&decl.props, &decl.children, &[]);
    }

    fn optional_bind_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn event(&mut self, decl: &Decl) {
        self.text("on ");
        self.text(name_or(&decl.kind, "event"));
        self.binder(&decl.name);
        self.body(&decl.props, &decl.children, &[]);
    }

    fn cell(&mut self, decl: &Decl) {
        self.text("cell ");
        self.text(name_or(&decl.kind, "coordinate"));
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
        self.text(name_or(&decl.kind, "kind"));
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
        }
        self.body(&decl.props, &decl.children, &[]);
    }

    fn plain_body_decl(&mut self, decl: &Decl) {
        self.text(decl.keyword.as_str());
        self.body(&decl.props, &decl.children, &[]);
    }

    fn field(&mut self, decl: &Decl) {
        self.text("field ");
        self.text(name_or(&decl.name, "field"));
        self.text(": ");
        if let Some(value) = decl.props.get("type") {
            self.value(value);
        } else {
            self.text("null");
        }
        if matches!(decl.props.get("nullable"), Some(Value::Bool(true))) {
            self.text(" nullable");
        }
        self.line(";");
    }

    fn slot(&mut self, decl: &Decl) {
        self.text("slot ");
        self.text(name_or(&decl.kind, "expr"));
        self.text(" as ");
        self.text(name_or(&decl.name, "slot"));
        if decl.props.is_empty() && decl.children.is_empty() {
            self.line(";");
        } else {
            self.body(&decl.props, &decl.children, &[]);
        }
    }

    fn channel(&mut self, decl: &Decl) {
        self.text("channel ");
        self.text(name_or(&decl.name, "channel"));
        if let Some(kind) = &decl.kind {
            self.text(": ");
            self.text(kind.as_str());
        }
        self.line(";");
    }

    fn output(&mut self, decl: &Decl) {
        self.text("output ");
        self.text(name_or(&decl.name, "output"));
        if let Some(value) = decl.props.get("value") {
            self.text(": ");
            self.value(value);
        }
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
        let kind = name_or(&decl.kind, "param");
        self.text(kind);
        if kind == "cursor" {
            self.text(" = ");
        } else {
            self.text(" ");
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
        self.line(" {");
        self.indent += 1;
        for (key, value) in props.iter() {
            if skip.contains(&key.as_str()) {
                continue;
            }
            self.text(key.as_str());
            self.text(": ");
            self.property_value(value);
        }
        for child in children {
            self.decl(child);
        }
        self.indent -= 1;
        self.line("}");
    }

    fn property_value(&mut self, value: &Value) {
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
            Value::Visual(value) => {
                self.text("value ");
                self.value(value);
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
            Value::Query(value) => self.text(&value.canonical_sql()),
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
            Value::Visual(value) => {
                self.text("value ");
                self.value(value);
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
        RefKind::Group => "group",
        RefKind::Selection => "selection",
        RefKind::Tool => "tool",
        RefKind::Widget => "widget",
        RefKind::Resource => "resource",
    }
}

fn name_or<'a>(name: &'a Option<Name>, fallback: &'a str) -> &'a str {
    name.as_ref().map_or(fallback, Name::as_str)
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
            "avenger 1; define tool brushing { slot number as radius; channel x: x; output domain: span(0, 1); export brush.domain as domain; tool behavior as inner {} }",
        );
        round_trip(
            "avenger 1; catalog memory as local { schema tables as vega { table inline as movies { values: []; } } }",
        );
    }

    #[test]
    fn print_sorts_properties_but_preserves_children() {
        let printed = round_trip(
            "avenger 1; chart cartesian { z: 2; mark symbol as first {} a: 1; mark line as second {} }",
        );
        assert!(printed.find("a: 1;").unwrap() < printed.find("z: 2;").unwrap());
        assert!(printed.find("first").unwrap() < printed.find("second").unwrap());
    }
}
