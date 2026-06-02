use std::{collections::HashSet, sync::Arc};

use datafusion::{
    arrow::{
        datatypes::{DataType, Field, FieldRef, Schema, SchemaRef},
        record_batch::RecordBatch,
    },
    logical_expr::expr::Placeholder,
    prelude::Expr,
    scalar::ScalarValue,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, SerializableDataType, SerializableExpr,
    SerializableRecordBatch, Sharing,
};

pub const STORE_METADATA_PREFIX: &str = "__avenger_store_";
pub const STORE_NAME_COLUMN: &str = "__avenger_store_name";
pub const STORE_OWNER_KEY_COLUMN: &str = "__avenger_store_owner_key";
pub const STORE_REVISION_COLUMN: &str = "__avenger_store_revision";

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoreFieldSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableDataType>")]
    pub data_type: DataType,
    pub nullable: bool,
}

impl StoreFieldSpec {
    pub fn new(name: impl Into<String>, data_type: DataType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }

    pub fn to_field_ref(&self) -> FieldRef {
        Arc::new(Field::new(
            self.name.clone(),
            self.data_type.clone(),
            self.nullable,
        ))
    }
}

impl From<FieldRef> for StoreFieldSpec {
    fn from(field: FieldRef) -> Self {
        Self {
            name: field.name().clone(),
            data_type: field.data_type().clone(),
            nullable: field.is_nullable(),
        }
    }
}

impl From<&FieldRef> for StoreFieldSpec {
    fn from(field: &FieldRef) -> Self {
        Self {
            name: field.name().clone(),
            data_type: field.data_type().clone(),
            nullable: field.is_nullable(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    pub name: String,
    pub fields: Vec<StoreFieldSpec>,
    pub initial: Option<RecordBatch>,
    pub primary_key: Vec<String>,
    pub sharing: Sharing,
}

impl Store {
    pub fn new(name: impl Into<String>, schema: SchemaRef) -> Self {
        Self {
            name: name.into(),
            fields: schema.fields().iter().map(StoreFieldSpec::from).collect(),
            initial: None,
            primary_key: Vec::new(),
            sharing: Sharing::Shared,
        }
    }

    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
            initial: None,
            primary_key: Vec::new(),
            sharing: Sharing::Shared,
        }
    }

    pub fn from_record_batch(name: impl Into<String>, batch: RecordBatch) -> Self {
        Self {
            name: name.into(),
            fields: batch
                .schema()
                .fields()
                .iter()
                .map(StoreFieldSpec::from)
                .collect(),
            initial: Some(batch),
            primary_key: Vec::new(),
            sharing: Sharing::Shared,
        }
    }

    pub fn field(mut self, name: impl Into<String>, data_type: DataType, nullable: bool) -> Self {
        self.fields
            .push(StoreFieldSpec::new(name, data_type, nullable));
        self
    }

    pub fn primary_key(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.primary_key = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn sharing(mut self, sharing: Sharing) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn compile(&self) -> Result<CompiledStoreSpec, AvengerChartError> {
        let spec = CompiledStoreSpec {
            name: self.name.clone(),
            fields: self.fields.clone(),
            initial: self.initial.clone(),
            primary_key: self.primary_key.clone(),
            sharing: self.sharing,
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledStoreSpec {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<StoreFieldSpec>,
    #[serde_as(as = "Option<FromInto<SerializableRecordBatch>>")]
    pub initial: Option<RecordBatch>,
    #[serde(default)]
    pub primary_key: Vec<String>,
    #[serde(default = "default_store_sharing")]
    pub sharing: Sharing,
}

impl CompiledStoreSpec {
    pub fn schema(&self) -> SchemaRef {
        Arc::new(Schema::new(
            self.fields
                .iter()
                .map(StoreFieldSpec::to_field_ref)
                .collect::<Vec<_>>(),
        ))
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        validate_store_name(&self.name)?;
        let mut names = std::collections::HashSet::new();
        for field in &self.fields {
            if field.name.starts_with(STORE_METADATA_PREFIX) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' field '{}' uses reserved prefix '{}'",
                    self.name, field.name, STORE_METADATA_PREFIX
                )));
            }
            if !names.insert(field.name.clone()) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' declares duplicate field '{}'",
                    self.name, field.name
                )));
            }
        }
        for key in &self.primary_key {
            let Some(field) = self.fields.iter().find(|field| field.name == *key) else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' primary key field '{}' does not exist",
                    self.name, key
                )));
            };
            if field.nullable {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' primary key field '{}' must be non-nullable",
                    self.name, key
                )));
            }
        }
        if let Some(initial) = &self.initial {
            let expected = self.schema();
            if initial.schema().fields() != expected.fields() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' initial RecordBatch schema does not match store schema",
                    self.name
                )));
            }
            let rows = self.rows_from_batch(initial)?;
            self.validate_rows(&rows)?;
        }
        Ok(())
    }

    pub fn field(&self, name: &str) -> Option<&StoreFieldSpec> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub fn initial_rows(&self) -> Result<Vec<StoreRowValue>, AvengerChartError> {
        match &self.initial {
            Some(initial) => self.rows_from_batch(initial),
            None => Ok(Vec::new()),
        }
    }

    pub fn validate_rows(&self, rows: &[StoreRowValue]) -> Result<(), AvengerChartError> {
        if self.primary_key.is_empty() {
            return Ok(());
        }

        let mut keys: HashSet<Vec<ScalarValue>> = HashSet::new();
        for row in rows {
            let key = self.primary_key_values(row)?;
            if !keys.insert(key) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' contains duplicate primary-key rows",
                    self.name
                )));
            }
        }
        Ok(())
    }

    pub fn primary_key_values(
        &self,
        row: &StoreRowValue,
    ) -> Result<Vec<ScalarValue>, AvengerChartError> {
        let mut key = Vec::with_capacity(self.primary_key.len());
        for field in &self.primary_key {
            let Some(value) = row.get(field) else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' row is missing primary-key field '{}'",
                    self.name, field
                )));
            };
            if value.is_null() {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Store '{}' row has null primary-key field '{}'",
                    self.name, field
                )));
            }
            key.push(value.clone());
        }
        Ok(key)
    }

    fn rows_from_batch(
        &self,
        batch: &RecordBatch,
    ) -> Result<Vec<StoreRowValue>, AvengerChartError> {
        let mut rows = Vec::with_capacity(batch.num_rows());
        for row_index in 0..batch.num_rows() {
            let mut row = StoreRowValue::new();
            for (field_index, field) in self.fields.iter().enumerate() {
                let value = ScalarValue::try_from_array(batch.column(field_index), row_index)
                    .map_err(|err| {
                        AvengerChartError::InvalidArgument(format!(
                            "Store '{}' initial RecordBatch value for field '{}' could not be converted to ScalarValue: {err}",
                            self.name, field.name
                        ))
                    })?;
                row.insert(field.name.clone(), value);
            }
            rows.push(row);
        }
        Ok(rows)
    }
}

