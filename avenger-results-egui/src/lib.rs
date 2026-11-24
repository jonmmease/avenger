//! The result of a chart script in egui, as the window of `avenger view` shows it: one chart
//! that fills its view, or the table of the result, whose chart cells open their charts over the
//! table.
//! [`ResultView`] is the view, which a host shows with what its charts share, [`Shared`]. A table
//! without charts can show from a [`RowSource`], which reads its rows a page at a time.

pub mod charts;
pub mod exec;
mod gpu;
mod input;
mod live;
pub mod paged;
mod state;
mod table;
mod view;

pub use charts::{Images, StaticChart, Surface};
pub use exec::{Exec, Task};
pub use paged::{PagedTable, RowSource, RowStream, PAGE_ROWS};
pub use state::{describe_change, plot_name, ChartState, ScaleState};
pub use view::{ResultView, Shared};
