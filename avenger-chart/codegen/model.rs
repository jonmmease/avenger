//! The parameter model of the messages that SQL arguments set: the parameters of each message, the
//! arguments that each takes, and the chains of conversions that turn an argument into a message.
//!
//! A message's parameters are its required content, then each other field outside a oneof and
//! each oneof, in the order of their first field numbers. A field of `Encoding` type is a channel
//! and a `TableRef` is a table, so neither is a parameter, and a oneof loses its members of
//! either kind.
//!
//! The conversions turn an argument, a SQL value of one class or a fragment of a message, into a
//! message, one step at a time. Each step builds a default builder of a message and calls one
//! setter with the result of the step before it:
//!
//! - rule A: a message whose only required content is one field, from the argument of that field
//! - rule O: a message whose first field, by number, is a member of a oneof, and whose other fields
//!   are not required, from an argument of the oneof
//! - rule F: a message without required content, from a field that the `lift` option marks
//!
//! An argument of a oneof sets a member by the same steps: a name of a member without required
//! content in a oneof whose members take no strings, a fragment, or a value of a member's class.
//! A message or a oneof accepts an argument when exactly one shortest chain leads to it, where a
//! value that a member takes by its type precedes one that a `ValueRef` member takes.

use std::collections::{BTreeMap, BTreeSet};

use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto,
};

use super::{annotations::Annotations, snake, Messages, PACKAGE};

/// The most steps of a chain.
const MAX_STEPS: u32 = 8;

/// The class of a SQL argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Class {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Bytes,
    /// A value of another type, such as a date, which only a `ValueRef` takes.
    Other,
}

impl Class {
    pub(super) const ALL: [Class; 7] = [
        Class::Bool,
        Class::Int,
        Class::Float,
        Class::Str,
        Class::Array,
        Class::Bytes,
        Class::Other,
    ];

    pub(super) fn variant(self) -> &'static str {
        match self {
            Class::Bool => "Bool",
            Class::Int => "Int",
            Class::Float => "Float",
            Class::Str => "Str",
            Class::Array => "Array",
            Class::Bytes => "Bytes",
            Class::Other => "Other",
        }
    }
}

/// What a field holds, as an argument sets it.
#[derive(Clone, Debug)]
pub(super) enum Shape {
    Bool,
    /// An integer of the Rust type.
    Int(&'static str),
    /// A floating-point number of the Rust type.
    Float(&'static str),
    Str,
    Bytes,
    /// An enum, by its full name.
    Enum(String),
    Value,
    /// A message, by its full name.
    Message(String),
    Repeated(Box<Shape>),
}

impl Shape {
    pub(super) fn of(field: &FieldDescriptorProto) -> Option<Shape> {
        let item = match field.r#type() {
            Type::Bool => Shape::Bool,
            Type::Int32 | Type::Sint32 | Type::Sfixed32 => Shape::Int("i32"),
            Type::Int64 | Type::Sint64 | Type::Sfixed64 => Shape::Int("i64"),
            Type::Uint32 | Type::Fixed32 => Shape::Int("u32"),
            Type::Uint64 | Type::Fixed64 => Shape::Int("u64"),
            Type::Float => Shape::Float("f32"),
            Type::Double => Shape::Float("f64"),
            Type::String => Shape::Str,
            Type::Bytes => Shape::Bytes,
            Type::Enum => Shape::Enum(field.type_name().to_string()),
            Type::Message => match field.type_name().rsplit('.').next() {
                Some("ValueRef") => Shape::Value,
                Some("TableRef" | "Table" | "Encoding") => return None,
                _ => Shape::Message(field.type_name().to_string()),
            },
            Type::Group => return None,
        };
        Some(match field.label() == Label::Repeated {
            true => Shape::Repeated(Box::new(item)),
            false => item,
        })
    }

    /// The classes of the arguments that a field of this shape takes, with the precedence of each:
    /// a lower precedence picks a member before a higher one.
    pub(super) fn classes(&self) -> Vec<(Class, u8)> {
        match self {
            Shape::Bool => vec![(Class::Bool, 0)],
            Shape::Int(_) => vec![(Class::Int, 0)],
            Shape::Float(_) => vec![(Class::Float, 0), (Class::Int, 1)],
            Shape::Str | Shape::Enum(_) => vec![(Class::Str, 0)],
            Shape::Bytes => vec![(Class::Bytes, 0)],
            Shape::Repeated(_) => vec![(Class::Array, 0)],
            Shape::Value => [
                Class::Bool,
                Class::Int,
                Class::Float,
                Class::Str,
                Class::Array,
                Class::Other,
            ]
            .into_iter()
            .map(|class| (class, 2))
            .collect(),
            Shape::Message(_) => Vec::new(),
        }
    }

    /// Whether an argument of this shape is a string, so a member's name would be ambiguous.
    fn takes_strings(&self) -> bool {
        matches!(self, Shape::Str | Shape::Enum(_) | Shape::Value)
    }
}

/// A parameter of a message.
pub(super) struct Param<'a> {
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) kind: ParamKind<'a>,
}

