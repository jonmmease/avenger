//! Ported from crates/typst-library/src/foundations/module.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: modules are only built in (`sym`, `emoji`, `math`), so they have no content and
//! no file.

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use ecow::{EcoString, eco_format};

use crate::typst_library::diag::{StrResult, WarningSink, bail};
use crate::typst_library::foundations::{Repr, Scope, Value, ty};

/// A collection of variables and functions that are commonly related to a
/// single theme.
#[derive(Clone)]
pub struct Module {
    /// The module's name.
    name: Option<EcoString>,
    /// The reference-counted inner fields.
    inner: Arc<ModuleInner>,
}

ty!(Module, name = "module", title = "Module", long = "module");

/// The internal representation of a [`Module`].
#[derive(Debug, Clone)]
struct ModuleInner {
    /// The top-level definitions that were bound in this module.
    scope: Scope,
}

impl Module {
    /// Create a new module.
    pub fn new(name: impl Into<EcoString>, scope: Scope) -> Self {
        Self {
            name: Some(name.into()),
            inner: Arc::new(ModuleInner { scope }),
        }
    }

    /// Get the module's name.
    pub fn name(&self) -> Option<&EcoString> {
        self.name.as_ref()
    }

    /// Access the module's scope.
    pub fn scope(&self) -> &Scope {
        &self.inner.scope
    }

    /// Try to access a definition in the module.
    pub fn field(&self, field: &str, sink: impl WarningSink) -> StrResult<&Value> {
        match self.scope().get(field) {
            Some(binding) => Ok(binding.read_checked(sink)),
            None => match &self.name {
                Some(name) => bail!("module `{name}` does not contain `{field}`"),
                None => bail!("module does not contain `{field}`"),
            },
        }
    }
}

impl Debug for Module {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.name)
            .field("scope", &self.inner.scope)
            .finish()
    }
}

impl Repr for Module {
    fn repr(&self) -> EcoString {
        match &self.name {
            Some(module) => eco_format!("<module {module}>"),
            None => "<module>".into(),
        }
    }
}

impl PartialEq for Module {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Arc::ptr_eq(&self.inner, &other.inner)
    }
}
