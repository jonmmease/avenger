//! The parameter model of the messages that SQL arguments set: the parameters of each message, the
//! arguments that each takes, and the conversions of a plain value to a message.
//!
//! A message's parameters are its required content, then each other field outside a oneof and
//! each oneof, in the order of their first field numbers. A field of `Encoding` type is a channel
//! and a `TableRef` is a table, so neither is a parameter, and a oneof loses its members of
//! either kind. A oneof whose members all take one kind of plain argument, as `options::splits`
//! describes, gives a parameter for each member, of which a call gives at most one, and so does
//! the oneof of a message that holds only a choice, such as a tool, whose members are then the
//! parameters of its constructor. A map, the named options of a custom scale or a custom
//! projection, is a parameter that takes a struct, one option for each field.
//!
//! A parameter takes three forms of argument, of which the format sees two, since the analysis of
//! avenger-sql wraps the third in the second:
//!
//! 1. A plain value: a value of a plain field, the values of a oneof's plain members, a string
//!    that names a member without required content, or a shorthand of a field's message.
//! 2. A fragment of the parameter's own constructor: of the field's message, or of the oneof's
//!    constructor, which builds the oneof of its message.
//!
//! The shorthands of a message are the fields that the `lift` option marks and the oneofs that the
//! `lift_oneof` option marks, which a plain value fills in one step: a lifted field by its own
//! value, and a lifted oneof by the values of its plain members, by the name of a member, or,
//! after both, by a shorthand of a member's message. A shorthand converts to the message only when
//! the field that it fills is the message's only required content, and the first parameter of the
//! message's constructor takes every shorthand. The build fails when a lift marks a field that
//! holds a message or one value reaches two lifted fields of a message.

use std::collections::{BTreeMap, BTreeSet};

use prost_types::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, FieldDescriptorProto,
};

use super::{annotations::Annotations, snake, Messages, PACKAGE};

/// The precedence that a value of a member's message, through its shorthand, takes after the
/// values of the plain members of its oneof.
const MEMBER_SHORTHAND: u8 = 4;

/// The class of a SQL argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Class {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Bytes,
    /// A struct, whose fields are named values.
    Struct,
    /// A value of another type, such as a date, which only a `ValueRef` takes.
    Other,
}

impl Class {
    pub(super) fn variant(self) -> &'static str {
        match self {
            Class::Bool => "Bool",
            Class::Int => "Int",
            Class::Float => "Float",
            Class::Str => "Str",
            Class::Array => "Array",
            Class::Bytes => "Bytes",
            Class::Struct => "Struct",
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
    /// A string of Typst markup, which the `markup` option marks.
    Markup,
    Bytes,
    /// An enum, by its full name.
    Enum(String),
    Value,
    /// A message, by its full name.
    Message(String),
    Repeated(Box<Shape>),
    /// A map from a name to an `OptionValue`, the named options of a custom scale or a custom
    /// projection, which a struct sets.
    Options,
}

/// Whether the message `name` is the entry of a map field. The only nested messages of the package
/// are the entries of maps, as build.rs checks.
pub(super) fn is_map_entry(name: &str) -> bool {
    (name.strip_prefix(PACKAGE)).is_some_and(|name| name.matches('.').count() > 1)
}

impl Shape {
    pub(super) fn of(field: &FieldDescriptorProto) -> Option<Shape> {
        if field.r#type() == Type::Message && is_map_entry(field.type_name()) {
            // The value of the entry is its field `value`, which the name of the entry type does
            // not give, so the map's value is checked where the map is read.
            return Some(Shape::Options);
        }
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
            Shape::Str | Shape::Markup | Shape::Enum(_) => vec![(Class::Str, 0)],
            Shape::Bytes => vec![(Class::Bytes, 0)],
            Shape::Repeated(_) => vec![(Class::Array, 0)],
            Shape::Options => vec![(Class::Struct, 0)],
            Shape::Value => [
                Class::Bool,
                Class::Int,
                Class::Float,
                Class::Str,
                Class::Array,
                Class::Struct,
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
        matches!(
            self,
            Shape::Str | Shape::Markup | Shape::Enum(_) | Shape::Value
        )
    }

