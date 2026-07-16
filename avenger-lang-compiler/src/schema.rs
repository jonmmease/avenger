use std::sync::Arc;

use avenger_chart_lang_registry::{NativeRegistry, RegistryError, ResolvedDeclaration};
use avenger_chart_schema::{NativeKindKey, NativeSchemaSnapshot};
use serde_json::{Value, json};

/// One immutable host registry shared by validation, semantic schema
/// generation, lowering, and compiler artifacts.
#[derive(Clone)]
pub struct LanguageHost {
    registry: Arc<NativeRegistry>,
}

impl LanguageHost {
    pub fn new(registry: Arc<NativeRegistry>) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &Arc<NativeRegistry> {
        &self.registry
    }

    pub fn authoring_schema(&self) -> &NativeSchemaSnapshot {
        self.registry.snapshot()
    }

    pub fn validate_native_declaration(
        &self,
        key: &NativeKindKey,
        declaration: &ResolvedDeclaration,
    ) -> Result<(), RegistryError> {
        self.registry.validate(key, declaration)
    }

    pub fn semantic_json_schema(&self) -> SemanticJsonSchema {
        SemanticJsonSchema(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://avenger.dev/schema/semantic/v1.json",
            "title": "Avenger semantic AST",
            "type": "object",
            "x-avenger-language-major": self.authoring_schema().version.major,
            "x-avenger-native-profile": self.registry.profile_id().as_str(),
            "x-avenger-native-schema": self.authoring_schema(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticJsonSchema(Value);

impl SemanticJsonSchema {
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn into_value(self) -> Value {
        self.0
    }
}
