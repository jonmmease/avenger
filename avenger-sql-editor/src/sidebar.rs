//! The data sidebar: the files that the session holds as tables, the sample tables, each with its
//! columns, and the functions that avenger-sql adds. Clicking a name inserts it in the query.

use eframe::egui;

use arrow::datatypes::DataType;
use avenger_sql_language::quote;

use crate::query::{Function, Functions};
use crate::samples::{Sample, SCHEMA};
use crate::sources::Source;

/// What the user did in the sidebar this frame.
#[derive(Default)]
pub struct SidebarOutput {
    /// Text to insert at the editor's cursor.
    pub insert: Option<String>,
    /// Whether the user asked to add files.
    pub add_files: bool,
}

/// Shows the sidebar: the files of `sources`, and `pending` files that are still reading, the
/// sample tables, which are `None` while they load, and the functions.
pub fn show(
    ui: &mut egui::Ui,
    sources: &[Source],
    pending: usize,
    samples: Option<&[Sample]>,
    functions: &Functions,
) -> SidebarOutput {
    let mut output = SidebarOutput::default();
    ui.horizontal(|ui| {
        ui.heading("Data");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Add")
                .on_hover_text("Add CSV or Parquet files. You can also drop files on the window.")
                .clicked()
            {
                output.add_files = true;
            }
        });
    });
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            section(ui, "Files", |ui| {
                if sources.is_empty() && pending == 0 {
                    ui.weak("Add a CSV or Parquet file to query it as a table.");
                }
                for source in sources {
                    let header =
                        egui::CollapsingHeader::new(egui::RichText::new(&source.name).monospace())
                            .id_salt(("source", &source.name))
                            .default_open(false)
                            .show(ui, |ui| columns(ui, &source.columns, &mut output));
                    let path = source.path.display().to_string();
                    header
                        .header_response
                        .on_hover_text(format!("{} · {path}", source.kind.label()));
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.weak(source.kind.label());
                        if ui
                            .small_button("Insert")
                            .on_hover_text("Insert the table name")
                            .clicked()
                        {
                            output.insert = Some(quote(&source.name));
                        }
                    });
                }
                if pending > 0 {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Reading files…");
                    });
                }
            });
            section(ui, "Samples", |ui| {
                let Some(samples) = samples else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Loading the samples…");
                    });
                    return;
                };
                for sample in samples {
                    let name = format!("{SCHEMA}.{}", sample.name);
                    egui::CollapsingHeader::new(egui::RichText::new(&name).monospace())
                        .id_salt(("sample", &sample.name))
                        .default_open(false)
                        .show(ui, |ui| columns(ui, &sample.columns, &mut output))
                        .header_response
                        .on_hover_text("A sample table, which the examples read");
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        if ui
                            .small_button("Insert")
                            .on_hover_text("Insert the table name")
                            .clicked()
                        {
                            output.insert = Some(name);
                        }
                    });
                }
            });
            section(ui, "Functions", |ui| {
                group(ui, "Marks", &functions.marks, &mut output);
                group(ui, "Compositions", &functions.compositions, &mut output);
                group(ui, "Constructors", &functions.constructors, &mut output);
                group(ui, "Transforms", &functions.transforms, &mut output);
            });
        });
    output
}

/// The columns of a table, each of whose names inserts itself on a click.
fn columns(ui: &mut egui::Ui, columns: &[(String, DataType)], output: &mut SidebarOutput) {
    for (name, data_type) in columns {
        ui.horizontal(|ui| {
            let response = ui
                .add(
                    egui::Label::new(egui::RichText::new(name).monospace())
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("Insert the column name");
            if response.clicked() {
                output.insert = Some(quote(name));
            }
            ui.weak(data_type.to_string());
        });
    }
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).small().strong());
    ui.separator();
    add(ui);
}

fn group(ui: &mut egui::Ui, title: &str, functions: &[Function], output: &mut SidebarOutput) {
    egui::CollapsingHeader::new(format!("{title} ({})", functions.len()))
        .id_salt(("functions", title))
        .default_open(false)
        .show(ui, |ui| {
            for function in functions {
                let label = egui::Label::new(egui::RichText::new(&function.name).monospace())
                    .sense(egui::Sense::click());
                let mut response = ui.add(label);
                if !function.doc.is_empty() {
                    response = response.on_hover_text(&function.doc);
                }
                if response.clicked() {
                    output.insert = Some(format!("{}(", function.name));
                }
            }
        });
}