pub(super) enum ParamKind<'a> {
    Field(&'a FieldDescriptorProto, Shape),
    /// A oneof, by its index in the message, with its members that are parameters.
    Oneof(usize, Vec<(&'a FieldDescriptorProto, Shape)>),
}

/// A message of the model.
pub(super) struct MessageModel<'a> {
    /// The full name, such as `.avenger.chart.v1.Scale`.
    pub(super) name: String,
    pub(super) message: &'a DescriptorProto,
    /// The name of the message's constructor, or of the function that applies its parameters.
    pub(super) function: String,
    pub(super) params: Vec<Param<'a>>,
}

/// A node of the conversion graph: a message, or a oneof of a message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Node {
    Message(String),
    Oneof(String, usize),
}

/// The source of a chain: an argument of a class, or a fragment of a message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Source {
    Class(Class),
    Fragment(String),
}

/// How a step sets its target.
#[derive(Clone, Debug)]
pub(super) enum Set<'a> {
    /// A field outside a oneof, through its setter.
    Field(&'a FieldDescriptorProto),
    /// A member of a oneof: its setter, or `with_` and the member's builder for a message.
    Member(&'a FieldDescriptorProto),
    /// A member that a string names, of the oneof of the message at its index.
    Names(String, usize),
}

/// What a step reads.
#[derive(Clone, Debug)]
pub(super) enum Input {
    /// An argument of the class, converted as the field of the step takes it.
    Class(Class, Shape),
    /// The message that the step before builds.
    Message(String),
    /// A string that names a member.
    Name,
}

/// One step of a chain.
pub(super) struct Edge<'a> {
    pub(super) to: Node,
    pub(super) input: Input,
    pub(super) set: Set<'a>,
    pub(super) precedence: u8,
}

/// The best chain to a node from one source: its steps, its precedence, the number of chains of
/// that length and precedence, up to 2, and the last edge of the one chain.
#[derive(Clone, Copy)]
struct Best {
    steps: u32,
    precedence: u8,
    count: u8,
    edge: Option<usize>,
}

/// The parameter model and the conversion graph.
pub(super) struct Model<'a> {
    pub(super) messages: Vec<MessageModel<'a>>,
    pub(super) edges: Vec<Edge<'a>>,
    /// The members that a string names, by the full name of their message and the index of their
    /// oneof: the members whose messages have no required content, in a oneof whose members take
    /// no strings.
    pub(super) named: BTreeMap<(String, usize), Vec<String>>,
    /// The best chain to each node from each source.
    best: BTreeMap<(Node, Source), Best>,
}

