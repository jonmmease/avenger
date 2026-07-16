use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Capabilities made available while resolving language imports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCapabilities {
    pub project_root: PathBuf,
    pub allow_memory: bool,
    pub allow_std: bool,
    pub allow_filesystem: bool,
    pub allow_http: bool,
}

impl ImportCapabilities {
    pub fn in_memory(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            allow_memory: true,
            allow_std: true,
            allow_filesystem: false,
            allow_http: false,
        }
    }

    pub fn project(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            allow_memory: false,
            allow_std: true,
            allow_filesystem: true,
            allow_http: false,
        }
    }
}

/// Capabilities made available to catalog and table providers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataCapabilities {
    pub allow_filesystem: bool,
    pub allow_http: bool,
    pub allow_environment: bool,
}

/// Explicit environment lookup seam. Compilation never reads ambient process
/// environment without a provider supplied through compiler options.
pub trait EnvironmentProvider: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
}

#[derive(Clone, Debug, Default)]
pub struct EmptyEnvironmentProvider;

impl EnvironmentProvider for EmptyEnvironmentProvider {
    fn get(&self, _name: &str) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct MapEnvironmentProvider {
    values: BTreeMap<String, String>,
}

impl MapEnvironmentProvider {
    pub fn new(values: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

impl EnvironmentProvider for MapEnvironmentProvider {
    fn get(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}
