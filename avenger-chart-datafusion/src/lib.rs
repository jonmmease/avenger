//! The DataFusion side of avenger's charts, which runs natively and in WebAssembly pages: the
//! functions that build charts in plans, and the codec of the logical plans that call them.

pub mod codec;
pub mod functions;
pub mod proto {
    //! The prost messages of package `avenger.chart.datafusion.v1`, what the codec writes into
    //! DataFusion's messages of a plan, generated from
    //! `proto/avenger/chart/datafusion/v1/plan.proto`, whose comments are their documentation.
    include!(concat!(env!("OUT_DIR"), "/avenger.chart.datafusion.v1.rs"));
}