    /// The shape of the field `field` of the message `message`: a field of strings that the
    /// `markup` option marks holds markup.
    pub(super) fn of_field(
        message: &str,
        field: &FieldDescriptorProto,
        annotations: &Annotations,
    ) -> Option<Shape> {
        let shape = Shape::of(field)?;
        if !annotations.is_markup(message, field.name()) {
            return Some(shape);
        }
        Some(match shape {
            Shape::Str => Shape::Markup,
            Shape::Repeated(item) if matches!(*item, Shape::Str) => {
                Shape::Repeated(Box::new(Shape::Markup))
            }
            shape => shape,
        })
    }

    /// Whether the shape holds a message, which a lift never marks.
    fn holds_message(&self) -> bool {
        match self {
            Shape::Message(_) => true,
            Shape::Repeated(item) => item.holds_message(),
            _ => false,
        }
    }
}

/// A parameter of a message.
pub(super) struct Param<'a> {
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) kind: ParamKind<'a>,
    /// The index of the oneof of a member that is a parameter of its own, as `options::splits`
    /// describes, or as the oneof of a message that holds only a choice gives: a call gives at
    /// most one of the parameters of one oneof.
    pub(super) exclusive: Option<usize>,
    /// The name of the constructor of the parameter's own form: the field's name for a message
    /// field, the oneof's name for a oneof, the `element` option of a repeated message field, and
    /// the member's name for a member of a message. `None` for a parameter of plain values.
    pub(super) constructor: Option<String>,
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
    /// The Rust name of the message's functions, its name in snake case, which is unique.
    pub(super) function: String,
    pub(super) params: Vec<Param<'a>>,
    /// The shorthands of the message, each class once: the chains that a plain value of the class
    /// takes to the message, whose last step sets a lifted field or oneof.
    pub(super) shorthand: Vec<Chain<'a>>,
    /// The parameter that a shorthand of a class fills, by the index of the chain in `shorthand`,
    /// or `None` for a member that a string names, which a call learns only from the string.
    pub(super) filled: Vec<Option<usize>>,
    /// Whether the message's first constructor parameter is a lifted parameter itself, a field of
    /// plain values that is the message's only lift, in place of a parameter `value` of its own.
    pub(super) merged: Option<usize>,
}

impl MessageModel<'_> {
    /// Whether the message has shorthands, so its constructor's first parameter takes them.
    pub(super) fn has_shorthand(&self) -> bool {
        !self.shorthand.is_empty()
    }

    /// The shorthands that convert to the message alone: those whose field is the message's only
    /// required content.
    pub(super) fn bare(&self) -> Vec<&Chain<'_>> {
        (self.shorthand.iter().zip(&self.filled))
            .filter(|(_, filled)| {
                (self.params.iter().enumerate())
                    .all(|(index, param)| !param.required || Some(index) == **filled)
            })
            .map(|(chain, _)| chain)
            .collect()
    }
}

/// A target of a step: a message, built from its default builder, or a oneof of a message, set on
/// a builder of the message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Node {
    Message(String),
    Oneof(String, usize),
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
    /// An argument, converted as the field of the step, of this shape, takes it.
    Class(Shape),
    /// The message that the step before builds.
    Message,
    /// A string that names a member.
    Name,
}

/// One step of a chain.
#[derive(Clone, Debug)]
pub(super) struct Edge<'a> {
    pub(super) to: Node,
    pub(super) input: Input,
    pub(super) set: Set<'a>,
}

/// The steps that turn a plain value of a class into a target, from the step that reads the value
/// to the step that sets the target, with the precedence of the chain among the chains of its
/// class.
#[derive(Clone, Debug)]
pub(super) struct Chain<'a> {
    pub(super) class: Class,
    pub(super) precedence: u8,
    pub(super) edges: Vec<Edge<'a>>,
}

