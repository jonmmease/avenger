//! The generation of `constructors.rs`, behind the `functions` feature: the table of the
//! constructors, a function for each message of the option tree with fields, except the marks,
//! and the function that builds the fragment of each from its arguments, through the code of the
//! parameter model in `parameters.rs`.

use std::fmt::Write;

use super::{
    builders::Comments,
    model::Model,
    parameters::{builder, doc, params_const},
};

/// The code of `constructors.rs`, where `constructors` names the messages with constructors.
pub(super) fn constructors(
    model: &Model,
    comments: &Comments,
    constructors: &[String],
) -> Result<String, String> {
    let mut code = String::from("pub(crate) static CONSTRUCTORS: &[Constructor] = &[\n");
    for name in constructors {
        let message = model.message(name).ok_or(format!("`{name}` has a model"))?;
        let function = &message.function;
        writeln!(
            code,
            "    Constructor {{ name: {function:?}, message: {:?}, doc: {:?}, params: {}, build: build_{function} }},",
            name.trim_start_matches('.'),
            doc(comments.messages.get(name)),
            params_const(message)
        )
        .unwrap();
    }
    code.push_str("];\n\n");
    code.push_str("/// The name of the constructor of the message with the full name `message`.\npub(crate) fn constructor_name(message: &str) -> Option<&'static str> {\n    Some(match message {\n");
    for name in constructors {
        let message = model.message(name).expect("a model");
        writeln!(
            code,
            "        {:?} => {:?},",
            name.trim_start_matches('.'),
            message.function
        )
        .unwrap();
    }
    code.push_str("        _ => return None,\n    })\n}\n\n");
    for name in constructors {
        let message = model.message(name).expect("a model");
        let function = &message.function;
        writeln!(
            code,
            "fn build_{function}(args: &[Option<Arg>]) -> Result<crate::proto::Fragment, ArgError> {{\n    let builder = apply_all_{function}({}::default(), args)?;\n    builder.to_fragment().map_err(ArgError::invalid)\n}}\n",
            builder(name)
        )
        .unwrap();
    }
    Ok(code)
}
