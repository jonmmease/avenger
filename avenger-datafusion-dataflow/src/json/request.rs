use super::{invalid, sources::AssetBindings, spec::*, values};
use crate::{
    DataflowResult, PreparedDataflow, Result, ScopeInstance, ScopeInterface, ScopedBindingsBuilder,
    TableSnapshot,
};
use datafusion::arrow::datatypes::SchemaRef;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
fn scope(root: &ScopeInterface, path: &[String]) -> Result<ScopeInterface> {
    let mut current = root.clone();
    for name in path {
        current = current.scope(name)?;
    }
    Ok(current)
}
fn table(
    binding: &TableBinding,
    schema: SchemaRef,
    assets: &AssetBindings,
) -> Result<TableSnapshot> {
    match binding {
        TableBinding::Inline(rows) => values::table(schema, &rows.values),
        TableBinding::Asset(a) => assets
            .get(&a.asset)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown request asset {}", a.asset))),
    }
}
fn scoped(
    mut b: ScopedBindingsBuilder,
    names: &ScopeInterface,
    scalars: &BTreeMap<String, Value>,
    tables: &BTreeMap<String, TableBinding>,
    assets: &AssetBindings,
) -> Result<ScopedBindingsBuilder> {
    for (name, value) in scalars {
        let input = names.scalar_input(name)?;
        b = b.scalar(&input, values::scalar(value, input.field().data_type())?)?;
    }
    for (name, value) in tables {
        let input = names.table_input(name)?;
        b = b.table(
            &input,
            table(
                value,
                std::sync::Arc::new(input.schema().as_arrow().clone()),
                assets,
            )?,
        )?;
    }
    Ok(b)
}
impl PreparedDataflow {
    /// Bind an independent request by public names and execute through the typed query API.
    pub async fn query_request(
        &self,
        request: &QueryRequest,
        assets: &AssetBindings,
    ) -> Result<DataflowResult> {
        let root = self.interface().root();
        let mut inputs = self.inputs();
        for (name, value) in &request.bindings.scalars {
            let input = root.scalar_input(name)?;
            inputs = inputs.scalar(&input, values::scalar(value, input.field().data_type())?)?;
        }
        for (name, value) in &request.bindings.tables {
            let input = root.table_input(name)?;
            inputs = inputs.table(
                &input,
                table(
                    value,
                    std::sync::Arc::new(input.schema().as_arrow().clone()),
                    assets,
                )?,
            )?;
        }
        let mut defaults = HashSet::new();
        for binding in &request.bindings.scope_defaults {
            if !defaults.insert(&binding.scope) {
                return Err(invalid("duplicate scope_defaults address"));
            }
            let names = scope(&root, &binding.scope)?;
            let handle = names
                .handle()
                .ok_or_else(|| invalid("scope_defaults requires a child scope"))?;
            inputs = inputs.scope_defaults(handle, |b| {
                scoped(b, &names, &binding.scalars, &binding.tables, assets)
            })?;
        }
        let mut overrides = HashSet::new();
        for binding in &request.bindings.overrides {
            let mut names = root.clone();
            let mut instance: Option<ScopeInstance> = None;
            for address in &binding.path {
                names = names.scope(&address.scope)?;
                let handle = names.handle().expect("child scope");
                if address.key.len() != handle.key_schema().fields().len() {
                    return Err(invalid("partition key width mismatch"));
                }
                let key = address
                    .key
                    .iter()
                    .zip(handle.key_schema().fields())
                    .map(|(v, f)| values::scalar(v, f.data_type()))
                    .collect::<Result<Vec<_>>>()?;
                instance = Some(match instance {
                    None => handle.instance(key)?,
                    Some(parent) => parent.child(handle, key)?,
                });
            }
            let instance = instance.ok_or_else(|| invalid("override path cannot be empty"))?;
            if !overrides.insert(instance.clone()) {
                return Err(invalid("duplicate override address"));
            }
            inputs = inputs.at(&instance, |b| {
                scoped(b, &names, &binding.scalars, &binding.tables, assets)
            })?;
        }
        let inputs = inputs.finish()?;
        let tables = request
            .outputs
            .tables
            .iter()
            .map(|o| scope(&root, &o.scope)?.table_output(&o.output))
            .collect::<Result<Vec<_>>>()?;
        let scalars = request
            .outputs
            .scalars
            .iter()
            .map(|o| scope(&root, &o.scope)?.scalar_output(&o.output))
            .collect::<Result<Vec<_>>>()?;
        self.query(&tables, &scalars, &inputs).await
    }
}