/// The parameter model.
pub(super) struct Model<'a> {
    pub(super) messages: Vec<MessageModel<'a>>,
    /// The members that a string names, by the full name of their message and the index of their
    /// oneof: the members whose messages have no required content, in a oneof whose members take
    /// no strings.
    pub(super) named: BTreeMap<(String, usize), Vec<String>>,
    /// The values of each oneof parameter, each class once.
    oneof_values: BTreeMap<(String, usize), Vec<Chain<'a>>>,
    /// The lifted fields and oneofs, by the full name of their message and their name.
    lifts: BTreeSet<(String, String)>,
    lifted_oneofs: BTreeSet<(String, String)>,
}

impl<'a> Model<'a> {
    /// The model of the messages `names`, from the descriptors.
    pub(super) fn new(
        messages: &'a Messages<'a>,
        annotations: &Annotations,
        names: &[String],
        excluded: &BTreeSet<(String, String)>,
    ) -> Result<Self, String> {
        for (message, field) in &annotations.markup {
            let strings = (messages.get(message))
                .and_then(|(_, _, descriptor)| {
                    (descriptor.field.iter()).find(|f| f.name() == field)
                })
                .is_some_and(|field| field.r#type() == Type::String);
            if !strings {
                return Err(format!(
                    "`markup` marks `{message}.{field}`, which is not a field of strings"
                ));
            }
        }
        let mut models = Vec::new();
        for name in names {
            let (_, _, message) = messages.get(name).ok_or(format!("`{name}` is a message"))?;
            models.push(MessageModel {
                name: name.clone(),
                message,
                function: snake(message.name()),
                params: params(name, message, annotations, excluded)?,
                shorthand: Vec::new(),
                filled: Vec::new(),
                merged: None,
            });
        }
        let mut model = Model {
            messages: models,
            named: BTreeMap::new(),
            oneof_values: BTreeMap::new(),
            lifts: annotations.lift.clone(),
            lifted_oneofs: annotations.lift_oneof.clone(),
        };
        model.named = model.named_members(messages, annotations);
        // The values of each oneof, then the shorthands of each message, which read the
        // shorthands of the messages of a lifted oneof's members.
        let mut values = BTreeMap::new();
        for message in &model.messages {
            for param in &message.params {
                if let ParamKind::Oneof(index, members) = &param.kind {
                    let node = Node::Oneof(message.name.clone(), *index);
                    let chains = model.oneof_chains(&message.name, *index, members, node, 0)?;
                    values.insert((message.name.clone(), *index), resolve(chains));
                }
            }
        }
        model.oneof_values = values;
        let mut shorthands = Vec::new();
        for message in &model.messages {
            shorthands.push(model.shorthands(message, annotations, 0)?);
        }
        for (message, (shorthand, filled, merged)) in model.messages.iter_mut().zip(shorthands) {
            message.shorthand = shorthand;
            message.filled = filled;
            message.merged = merged;
        }
        Ok(model)
    }

    /// The model of the message `name`.
    pub(super) fn message(&self, name: &str) -> Option<&MessageModel<'a>> {
        self.messages.iter().find(|model| model.name == name)
    }

    /// The values of the oneof at `index` of the message `name`, each class once.
    pub(super) fn oneof_values(&self, name: &str, index: usize) -> &[Chain<'a>] {
        (self.oneof_values.get(&(name.to_string(), index))).map_or(&[], Vec::as_slice)
    }

    /// The members that a string names in each oneof of the model.
    fn named_members(
        &self,
        messages: &Messages,
        annotations: &Annotations,
    ) -> BTreeMap<(String, usize), Vec<String>> {
        let mut named = BTreeMap::new();
        for model in &self.messages {
            let mut oneofs: BTreeMap<usize, Vec<(&FieldDescriptorProto, &Shape)>> = BTreeMap::new();
            for param in &model.params {
                match (&param.kind, param.exclusive) {
                    (ParamKind::Oneof(index, members), _) => {
                        let entry = oneofs.entry(*index).or_default();
                        entry.extend(members.iter().map(|(member, shape)| (*member, shape)));
                    }
                    (ParamKind::Field(member, shape), Some(index)) => {
                        oneofs.entry(index).or_default().push((member, shape));
                    }
                    _ => {}
                }
            }
            for (index, members) in oneofs {
                // A name picks a member without required content, which an empty message sets,
                // in a oneof whose members take no strings.
                if members.iter().any(|(_, shape)| shape.takes_strings()) {
                    continue;
                }
                let pickable: Vec<String> = (members.iter())
                    .filter(|(_, shape)| match shape {
                        Shape::Message(type_name) => {
                            !has_required(messages, annotations, type_name)
                        }
                        _ => false,
                    })
                    .map(|(member, _)| member.name().to_string())
                    .collect();
                if !pickable.is_empty() {
                    named.insert((model.name.clone(), index), pickable);
                }
            }
        }
        named
    }

