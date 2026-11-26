//! `SqlProject` in JavaScript: a project of files in memory, its charts as the bytes of chart
//! messages, which avenger-chart-web's views draw, and the features of an editor as plain objects.

use std::rc::Rc;

use js_sys::{Array, Object, Promise, Uint8Array};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::project;

/// A project of avenger-sql whose files the page holds.
#[wasm_bindgen(js_name = SqlProject)]
pub struct SqlProject {
    inner: Rc<project::SqlProject>,
}

fn value(value: &impl Serialize) -> Result<JsValue, JsError> {
    let serializer = serde_wasm_bindgen::Serializer::json_compatible();
    value
        .serialize(&serializer)
        .map_err(|error| JsError::new(&error.to_string()))
}

#[wasm_bindgen(js_class = SqlProject)]
impl SqlProject {
    /// Opens the project of `files`, an object of each file's text or bytes by its path relative
    /// to the project's root, such as `{ "avenger.toml": "...", "data/files/sales.csv": bytes }`,
    /// in a store named `name`, and loads its data scripts. Resolves to the project.
    pub fn open(name: String, files: Object) -> Result<Promise, JsError> {
        let mut entries = Vec::new();
        for entry in Object::entries(&files).iter() {
            let entry = Array::from(&entry);
            let path = (entry.get(0).as_string())
                .ok_or_else(|| JsError::new("a file's path is a string"))?;
            let contents = entry.get(1);
            let bytes = match contents.as_string() {
                Some(text) => text.into_bytes(),
                None => Uint8Array::new(&contents).to_vec(),
            };
            entries.push((path, bytes));
        }
        Ok(future_to_promise(async move {
            let project = (project::SqlProject::open(&name, entries).await)
                .map_err(|error| JsError::new(&error))?;
            Ok(SqlProject {
                inner: Rc::new(project),
            }
            .into())
        }))
    }

    /// The paths of the chart scripts.
    #[wasm_bindgen(js_name = chartScripts)]
    pub fn chart_scripts(&self) -> Vec<String> {
        self.inner.chart_scripts().to_vec()
    }

    /// The text of the file at `path`, or `undefined`.
    pub fn text(&self, path: &str) -> Option<String> {
        self.inner.text(path)
    }

    /// Resolves to the bytes of each chart of the chart script at `path`, by row, or rejects with
    /// the script's error.
    pub fn charts(&self, path: String) -> Promise {
        let inner = self.inner.clone();
        future_to_promise(async move {
            let charts = (inner.charts(&path).await).map_err(|error| JsError::new(&error))?;
            let array = Array::new();
            for chart in charts {
                array.push(&Uint8Array::from(chart.as_slice()));
            }
            Ok(array.into())
        })
    }

    /// Sets the text of the file at `path`, as an editor changes it, and resolves when a data
    /// script or a module that it changes has loaded again.
    #[wasm_bindgen(js_name = setText)]
    pub fn set_text(&self, path: String, text: String) -> Promise {
        let inner = self.inner.clone();
        future_to_promise(async move {
            (inner.set_text(&path, text).await).map_err(|error| JsError::new(&error))?;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// The errors and the warnings of the file at `path`: objects with `range`, `severity`,
    /// `message`, `help`, `source`, and `fixes`. Ranges count UTF-16 code units, as strings do.
    pub fn diagnostics(&self, path: &str) -> Result<JsValue, JsError> {
        value(&self.inner.diagnostics(path))
    }

    /// The completions at the position `at` of the file at `path`: `{ range, items }`, or
    /// `null`.
    pub fn completions(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&self.inner.completions(path, at))
    }

    /// The hover at the position `at` of the file at `path`: `{ range, markdown }`, or `null`.
    pub fn hover(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&self.inner.hover(path, at))
    }

    /// The forms of the call at the position `at` of the file at `path`, or `null`.
    #[wasm_bindgen(js_name = signatureHelp)]
    pub fn signature_help(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&self.inner.signature_help(path, at))
    }

    /// The outline of the file at `path`.
    pub fn symbols(&self, path: &str) -> Result<JsValue, JsError> {
        value(&self.inner.symbols(path))
    }

    /// The classes of the tokens of the file at `path`: objects with `range`, `kind`, such as
    /// `Keyword` or `Name(Column)`, and `definition`.
    pub fn highlights(&self, path: &str) -> Result<JsValue, JsError> {
        value(&self.inner.highlights(path))
    }

    /// The definition of the name at the position `at` of the file at `path`: `{ origin, path,
    /// target }`, or `null`.
    pub fn definition(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&self.inner.definition(path, at))
    }

    /// The uses and the definitions of the name at the position `at` of the file at `path`.
    pub fn references(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&self.inner.references(path, at))
    }

    /// The range of the name at the position `at` of the file at `path` that a rename renames.
    /// Throws the reason a rename can't rename it.
    #[wasm_bindgen(js_name = prepareRename)]
    pub fn prepare_rename(&self, path: &str, at: u32) -> Result<JsValue, JsError> {
        value(&(self.inner.prepare_rename(path, at)).map_err(|error| JsError::new(&error))?)
    }

    /// The edits of each file that rename the name at the position `at` of the file at `path` to
    /// `name`: objects with `path` and `edits`. Throws the reason a rename can't rename it.
    pub fn rename(&self, path: &str, at: u32, name: &str) -> Result<JsValue, JsError> {
        value(&(self.inner.rename(path, at, name)).map_err(|error| JsError::new(&error))?)
    }

    /// The edits that format the file at `path`: objects with `range` and `text`.
    pub fn formatting(&self, path: &str) -> Result<JsValue, JsError> {
        value(&self.inner.formatting(path))
    }

    /// The ranges of the file at `path` that an editor folds: objects with `range` and `comment`.
    #[wasm_bindgen(js_name = foldingRanges)]
    pub fn folding_ranges(&self, path: &str) -> Result<JsValue, JsError> {
        value(&self.inner.folding_ranges(path))
    }
}
