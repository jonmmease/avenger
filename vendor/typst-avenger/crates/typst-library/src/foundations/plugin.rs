use ecow::EcoString;
use typst_syntax::Spanned;

use crate::diag::{SourceResult, StrResult, bail};
use crate::engine::Engine;
use crate::foundations::{Bytes, Func, Module, Value, cast, func, scope};
use crate::loading::DataSource;

#[func(scope)]
pub fn plugin(
    engine: &mut Engine,
    /// A path to a WebAssembly file or raw WebAssembly bytes.
    _source: Spanned<DataSource>,
) -> SourceResult<Module> {
    let _ = engine;
    bail!(typst_syntax::Span::detached(), "plugins are not available in avenger-typst math fragments")
}

#[scope]
impl plugin {
    #[func]
    pub fn transition(
        /// The plugin function to call.
        func: PluginFunc,
        /// The byte buffers to call the function with.
        #[variadic]
        arguments: Vec<Bytes>,
    ) -> StrResult<Module> {
        func.transition(arguments)
    }
}

/// A function loaded from a WebAssembly plugin.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PluginFunc {
    name: EcoString,
}

impl PluginFunc {
    pub fn name(&self) -> &EcoString {
        &self.name
    }

    pub fn call(&self, _: Vec<Bytes>) -> StrResult<Bytes> {
        bail!("plugins are not available in avenger-typst math fragments")
    }

    pub fn transition(&self, _: Vec<Bytes>) -> StrResult<Module> {
        bail!("plugins are not available in avenger-typst math fragments")
    }
}

cast! {
    PluginFunc,
    self => Value::Func(self.into()),
    v: Func => v.to_plugin().ok_or("expected plugin function")?.clone(),
}