    /// The chains of the values of the oneof at `index` of `name` to `to`: the values of its plain
    /// members, the names of its members, and the shorthands of its members' messages. `depth`
    /// stops a shorthand of a member from reaching through another oneof.
    fn oneof_chains(
        &self,
        name: &str,
        index: usize,
        members: &[(&'a FieldDescriptorProto, Shape)],
        to: Node,
        depth: u8,
    ) -> Result<Vec<Chain<'a>>, String> {
        let mut chains = Vec::new();
        // The first `ValueRef` member by number takes values before the other `ValueRef`
        // members, as an explicit domain does before a summary.
        let first_value = (members.iter())
            .filter(|(_, shape)| matches!(shape, Shape::Value))
            .map(|(member, _)| member.number())
            .min();
        for (member, shape) in members {
            match shape {
                Shape::Message(type_name) => {
                    if depth > 0 {
                        continue;
                    }
                    let Some(model) = self.message(type_name) else {
                        continue;
                    };
                    for chain in self.raw_shorthands(model, depth + 1)? {
                        let mut edges = chain.edges.clone();
                        edges.push(Edge {
                            to: to.clone(),
                            input: Input::Message,
                            set: Set::Member(member),
                        });
                        chains.push(Chain {
                            class: chain.class,
                            precedence: chain.precedence + MEMBER_SHORTHAND,
                            edges,
                        });
                    }
                }
                _ => {
                    let later =
                        matches!(shape, Shape::Value) && Some(member.number()) != first_value;
                    for (class, precedence) in shape.classes() {
                        chains.push(Chain {
                            class,
                            precedence: precedence + u8::from(later),
                            edges: vec![Edge {
                                to: to.clone(),
                                input: Input::Class(shape.clone()),
                                set: Set::Member(member),
                            }],
                        });
                    }
                }
            }
        }
        if self.named.contains_key(&(name.to_string(), index)) {
            chains.push(Chain {
                class: Class::Str,
                precedence: 0,
                edges: vec![Edge {
                    to,
                    input: Input::Name,
                    set: Set::Names(name.to_string(), index),
                }],
            });
        }
        Ok(chains)
    }

    /// Whether the oneof at `index` of `model` is lifted.
    fn lifted_oneof(&self, model: &MessageModel, index: usize) -> bool {
        let oneof = model.message.oneof_decl[index].name().to_string();
        self.lifted_oneofs.contains(&(model.name.clone(), oneof))
    }

    /// The chains of the shorthands of `model` before resolution, each with the lifted parameter
    /// that it fills.
    fn raw_shorthands(
        &self,
        model: &MessageModel<'a>,
        depth: u8,
    ) -> Result<Vec<Chain<'a>>, String> {
        Ok(self
            .lifted_chains(model, depth)?
            .into_iter()
            .map(|(chain, _)| chain)
            .collect())
    }

