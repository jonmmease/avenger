//! The function of a call by its place in the text: a constructor of a chart's option, by the name
//! that the parameter of its argument gives it, or a function of the library.
//!
//! A constructor is valid only in an argument of a chart function: a mark, a composition, a
//! channel function, or a constructor in them. A channel function is valid only as the value of
//! the channel of its name, as `color => color(species, legend => FALSE)`. A constructor's name
//! resolves against what the argument's parameter takes, its [`Takes`](super::Takes), so one name
//! builds different messages in different places, as `type => point(...)` is a point scale in
//! `scale(...)` and a point selection in `select(...)`, and `log(...)` in `type =>` is a log scale
//! where `LOG(x)` elsewhere is GoogleSQL's. The analysis, completion, hover, and signature help
//! resolve a call through [`called`], which reads only the syntax, so it works on incomplete text.

use avenger_sql_syntax::{
    ast::{self, AstNode},
    SyntaxKind, SyntaxNode,
};

use super::{Builds, Function, Library, Param};

/// The function of a call, with the name that the call writes.
#[derive(Clone, Debug)]
pub struct Called<'l> {
    pub function: &'l Function,
    /// The name that the call writes, or for a function of the library, its name.
    pub name: String,
    /// For a constructor, the place that it fills.
    pub fills: Option<Fills<'l>>,
}

/// The place that a constructor fills: the constructor that the parameter of its argument takes
/// under its name, and the path of the parameters to it from the outermost chart function.
#[derive(Clone, Debug)]
pub struct Fills<'l> {
    pub builds: &'l Builds,
    /// The constructor that wraps a constructor one level down, the parameter's own, by its
    /// unique name.
    pub target: Option<&'l str>,
    /// Whether the call is an element of an array.
    pub element: bool,
    /// The outermost chart function, as its call writes it.
    pub root: String,
    /// The parameters from the outermost chart function to the place, such as `y`, `scale`, and
    /// `pin`.
    pub path: Vec<String>,
}

impl Fills<'_> {
    /// The place in words, such as `` the `pin` of the `scale` of `y` of `symbol(...)` ``.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(part) = &self.builds.part {
            parts.push(format!("the `{part}`"));
        }
        for param in self.path.iter().rev() {
            parts.push(format!("the `{param}`"));
        }
        parts.push(format!("`{}(...)`", self.root));
        parts.join(" of ")
    }
}

/// The argument of a chart function that holds an expression directly, or as an element of an
/// array, or in parentheses: the call that it is an argument of, and its parameter.
#[derive(Clone, Debug)]
pub struct Argument<'l> {
    pub owner: Called<'l>,
    pub param: &'l Param,
    /// Whether the expression is an element of an array.
    pub element: bool,
}

impl Argument<'_> {
    /// The outermost chart function and the parameters to the argument's parameter.
    pub fn path(&self) -> (String, Vec<String>) {
        let (root, mut path) = match &self.owner.fills {
            Some(fills) => {
                let mut path = fills.path.clone();
                if let Some(part) = &fills.builds.part {
                    path.push(part.clone());
                }
                (fills.root.clone(), path)
            }
            None => (self.owner.name.clone(), Vec::new()),
        };
        path.push(self.param.name.clone());
        (root, path)
    }
}

/// The argument whose value is the expression `node`, of a call of a function of the library or
/// of a constructor.
pub fn argument<'l>(library: &'l Library, node: &SyntaxNode) -> Option<Argument<'l>> {
    let mut node = node.clone();
    let mut element = false;
    let argument = loop {
        let parent = node.parent()?;
        match parent.kind() {
            SyntaxKind::PAREN_EXPR => node = parent,
            SyntaxKind::ARRAY_EXPR if !element => {
                element = true;
                node = parent;
            }
            SyntaxKind::ARG | SyntaxKind::NAMED_ARG => break parent,
            _ => return None,
        }
    };
    let list = ast::ArgList::cast(argument.parent()?)?;
    let call = list.syntax().parent()?;
    let owner = called(library, &call)?;
    let name = match ast::NamedArg::cast(argument.clone()) {
        Some(named) => named.name_ref()?.text()?,
        None => {
            let position = (list.arguments())
                .filter(|other| matches!(other, ast::Argument::Arg(_)))
                .take_while(|other| other.syntax() != &argument)
                .count();
            owner.function.positional_param(position)?
        }
    };
    let param = owner.function.param_named(&name)?;
    Some(Argument {
        owner,
        param,
        element,
    })
}

/// The function of the call `node`: the constructor that the parameter of its argument takes under
/// its name, or the function of the library of its name.
pub fn called<'l>(library: &'l Library, node: &SyntaxNode) -> Option<Called<'l>> {
    let call = ast::CallExpr::cast(node.clone())?;
    let names = call.path()?.names()?;
    if let [name] = names.as_slice() {
        if let Some(called) = constructor(library, node, name) {
            return Some(called);
        }
    }
    let name = match names.as_slice() {
        [name] => name,
        [safe, name] if safe.eq_ignore_ascii_case("SAFE") => name,
        _ => return None,
    };
    let function = library.get(name)?;
    Some(Called {
        function,
        name: function.name.clone(),
        fills: None,
    })
}

/// The constructor named `name` that the argument whose value is the call `node` takes.
pub fn constructor<'l>(library: &'l Library, node: &SyntaxNode, name: &str) -> Option<Called<'l>> {
    let argument = argument(library, node)?;
    contextual(library, &argument, name)
}

/// The constructor named `name` that `argument` takes.
pub fn contextual<'l>(
    library: &'l Library,
    argument: &Argument<'l>,
    name: &str,
) -> Option<Called<'l>> {
    let builds = argument.param.takes.find(name)?;
    let function = library.constructor(&builds.constructor)?;
    let (root, path) = argument.path();
    Some(Called {
        function,
        name: builds.name.clone(),
        fills: Some(Fills {
            builds,
            target: argument.param.takes.target.as_deref(),
            element: argument.element,
            root,
            path,
        }),
    })
}

/// The suggestion for a constructor named `name` that `argument` does not take: a parameter beside
/// the argument's that takes it, as `` `type => log(...)` ``, or a parameter of the argument's own
/// constructor, as `` `scale => scale(type => log(...))` ``.
pub fn suggestion(argument: &Argument, name: &str, library: &Library) -> Option<String> {
    let param = &argument.param.name;
    let sibling = (argument.owner.function.params.iter())
        .find(|other| other.name != *param && other.takes.find(name).is_some());
    if let Some(sibling) = sibling {
        let builds = sibling.takes.find(name)?;
        return Some(match builds.array {
            true => format!("`{} => [{name}(...)]`", sibling.name),
            false => format!("`{} => {name}(...)`", sibling.name),
        });
    }
    let own = (argument.param.takes.constructors.iter()).find(|builds| builds.part.is_none())?;
    let target = library.constructor(&own.constructor)?;
    let inner = (target.params.iter()).find(|inner| inner.takes.find(name).is_some())?;
    Some(format!(
        "`{param} => {}({} => {name}(...))`",
        own.name, inner.name
    ))
}
