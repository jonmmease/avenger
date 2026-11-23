//! The result of a chart script in egui, as the avenger-sql editor shows it: one chart that fills
//! its view, or the table of the result, whose chart cells open their charts over the table.
//! [`ResultView`] is the view, which a host shows with what its charts share, [`Shared`].

pub mod charts;
pub mod exec;
mod gpu;
mod input;
mod live;
mod table;
mod view;

pub use charts::{Images, StaticChart, Surface};
pub use exec::{Exec, Task};
pub use view::{ResultView, Shared};