    /// The chains of the shorthands of `model` before resolution, each with the index of the
    /// parameter that it fills, or `None` for a named member of a choice that the message's
    /// parameters split.
    fn lifted_chains(
        &self,
        model: &MessageModel<'a>,
        depth: u8,
    ) -> Result<Vec<(Chain<'a>, Option<usize>)>, String> {
        let name = &model.name;
        let node = Node::Message(name.clone());
        let mut chains = Vec::new();
        let mut oneofs_done = BTreeSet::new();
        for (index, param) in model.params.iter().enumerate() {
            match &param.kind {
                ParamKind::Field(field, shape) if param.exclusive.is_none() => {
                    if !self
                        .lifts
                        .contains(&(name.clone(), field.name().to_string()))
                    {
                        continue;
                    }
                    for (class, precedence) in shape.classes() {
                        chains.push((
                            Chain {
                                class,
                                precedence,
                                edges: vec![Edge {
                                    to: node.clone(),
                                    input: Input::Class(shape.clone()),
                                    set: Set::Field(field),
                                }],
                            },
                            Some(index),
                        ));
                    }
                }
                ParamKind::Field(field, shape) => {
                    // A member of a split oneof or of a choice: its own lift, or the oneof's.
                    if self
                        .lifts
                        .contains(&(name.clone(), field.name().to_string()))
                    {
                        for (class, precedence) in shape.classes() {
                            chains.push((
                                Chain {
                                    class,
                                    precedence,
                                    edges: vec![Edge {
                                        to: node.clone(),
                                        input: Input::Class(shape.clone()),
                                        set: Set::Member(field),
                                    }],
                                },
                                Some(index),
                            ));
                        }
                        continue;
                    }
                    let oneof = param.exclusive.expect("a member");
                    if !oneofs_done.insert(oneof) || !self.lifted_oneof(model, oneof) {
                        continue;
                    }
                    let members: Vec<(&'a FieldDescriptorProto, Shape)> = (model.params.iter())
                        .filter(|other| other.exclusive == Some(oneof))
                        .filter_map(|other| match &other.kind {
                            ParamKind::Field(member, shape) => Some((*member, shape.clone())),
                            ParamKind::Oneof(..) => None,
                        })
                        .collect();
                    for chain in self.oneof_chains(name, oneof, &members, node.clone(), depth)? {
                        // The member that the chain sets, when its last step names it.
                        let filled = match &chain.edges.last().expect("a step").set {
                            Set::Member(member) => (model.params.iter()).position(|other| {
                                matches!(other.kind, ParamKind::Field(field, _) if field.name() == member.name())
                            }),
                            _ => None,
                        };
                        chains.push((chain, filled));
                    }
                }
                ParamKind::Oneof(oneof, members) => {
                    if !self.lifted_oneof(model, *oneof) {
                        continue;
                    }
                    for chain in self.oneof_chains(name, *oneof, members, node.clone(), depth)? {
                        chains.push((chain, Some(index)));
                    }
                }
            }
        }
        Ok(chains)
    }

    /// The shorthands of `model`, each class once, with the parameter that each fills and the
    /// lifted parameter that is the constructor's first parameter itself, after the checks of the
    /// rules of lifts.
    #[allow(clippy::type_complexity)]
    fn shorthands(
        &self,
        model: &MessageModel<'a>,
        annotations: &Annotations,
        depth: u8,
    ) -> Result<(Vec<Chain<'a>>, Vec<Option<usize>>, Option<usize>), String> {
        let name = &model.name;
        let short = model.message.name();
        // Each lift marks a parameter of plain values of the message.
        for (message, field) in &annotations.lift {
            if message != name {
                continue;
            }
            let param = (model.params.iter())
                .find(|param| matches!(param.kind, ParamKind::Field(of, _) if of.name() == field));
            match param {
                // A lift of a field that is no parameter, as the column of a facet, which its
                // composition sets, serves the builders only.
                None => {}
                Some(Param {
                    kind: ParamKind::Field(_, shape),
                    ..
                }) if shape.holds_message() => {
                    return Err(format!(
                        "`lift` marks `{short}.{field}`, which holds a message, and a shorthand \
                         fills a field of a plain value"
                    ))
                }
                Some(_) => {}
            }
        }
        for (message, oneof) in &annotations.lift_oneof {
            if message != name {
                continue;
            }
            let index = (model.message.oneof_decl.iter()).position(|o| o.name() == oneof);
            let param = index.filter(|index| {
                (model.params.iter()).any(|param| match &param.kind {
                    ParamKind::Oneof(of, _) => of == index,
                    ParamKind::Field(..) => param.exclusive == Some(*index),
                })
            });
            if param.is_none() {
                return Err(format!(
                    "`lift_oneof` marks `{short}.{oneof}`, which is not a parameter"
                ));
            }
        }
        let chains = self.lifted_chains(model, depth)?;
        // One kind of value reaches one lifted parameter, so a value fills one field.
        let mut by_class: BTreeMap<Class, BTreeSet<String>> = BTreeMap::new();
        for (chain, filled) in &chains {
            let param = match filled {
                Some(index) => model.params[*index].exclusive.map_or_else(
                    || model.params[*index].name.clone(),
                    |oneof| model.message.oneof_decl[oneof].name().to_string(),
                ),
                None => match &chain.edges.last().expect("a step").set {
                    Set::Names(_, oneof) => model.message.oneof_decl[*oneof].name().to_string(),
                    _ => unreachable!("a chain without a parameter names a member"),
                },
            };
            by_class.entry(chain.class).or_default().insert(param);
        }
        if let Some((class, params)) = by_class.iter().find(|(_, params)| params.len() > 1) {
            let params: Vec<String> = params.iter().map(|p| format!("`{p}`")).collect();
            return Err(format!(
                "the lifted fields {} of `{short}` both take a value of the class {}, so a \
                 shorthand would fill either",
                params.join(" and "),
                class.variant()
            ));
        }
        let (chains, filled): (Vec<Chain>, Vec<Option<usize>>) =
            resolve_filled(chains).into_iter().unzip();
        // The constructor's first parameter is a lifted field itself when it is the message's only
        // lift and takes plain values, as its shorthand does.
        let lifted: BTreeSet<usize> = filled.iter().flatten().copied().collect();
        let merged = match (
            lifted.iter().collect::<Vec<_>>().as_slice(),
            filled.contains(&None),
        ) {
            ([index], false) => match &model.params[**index] {
                Param {
                    kind: ParamKind::Field(_, _),
                    exclusive: None,
                    ..
                } => Some(**index),
                _ => None,
            },
            _ => None,
        };
        if !chains.is_empty() && merged.is_none() && model.params.iter().any(|p| p.name == "value")
        {
            return Err(format!(
                "`{short}` has a parameter `value`, which is the name of the first parameter of a \
                 constructor whose shorthands fill several fields or a oneof"
            ));
        }
        Ok((chains, filled, merged))
    }
}

/// The chains of each class with the least precedence, when exactly one chain has it.
fn resolve(chains: Vec<Chain>) -> Vec<Chain> {
    resolve_filled(chains.into_iter().map(|chain| (chain, None)).collect())
        .into_iter()
        .map(|(chain, _)| chain)
        .collect()
}

/// The chains of each class with the least precedence, with what they fill, when exactly one
/// chain has it.
fn resolve_filled<'a>(chains: Vec<(Chain<'a>, Option<usize>)>) -> Vec<(Chain<'a>, Option<usize>)> {
    let mut by_class: BTreeMap<Class, Vec<(Chain<'a>, Option<usize>)>> = BTreeMap::new();
    for (chain, filled) in chains {
        by_class
            .entry(chain.class)
            .or_default()
            .push((chain, filled));
    }
    let mut resolved = Vec::new();
    for (_, mut candidates) in by_class {
        let least = candidates
            .iter()
            .map(|(c, _)| c.precedence)
            .min()
            .expect("a chain");
        candidates.retain(|(chain, _)| chain.precedence == least);
        if candidates.len() == 1 {
            resolved.push(candidates.pop().expect("one chain"));
        }
    }
    resolved
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
) -> Result<Vec<Param<'a>>, String> {
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
                    .filter_map(|member| {
                        Some((member, Shape::of_field(name, member, annotations)?))
                    })
                    .collect();
                if members.is_empty() {
                    continue;
                }
                let oneof = message.oneof_decl[index].name();
                let required = annotations.is_required_oneof(name, oneof);
                let fields: Vec<&FieldDescriptorProto> = members.iter().map(|(m, _)| *m).collect();
                let splits = super::options::splits(&fields);
                if required && splits {
                    return Err(format!(
                        "the required oneof `{oneof}` of `{name}` has members that one value \
                         cannot tell apart, so no argument sets it"
                    ));
                }
                if splits {
                    for (member, shape) in members {
                        params.push((false, member.number(), member_param(member, shape, index)));
                    }
                    continue;
                }
                let first = members.iter().map(|(m, _)| m.number()).min().unwrap_or(0);
                params.push((
                    required,
                    first,
                    Param {
                        name: oneof.to_string(),
                        required,
                        kind: ParamKind::Oneof(index, members),
                        exclusive: None,
                        constructor: Some(oneof.to_string()),
                    },
                ));
            }
            None => {
                let Some(shape) = Shape::of_field(name, field, annotations) else {
                    continue;
                };
                let required = annotations.is_required(name, field.name());
                let key = (name.to_string(), field.name().to_string());
                let element = annotations.element.get(&key);
                let constructor = match &shape {
                    Shape::Message(_) => Some(field.name().to_string()),
                    Shape::Repeated(item) if matches!(**item, Shape::Message(_)) => {
                        element.cloned()
                    }
                    _ => None,
                };
                if element.is_some() && constructor.is_none() {
                    return Err(format!(
                        "`element` names the elements of `{name}.{}`, which is not a repeated \
                         message field",
                        field.name()
                    ));
                }
                params.push((
                    required,
                    field.number(),
                    Param {
                        name: field.name().to_string(),
                        required,
                        kind: ParamKind::Field(field, shape),
                        exclusive: None,
                        constructor,
                    },
                ));
            }
        }
    }
    params.sort_by_key(|(required, number, _)| (!*required, *number));
    let mut params: Vec<Param<'a>> = params.into_iter().map(|(_, _, param)| param).collect();
    // A message that holds only a choice takes the choice's members as its parameters, as a tool
    // takes `navigate`, `tooltip`, and `select`.
    if let [Param {
        kind: ParamKind::Oneof(..),
        ..
    }] = params.as_slice()
    {
        let Some(Param {
            kind: ParamKind::Oneof(index, members),
            ..
        }) = params.pop()
        else {
            unreachable!("one oneof")
        };
        params = (members.into_iter())
            .map(|(member, shape)| member_param(member, shape, index))
            .collect();
    }
    Ok(params)
}

