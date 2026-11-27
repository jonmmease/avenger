//! The DataFusion side of avenger's charts, which runs natively and in WebAssembly pages: the
//! functions that build charts in plans, and the codec of the logical plans that call them.

pub mod codec;
pub mod functions;