impl<'a> Model<'a> {
    /// The model of the messages `names`, from the descriptors.
    pub(super) fn new(
        messages: &'a Messages<'a>,
        annotations: &Annotations,
        names: &[String],
        excluded: &BTreeSet<(String, String)>,
    ) -> Result<Self, String> {
        let mut models = Vec::new();
        for name in names {
            let (_, _, message) = messages.get(name).ok_or(format!("`{name}` is a message"))?;
            let function = match annotations.constructor.get(name) {
                Some(function) => function.clone(),
                None => snake(message.name()),
            };
            models.push(MessageModel {
                name: name.clone(),
                message,
                function,
                params: params(name, message, annotations, excluded),
            });
        }
        let mut model = Model {
            edges: Vec::new(),
            named: BTreeMap::new(),
            messages: models,
            best: BTreeMap::new(),
        };
        (model.edges, model.named) = model.graph(messages, annotations);
        model.search(messages);
        Ok(model)
    }

    /// The model of the message `name`.
    pub(super) fn message(&self, name: &str) -> Option<&MessageModel<'a>> {
        self.messages.iter().find(|model| model.name == name)
    }

    /// The edges of the conversion graph.
    #[allow(clippy::type_complexity)]
    fn graph(
        &self,
        messages: &'a Messages<'a>,
        annotations: &Annotations,
    ) -> (Vec<Edge<'a>>, BTreeMap<(String, usize), Vec<String>>) {
        let mut edges = Vec::new();
        let mut named_members = BTreeMap::new();
        for model in &self.messages {
            let name = &model.name;
            let message = model.message;
            let required: Vec<&Param> = model.params.iter().filter(|p| p.required).collect();
            let node = Node::Message(name.clone());
            // Rule A: the one required field.
            if let [Param {
                kind: ParamKind::Field(field, shape),
                ..
            }] = required.as_slice()
            {
                edges.extend(field_edges(node.clone(), shape, Set::Field(field)));
            }
            // Rule O: the oneof of the first field, when the message requires nothing else.
            let first = message.field.iter().min_by_key(|field| field.number());
            let leading = first
                .and_then(|field| field.oneof_index.filter(|_| !field.proto3_optional()))
                .map(|index| index as usize);
            for param in &model.params {
                let ParamKind::Oneof(index, members) = &param.kind else {
                    continue;
                };
                let targets = match Some(*index) == leading
                    && required
                        .iter()
                        .all(|p| matches!(p.kind, ParamKind::Oneof(..)))
                {
                    true => vec![Node::Oneof(name.clone(), *index), node.clone()],
                    false => vec![Node::Oneof(name.clone(), *index)],
                };
                // A name picks a member without required content, which an empty message sets, in
                // a oneof whose members take no strings.
                let named = !members.iter().any(|(_, shape)| shape.takes_strings());
                let pickable: Vec<String> = (members.iter())
                    .filter(|(_, shape)| match shape {
                        Shape::Message(type_name) => {
                            named && !has_required(messages, annotations, type_name)
                        }
                        _ => false,
                    })
                    .map(|(member, _)| member.name().to_string())
                    .collect();
                if !pickable.is_empty() {
                    named_members.insert((name.clone(), *index), pickable.clone());
                }
                for target in targets {
                    for (member, shape) in members {
                        edges.extend(field_edges(target.clone(), shape, Set::Member(member)));
                    }
                    if !pickable.is_empty() {
                        edges.push(Edge {
                            to: target,
                            input: Input::Name,
                            set: Set::Names(name.clone(), *index),
                            precedence: 0,
                        });
                    }
                }
            }
            // Rule F: the lifted fields of a message without required content.
            if required.is_empty() {
                for field in &message.field {
                    let lifted = annotations
                        .lift
                        .contains(&(name.clone(), field.name().to_string()));
                    if let (true, Some(Shape::Message(_))) = (lifted, Shape::of(field)) {
                        edges.push(Edge {
                            to: node.clone(),
                            input: Input::Message(field.type_name().to_string()),
                            set: Set::Field(field),
                            precedence: 0,
                        });
                    }
                }
            }
        }
        // Only messages of the model have builders that a chain builds.
        edges.retain(|edge| match &edge.input {
            Input::Message(name) => self.message(name).is_some(),
            _ => true,
        });
        (edges, named_members)
    }

    /// The best chains from every source to every node.
    fn search(&mut self, messages: &Messages) {
        let mut sources: Vec<Source> = Class::ALL.iter().map(|c| Source::Class(*c)).collect();
        for model in &self.messages {
            let empty = messages
                .get(&model.name)
                .is_some_and(|(_, _, message)| message.field.is_empty());
            if !empty {
                sources.push(Source::Fragment(model.name.clone()));
            }
        }
        for source in sources {
            let mut best: BTreeMap<Node, Best> = BTreeMap::new();
            if let Source::Fragment(name) = &source {
                best.insert(
                    Node::Message(name.clone()),
                    Best {
                        steps: 0,
                        precedence: 0,
                        count: 1,
                        edge: None,
                    },
                );
            }
            for step in 1..=MAX_STEPS {
                let mut found: BTreeMap<Node, Vec<(u8, u8, usize)>> = BTreeMap::new();
                for (index, edge) in self.edges.iter().enumerate() {
                    if best.contains_key(&edge.to) {
                        continue;
                    }
                    let candidate = match (&edge.input, &source) {
                        (Input::Class(class, _), Source::Class(wanted)) if class == wanted => {
                            (step == 1).then_some((edge.precedence, 1))
                        }
                        (Input::Name, Source::Class(Class::Str)) => (step == 1).then_some((0, 1)),
                        (Input::Message(name), _) => best
                            .get(&Node::Message(name.clone()))
                            .filter(|found| found.steps == step - 1)
                            .map(|found| (found.precedence, found.count)),
                        _ => None,
                    };
                    if let Some((precedence, count)) = candidate {
                        found
                            .entry(edge.to.clone())
                            .or_default()
                            .push((precedence, count, index));
                    }
                }
                for (node, candidates) in found {
                    let precedence = candidates.iter().map(|c| c.0).min().expect("a candidate");
                    let winners: Vec<&(u8, u8, usize)> =
                        candidates.iter().filter(|c| c.0 == precedence).collect();
                    let count = winners.iter().map(|c| c.1).sum::<u8>().min(2);
                    let edge = (count == 1).then(|| winners[0].2);
                    best.insert(
                        node,
                        Best {
                            steps: step,
                            precedence,
                            count,
                            edge,
                        },
                    );
                }
            }
            for (node, found) in best {
                self.best.insert((node, source.clone()), found);
            }
        }
    }

    /// The chain from `source` to `node`, from its first edge to its last, when exactly one
    /// shortest chain leads there. A fragment of the node's own message has the empty chain.
    pub(super) fn chain(&self, node: &Node, source: &Source) -> Option<Vec<&Edge<'a>>> {
        let mut edges = Vec::new();
        let mut at = node.clone();
        loop {
            let found = self.best.get(&(at.clone(), source.clone()))?;
            if found.count != 1 {
                return None;
            }
            let Some(index) = found.edge else {
                edges.reverse();
                return Some(edges);
            };
            let edge = &self.edges[index];
            edges.push(edge);
            match &edge.input {
                Input::Message(name) => at = Node::Message(name.clone()),
                _ => {
                    edges.reverse();
                    return Some(edges);
                }
            }
        }
    }

    /// The sources that reach `node` through one chain, with their chains.
    pub(super) fn accepted(&self, node: &Node) -> Vec<(Source, Vec<&Edge<'a>>)> {
        let sources: BTreeSet<&Source> = (self.best.keys())
            .filter(|(at, _)| at == node)
            .map(|(_, source)| source)
            .collect();
        (sources.into_iter())
            .filter_map(|source| Some((source.clone(), self.chain(node, source)?)))
            .collect()
    }
}