/// The parameter of a member of the oneof at `index` that is a parameter of its own.
fn member_param<'a>(member: &'a FieldDescriptorProto, shape: Shape, index: usize) -> Param<'a> {
    let constructor = matches!(shape, Shape::Message(_)).then(|| member.name().to_string());
    Param {
        name: member.name().to_string(),
        required: false,
        kind: ParamKind::Field(member, shape),
        exclusive: Some(index),
        constructor,
    }
}

/// Checks the rules of the element names of the repeated message fields of `models`: every such
/// field of a message whose element does not hold only a choice has one.
pub(super) fn check_elements(model: &Model) -> Result<(), String> {
    for message in &model.messages {
        for param in &message.params {
            let ParamKind::Field(field, Shape::Repeated(item)) = &param.kind else {
                continue;
            };
            let Shape::Message(element) = item.as_ref() else {
                continue;
            };
            let choice = model.message(element).is_some_and(|element| {
                !element.params.is_empty() && element.params.iter().all(|p| p.exclusive.is_some())
            });
            if param.constructor.is_none() && !choice {
                return Err(format!(
                    "the repeated field `{}.{}` needs the `element` option, the name of the \
                     constructor of its elements",
                    message.message.name(),
                    field.name()
                ));
            }
        }
    }
    Ok(())
}

/// The full names of the messages whose parameters SQL arguments set: the messages of the option
/// tree, and `Encoding`, whose options the channel functions take.
pub(super) fn parameter_messages(tree: &[String]) -> Vec<String> {
    let mut names = tree.to_vec();
    names.push(format!("{PACKAGE}.Encoding"));
    names
}
