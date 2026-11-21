//! The data sidebar: the files that the session holds as tables, the sample tables, each with its
//! columns, and the functions that avenger-sql adds. With a project open, it lists the project's
//! files by role in place of the files and the samples, with the tables of each data script.
//! Clicking a name inserts it in the editor, and clicking a file opens it in a tab.

use std::collections::BTreeSet;

use eframe::egui;

use arrow::datatypes::DataType;
use avenger_sql_language::quote;

use crate::query::{Function, Functions};
use crate::samples::{Sample, SCHEMA};
use crate::sources::Source;
use crate::workspace::{Sidebar, Status};

/// What the user did in the sidebar this frame.
#[derive(Default)]
pub struct SidebarOutput {
    /// Text to insert at the editor's cursor.
    pub insert: Option<String>,
    /// Whether the user asked to add files.
    pub add_files: bool,
    /// The file of the project to open in a tab.
    pub open: Option<String>,
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

/// Shows the sidebar of the project `name`: its data scripts, each with the tables that it
/// registered and their columns, its chart scripts by folder, each with its own tables of files
/// and their columns, its stylesheets, and its modules,
/// with the file of the active tab, `active`, selected, and a spinner at each data script of
/// `loading`, and the functions.
pub fn project(
    ui: &mut egui::Ui,
    name: &str,
    sidebar: &Sidebar,
    active: Option<&str>,
    loading: &BTreeSet<String>,
    functions: &Functions,
) -> SidebarOutput {
    let mut output = SidebarOutput::default();
    ui.heading(name);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            section(ui, "Data scripts", |ui| {
                if sidebar.data.is_empty() {
                    ui.weak("Add a .sql file under data/ to load tables.");
                }
                for (path, state) in &sidebar.data {
                    ui.horizontal(|ui| {
                        file(ui, path, path, active, &mut output);
                        let failed = state.as_ref().map(|state| (state.status, state));
                        if loading.contains(path) || loading.contains("") {
                            ui.spinner();
                        } else if let Some((status, state)) =
                            failed.filter(|(status, _)| *status != Status::Loaded)
                        {
                            let what = match status {
                                Status::Failed => "The script has errors",
                                Status::NotLoaded => "The script reads a script that did not load",
                                Status::Blocked => "The script is in a cycle of scripts",
                                Status::Loaded => "",
                            };
                            let errors: Vec<&str> = (state.diagnostics.iter())
                                .map(|diagnostic| diagnostic.message.as_str())
                                .collect();
                            ui.colored_label(ui.visuals().error_fg_color, "⚠")
                                .on_hover_text(format!("{what}.\n\n{}", errors.join("\n")));
                        }
                    });
                    let tables = state.iter().flat_map(|state| &state.tables);
                    for table in tables {
                        let hover = format!("A table of {path}. Insert writes its name.");
                        project_table(ui, path, table, 12.0, &hover, &mut output);
                    }
                }
            });
            section(ui, "Charts", |ui| {
                if sidebar.charts.is_empty() {
                    ui.weak("Add a .sql file under charts/ to draw a chart.");
                }
                for (folder, paths) in &sidebar.charts {
                    if !folder.is_empty() {
                        ui.label(egui::RichText::new(format!("{folder}/")).weak().monospace());
                    }
                    for path in paths {
                        let name = path.rsplit('/').next().unwrap_or(path);
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            file(ui, path, name, active, &mut output);
                        });
                        // The chart's own tables of files, which only the chart reads.
                        for table in sidebar.chart_tables.get(path).into_iter().flatten() {
                            let hover = format!(
                                "A table of {path}'s own files, which only it reads. Insert writes its name."
                            );
                            project_table(ui, path, table, 24.0, &hover, &mut output);
                        }
                    }
                }
            });
            if !sidebar.stylesheets.is_empty() {
                section(ui, "Stylesheets", |ui| {
                    for path in &sidebar.stylesheets {
                        ui.label(egui::RichText::new(path).monospace())
                            .on_hover_text(
                            "A stylesheet of the project. The editor doesn't open stylesheets yet.",
                        );
                    }
                });
            }
            if !sidebar.modules.is_empty() {
                section(ui, "Modules", |ui| {
                    for module in &sidebar.modules {
                        ui.horizontal(|ui| {
                            file(ui, &module.path, &module.path, active, &mut output);
                            ui.weak(module.names.join("."));
                        });
                    }
                });
            }
            section(ui, "Functions", |ui| {
                group(ui, "Marks", &functions.marks, &mut output);
                group(ui, "Compositions", &functions.compositions, &mut output);
                group(ui, "Constructors", &functions.constructors, &mut output);
                group(ui, "Transforms", &functions.transforms, &mut output);
            });
        });
    output
}

/// The table `table` of the script at `path`, indented by `indent`, with its columns under its
/// name, `hover` on its name, and a button that inserts its name.
fn project_table(
    ui: &mut egui::Ui,
    path: &str,
    table: &crate::workspace::Table,
    indent: f32,
    hover: &str,
    output: &mut SidebarOutput,
) {
    let name: Vec<String> = table.names.iter().map(|name| quote(name)).collect();
    let name = name.join(".");
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            let header = egui::CollapsingHeader::new(egui::RichText::new(&name).monospace())
                .id_salt(("project table", path, &name))
                .default_open(false)
                .show(ui, |ui| columns(ui, &table.columns, output));
            header.header_response.on_hover_text(hover);
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if ui
                    .small_button("Insert")
                    .on_hover_text("Insert the table name")
                    .clicked()
                {
                    output.insert = Some(name.clone());
                }
            });
        });
    });
}

/// A file of the project, under the label `label`, which opens it on a click, selected while it
/// is the file of the active tab.
fn file(
    ui: &mut egui::Ui,
    path: &str,
    label: &str,
    active: Option<&str>,
    output: &mut SidebarOutput,
) {
    let selected = active == Some(path);
    let response = ui
        .selectable_label(selected, egui::RichText::new(label).monospace())
        .on_hover_text(format!("Open {path}"));
    if response.clicked() {
        output.open = Some(path.to_string());
    }
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