/// The edges that set a field of `shape` on `to` through `set`: one for each class of argument that it takes,
/// or one from its message.
fn field_edges<'a>(to: Node, shape: &Shape, set: Set<'a>) -> Vec<Edge<'a>> {
    match shape {
        Shape::Message(type_name) => vec![Edge {
            to,
            input: Input::Message(type_name.clone()),
            set,
            precedence: 0,
        }],
        _ => (shape.classes().into_iter())
            .map(|(class, precedence)| Edge {
                to: to.clone(),
                input: Input::Class(class, shape.clone()),
                set: set.clone(),
                precedence,
            })
            .collect(),
    }
}

/// Whether the message `name` has required content.
fn has_required(messages: &Messages, annotations: &Annotations, name: &str) -> bool {
    let Some((_, _, message)) = messages.get(name) else {
        return false;
    };
    let fields = message
        .field
        .iter()
        .any(|f| annotations.is_required(name, f.name()));
    let oneofs = (message.oneof_decl.iter()).any(|o| annotations.is_required_oneof(name, o.name()));
    fields || oneofs
}

/// The parameters of the message `name`.
fn params<'a>(
    name: &str,
    message: &'a DescriptorProto,
    annotations: &Annotations,
    excluded: &BTreeSet<(String, String)>,
) -> Vec<Param<'a>> {
    let excluded = |field: &FieldDescriptorProto| {
        excluded.contains(&(name.to_string(), field.name().to_string()))
    };
    // Each parameter with its first field number.
    let mut params: Vec<(bool, i32, Param<'a>)> = Vec::new();
    let mut oneofs_done = BTreeSet::new();
    for field in (message.field.iter()).filter(|field| !excluded(field)) {
        match field.oneof_index.filter(|_| !field.proto3_optional()) {
            Some(index) => {
                let index = index as usize;
                if !oneofs_done.insert(index) {
                    continue;
                }
                let members: Vec<(&FieldDescriptorProto, Shape)> = (message.field.iter())
                    .filter(|member| {
                        member.oneof_index == Some(index as i32)
                            && !member.proto3_optional()
                            && !excluded(member)
                    })
                    .filter_map(|member| Some((member, Shape::of(member)?)))
                    .collect();
                if members.is_empty() {
                    continue;
                }
                let first = members.iter().map(|(m, _)| m.number()).min().unwrap_or(0);
                let oneof = message.oneof_decl[index].name();
                let required = annotations.is_required_oneof(name, oneof);
                params.push((
                    required,
                    first,
                    Param {
                        name: oneof.to_string(),
                        required,
                        kind: ParamKind::Oneof(index, members),
                    },
                ));
            }
            None => {
                let Some(shape) = Shape::of(field) else {
                    continue;
                };
                let required = annotations.is_required(name, field.name());
                params.push((
                    required,
                    field.number(),
                    Param {
                        name: field.name().to_string(),
                        required,
                        kind: ParamKind::Field(field, shape),
                    },
                ));
            }
        }
    }
    params.sort_by_key(|(required, number, _)| (!*required, *number));
    params.into_iter().map(|(_, _, param)| param).collect()
}

/// The full names of the messages whose parameters SQL arguments set: the messages of the option
/// tree, and `Encoding`, whose options `scaled` takes.
pub(super) fn parameter_messages(tree: &[String]) -> Vec<String> {
    let mut names = tree.to_vec();
    names.push(format!("{PACKAGE}.Encoding"));
    names
}

/// Checks that the constructors of the messages with fields have distinct names that are
/// lowercase identifiers.
pub(super) fn check_names(constructors: &[&MessageModel]) -> Result<(), String> {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for constructor in constructors {
        let name = constructor.function.as_str();
        let valid = name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(format!(
                "the constructor `{name}` of `{}` is not a lowercase identifier",
                constructor.message.name()
            ));
        }
        if let Some(other) = seen.insert(name, constructor.message.name()) {
            return Err(format!(
                "the messages `{other}` and `{}` both have the constructor `{name}`",
                constructor.message.name()
            ));
        }
    }
    Ok(())
}