fn default_store_sharing() -> Sharing {
    Sharing::Shared
}

pub fn validate_store_name(name: &str) -> Result<(), AvengerChartError> {
    if name.is_empty()
        || name.contains('.')
        || name.starts_with(STORE_METADATA_PREFIX)
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Invalid store name '{name}'; names must be non-empty ASCII identifiers without periods"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreDataScope {
    #[default]
    CurrentOwner,
    Root,
    AllOwners,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreData {
    pub store_name: String,
    #[serde(default)]
    pub scope: StoreDataScope,
}

impl StoreData {
    pub fn new(store_name: impl Into<String>) -> Self {
        Self {
            store_name: store_name.into(),
            scope: StoreDataScope::CurrentOwner,
        }
    }

    pub fn current_scope(mut self) -> Self {
        self.scope = StoreDataScope::CurrentOwner;
        self
    }

    pub fn root(mut self) -> Self {
        self.scope = StoreDataScope::Root;
        self
    }

    pub fn all_scopes(mut self) -> Self {
        self.scope = StoreDataScope::AllOwners;
        self
    }

    pub fn field(&self, field: impl Into<String>) -> StoreFieldRef {
        StoreFieldRef {
            store_name: self.store_name.clone(),
            scope: self.scope,
            field: field.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreFieldRef {
    pub store_name: String,
    pub scope: StoreDataScope,
    pub field: String,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoreValueExpr {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

impl StoreValueExpr {
    pub fn new(expr: impl IntoExpr) -> Self {
        Self {
            expr: LogicalExprNode::from_expr(expr.into_expr())
                .expect("serialize store value expression"),
        }
    }

    pub fn to_expr(&self) -> Result<Expr, AvengerChartError> {
        self.expr
            .to_expr(&datafusion::prelude::SessionContext::new())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreRow {
    #[serde(default)]
    pub fields: IndexMap<String, StoreValueExpr>,
}

impl StoreRow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.fields.insert(name.into(), StoreValueExpr::new(expr));
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreFieldPatch {
    #[serde(default)]
    pub fields: IndexMap<String, StoreValueExpr>,
}

impl StoreFieldPatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.fields.insert(name.into(), StoreValueExpr::new(expr));
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreKey {
    #[serde(default)]
    pub fields: IndexMap<String, StoreValueExpr>,
}

impl StoreKey {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.fields.insert(name.into(), StoreValueExpr::new(expr));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StoreUpdate {
    Clear,
    ReplaceRows {
        rows: Vec<StoreRow>,
    },
    InsertRows {
        rows: Vec<StoreRow>,
    },
    UpsertRows {
        rows: Vec<StoreRow>,
    },
    UpdateByKey {
        key: StoreKey,
        fields: StoreFieldPatch,
    },
    DeleteByKey {
        key: StoreKey,
    },
    ToggleRows {
        rows: Vec<StoreRow>,
    },
}

impl StoreUpdate {
    pub fn clear() -> Self {
        Self::Clear
    }

    pub fn replace_rows(rows: impl IntoIterator<Item = StoreRow>) -> Self {
        Self::ReplaceRows {
            rows: rows.into_iter().collect(),
        }
    }

    pub fn insert_rows(rows: impl IntoIterator<Item = StoreRow>) -> Self {
        Self::InsertRows {
            rows: rows.into_iter().collect(),
        }
    }

    pub fn upsert_rows(rows: impl IntoIterator<Item = StoreRow>) -> Self {
        Self::UpsertRows {
            rows: rows.into_iter().collect(),
        }
    }

    pub fn update_by_key(key: StoreKey, fields: StoreFieldPatch) -> Self {
        Self::UpdateByKey { key, fields }
    }

    pub fn delete_by_key(key: StoreKey) -> Self {
        Self::DeleteByKey { key }
    }

    pub fn toggle_rows(rows: impl IntoIterator<Item = StoreRow>) -> Self {
        Self::ToggleRows {
            rows: rows.into_iter().collect(),
        }
    }
}

pub fn store_placeholder_expr(store_name: impl AsRef<str>) -> Expr {
    Expr::Placeholder(Placeholder {
        id: format!("$__store_{}", store_name.as_ref()),
        data_type: Some(DataType::Utf8),
    })
}

pub type StoreRowValue = IndexMap<String, ScalarValue>;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::{
        array::{Int32Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use datafusion::prelude::lit;

    use super::*;

    #[test]
    fn store_spec_roundtrips() {
        let store = Store::empty("selected_ids")
            .field("id", DataType::Utf8, false)
            .primary_key(["id"])
            .sharing(Sharing::Free);
        let spec = store.compile().unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        let restored: CompiledStoreSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "selected_ids");
        assert_eq!(restored.primary_key, vec!["id"]);
        assert_eq!(restored.sharing, Sharing::Free);
    }

    #[test]
    fn store_initial_batch_roundtrips() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("value", DataType::Int32, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["a", "b"])),
                Arc::new(Int32Array::from(vec![Some(1), None])),
            ],
        )
        .unwrap();
        let spec = Store::from_record_batch("items", batch)
            .primary_key(["id"])
            .compile()
            .unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        let restored: CompiledStoreSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.initial.unwrap().num_rows(), 2);
    }

    #[test]
    fn duplicate_initial_primary_key_errors() {
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(vec!["a", "a"]))])
            .unwrap();
        let err = Store::from_record_batch("items", batch)
            .primary_key(["id"])
            .compile()
            .unwrap_err();
        assert!(format!("{err:?}").contains("duplicate primary-key"));
    }

    #[test]
    fn reserved_store_field_errors() {
        let err = Store::empty("bad")
            .field(STORE_NAME_COLUMN, DataType::Utf8, false)
            .compile()
            .unwrap_err();
        assert!(format!("{err:?}").contains("reserved prefix"));
    }

    #[test]
    fn store_update_serializes() {
        let update = StoreUpdate::upsert_rows([StoreRow::new().field("id", lit("a"))]);
        let json = serde_json::to_string(&update).unwrap();
        let restored: StoreUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, update);
    }
}
