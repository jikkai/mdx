use std::collections::HashMap;

use jsonschema::Draft;
use serde::Deserialize;
use serde_json::Value;

use crate::Diagnostic;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeCollectionConfig {
    pub schema: Value,
    #[serde(default)]
    pub sensitive: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeConfig {
    pub cache: NativeCacheConfig,
    pub collections: HashMap<String, NativeCollectionConfig>,
    pub derived: NativeDerivedConfig,
    pub highlight: NativeHighlightConfig,
    pub manifests: HashMap<String, NativeManifestConfig>,
    #[serde(default)]
    pub math: NativeMathConfig,
    pub media: NativeMediaConfig,
    pub mdx: NativeMdxConfig,
    pub root: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NativeCacheConfig {
    pub directory: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeManifestConfig {
    pub collections: Vec<String>,
    pub fields: HashMap<String, ManifestField>,
    pub key: Option<String>,
    pub output: String,
    #[serde(default)]
    pub sort: Vec<ManifestSort>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ManifestField {
    Path(String),
    Projection(Projection),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    pub default: Option<Value>,
    pub from: String,
    pub transform: Option<ProjectionTransform>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum ProjectionTransform {
    #[serde(rename = "exists")]
    Exists,
    #[serde(rename = "mediaUrl")]
    MediaUrl,
    #[serde(rename = "sha256")]
    Sha256,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestSort {
    pub direction: Option<SortDirection>,
    pub field: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMdxConfig {
    pub gfm: bool,
    pub hard_breaks: bool,
    pub jsx_import_source: String,
    pub provider_import_source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMathConfig {
    pub enabled: bool,
    #[serde(default)]
    pub macros: HashMap<String, String>,
    pub single_dollar: bool,
}

impl Default for NativeMathConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            macros: HashMap::new(),
            single_dollar: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDerivedConfig {
    pub last_modified: bool,
    pub reading_time: bool,
}

#[derive(Debug, Deserialize)]
pub struct NativeHighlightConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaMissing {
    Error,
    Warn,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NativeMediaConfig {
    pub attributes: HashMap<String, Vec<String>>,
    pub enabled: bool,
    pub missing: MediaMissing,
}

impl Default for NativeMediaConfig {
    fn default() -> Self {
        Self {
            attributes: HashMap::from([
                ("audio".into(), vec!["src".into()]),
                ("embed".into(), vec!["src".into()]),
                ("img".into(), vec!["src".into(), "srcset".into()]),
                ("object".into(), vec!["data".into()]),
                ("source".into(), vec!["src".into(), "srcset".into()]),
                ("track".into(), vec!["src".into()]),
                ("video".into(), vec!["src".into(), "poster".into()]),
            ]),
            enabled: true,
            missing: MediaMissing::Error,
        }
    }
}

pub fn apply_schema_defaults_and_validate(
    file: &str,
    value: &mut Value,
    config: &NativeCollectionConfig,
) -> Result<(), Vec<Diagnostic>> {
    apply_defaults(value, &config.schema);
    let validator = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(&config.schema)
        .map_err(|error| {
            vec![Diagnostic::error(
                "AMAMO_SCHEMA_INVALID",
                Some(file),
                format!("Invalid JSON Schema: {error}"),
            )]
        })?;
    let diagnostics = validator
        .iter_errors(value)
        .map(|error| {
            Diagnostic::error(
                "AMAMO_SCHEMA_INVALID",
                Some(file),
                format!("{} at {}", error, error.instance_path()),
            )
        })
        .collect::<Vec<_>>();

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn apply_defaults(instance: &mut Value, schema: &Value) {
    if let (Some(object), Some(properties)) = (
        instance.as_object_mut(),
        schema.get("properties").and_then(Value::as_object),
    ) {
        for (name, property_schema) in properties {
            if !object.contains_key(name)
                && let Some(default) = property_schema.get("default")
            {
                object.insert(name.clone(), default.clone());
            }
            if let Some(value) = object.get_mut(name) {
                apply_defaults(value, property_schema);
            }
        }
    }

    if let (Some(items), Some(item_schema)) = (instance.as_array_mut(), schema.get("items")) {
        for item in items {
            apply_defaults(item, item_schema);
        }
    }

    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for child_schema in all_of {
            apply_defaults(instance, child_schema);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{NativeCollectionConfig, apply_schema_defaults_and_validate};

    fn collection(schema: serde_json::Value) -> NativeCollectionConfig {
        NativeCollectionConfig {
            schema,
            sensitive: vec!["password".into()],
        }
    }

    #[test]
    fn applies_declared_defaults_without_coercion() {
        let config = collection(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "draft": { "type": "boolean", "default": false }
            },
            "required": ["title"]
        }));
        let mut value = json!({ "title": "Hello" });

        apply_schema_defaults_and_validate("/project/post.mdx", &mut value, &config).unwrap();

        assert_eq!(value["draft"], false);
    }

    #[test]
    fn rejects_a_schema_type_mismatch() {
        let config = collection(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "draft": { "type": "boolean" } }
        }));
        let mut value = json!({ "draft": "false" });

        let diagnostics =
            apply_schema_defaults_and_validate("/project/post.mdx", &mut value, &config)
                .unwrap_err();

        assert_eq!(diagnostics[0].code, "AMAMO_SCHEMA_INVALID");
        assert_eq!(diagnostics[0].file.as_deref(), Some("/project/post.mdx"));
    }
}
