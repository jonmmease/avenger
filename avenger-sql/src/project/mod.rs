//! The data scripts of a project, together: the order that they run in, and the catalog of the
//! tables that they define.
//!
//! A project's data scripts register its tables, and a data script can read the tables of
//! another. [`Graph`] reads, from the parses of the scripts, the names that each defines and
//! reads, and gives the order that they run in, the errors of their names, such as a cycle, and
//! the scripts that a change of one reloads. [`ProjectCatalog`] holds the tables, views, and
//! schemas that the scripts' analyses define, with the place of each, over a base catalog, as
//! the analysis of each script after them, and of the chart scripts, reads them. Both read no
//! files: a loader reads the files, and gives the catalog their tables.

mod catalog;
mod graph;

pub use catalog::ProjectCatalog;
pub use graph::{
    script_names, unavailable_read, Edge, Graph, GraphError, Name, NameKind, Place, ScriptNames,
};
