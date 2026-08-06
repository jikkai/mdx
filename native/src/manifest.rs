use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::config::{ManifestField, NativeConfig, NativeManifestConfig, SortDirection};
pub use crate::config::{Projection, ProjectionTransform};
use crate::hast::normalize_path;
use crate::{Diagnostic, DocumentRecord};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedManifest {
    pub contents: String,
    pub path: String,
}

pub fn project_document(
    config: &NativeConfig,
    collection: &str,
    key: &str,
    locale: Option<&str>,
    slug: Option<&str>,
    file: &str,
    public: &Value,
    sensitive: &Value,
    derived: &Value,
) -> Result<Value, Vec<Diagnostic>> {
    let mut context = public.as_object().cloned().unwrap_or_default();
    context.insert("collection".into(), Value::String(collection.into()));
    context.insert("derived".into(), derived.clone());
    context.insert("file".into(), Value::String(file.into()));
    context.insert("frontmatter".into(), public.clone());
    context.insert("key".into(), Value::String(key.into()));
    context.insert(
        "locale".into(),
        locale.map_or(Value::Null, |value| Value::String(value.into())),
    );
    context.insert(
        "slug".into(),
        slug.map_or(Value::Null, |value| Value::String(value.into())),
    );
    let context = Value::Object(context);
    let sensitive_names = config
        .collections
        .get(collection)
        .map_or(&[][..], |collection| collection.sensitive.as_slice());
    let mut projections = Map::new();

    for (name, manifest) in &config.manifests {
        if !manifest.collections.iter().any(|value| value == collection) {
            continue;
        }
        let mut record = Map::new();
        for (output, field) in &manifest.fields {
            let projection = match field {
                ManifestField::Path(from) => Projection {
                    default: None,
                    from: from.clone(),
                    transform: None,
                },
                ManifestField::Projection(projection) => projection.clone(),
            };
            record.insert(
                output.clone(),
                project_field(
                    output,
                    &projection,
                    &context,
                    sensitive,
                    sensitive_names,
                    file,
                    Path::new(&config.root),
                )?,
            );
        }
        projections.insert(name.clone(), Value::Object(record));
    }
    Ok(Value::Object(projections))
}

pub fn project_field(
    output: &str,
    projection: &Projection,
    public: &Value,
    sensitive: &Value,
    sensitive_names: &[String],
    file: &str,
    root: &Path,
) -> Result<Value, Vec<Diagnostic>> {
    let sensitive_path = projection
        .from
        .strip_prefix("frontmatter.")
        .unwrap_or(&projection.from);
    let sensitive_name = sensitive_path.split('.').next().unwrap_or(sensitive_path);
    let is_sensitive = sensitive_names.iter().any(|name| name == sensitive_name);
    if is_sensitive
        && !matches!(
            projection.transform,
            Some(ProjectionTransform::Exists | ProjectionTransform::Sha256)
        )
    {
        return Err(vec![Diagnostic::error(
            "AMAMO_MANIFEST_SENSITIVE_FIELD",
            Some(file),
            format!("Manifest field `{output}` cannot expose sensitive field `{sensitive_name}`"),
        )]);
    }

    let source = if is_sensitive {
        dotted(sensitive, sensitive_path)
    } else {
        dotted(public, &projection.from)
    };
    match projection.transform {
        Some(ProjectionTransform::Exists) => {
            Ok(Value::Bool(source.is_some_and(|value| !value.is_null())))
        }
        Some(ProjectionTransform::Sha256) => {
            let Some(value) = source.or(projection.default.as_ref()) else {
                return Ok(Value::Null);
            };
            let bytes = value.as_str().map_or_else(
                || serde_json::to_vec(value).expect("JSON value serializes"),
                |value| value.as_bytes().to_vec(),
            );
            Ok(Value::String(format!("{:x}", Sha256::digest(bytes))))
        }
        Some(ProjectionTransform::MediaUrl) => {
            let value = source
                .or(projection.default.as_ref())
                .unwrap_or(&Value::Null);
            media_url(value, file, root)
        }
        None => Ok(source
            .cloned()
            .or_else(|| projection.default.clone())
            .unwrap_or(Value::Null)),
    }
}

pub fn render_manifests(
    config: &NativeConfig,
    records: &[DocumentRecord],
) -> Result<Vec<RenderedManifest>, Vec<Diagnostic>> {
    config
        .manifests
        .iter()
        .map(|(name, manifest)| render_manifest(name, manifest, records))
        .collect()
}

fn render_manifest(
    name: &str,
    manifest: &NativeManifestConfig,
    records: &[DocumentRecord],
) -> Result<RenderedManifest, Vec<Diagnostic>> {
    let mut entries = records
        .iter()
        .filter_map(|record| {
            record
                .projections
                .get(name)
                .cloned()
                .map(|projection| (record.key.clone(), projection))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| compare_entries(left, right, manifest));

    let value = if let Some(key_field) = &manifest.key {
        let mut keys = HashSet::new();
        let mut object = Map::new();
        for (_, entry) in entries {
            let key = dotted(&entry, key_field)
                .and_then(scalar_key)
                .ok_or_else(|| {
                    vec![Diagnostic::error(
                        "AMAMO_MANIFEST_DUPLICATE_KEY",
                        Some(&manifest.output),
                        format!(
                            "Manifest key field `{key_field}` must be a string, number, or boolean"
                        ),
                    )]
                })?;
            if !keys.insert(key.clone()) {
                return Err(vec![Diagnostic::error(
                    "AMAMO_MANIFEST_DUPLICATE_KEY",
                    Some(&manifest.output),
                    format!("Manifest key `{key}` is duplicated"),
                )]);
            }
            object.insert(key, entry);
        }
        Value::Object(object)
    } else {
        Value::Array(entries.into_iter().map(|(_, entry)| entry).collect())
    };
    let mut contents = serde_json::to_string_pretty(&value).map_err(|error| {
        vec![Diagnostic::error(
            "AMAMO_MANIFEST_WRITE",
            Some(&manifest.output),
            error.to_string(),
        )]
    })?;
    contents.push('\n');
    Ok(RenderedManifest {
        contents,
        path: manifest.output.clone(),
    })
}

fn compare_entries(
    left: &(String, Value),
    right: &(String, Value),
    manifest: &NativeManifestConfig,
) -> Ordering {
    for sort in &manifest.sort {
        let ordering = compare_values(
            dotted(&left.1, &sort.field),
            dotted(&right.1, &sort.field),
            sort.direction.as_ref(),
        );
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.0.cmp(&right.0)
}

fn compare_values(
    left: Option<&Value>,
    right: Option<&Value>,
    direction: Option<&SortDirection>,
) -> Ordering {
    let ordering = match (left, right) {
        (None | Some(Value::Null), None | Some(Value::Null)) => Ordering::Equal,
        (None | Some(Value::Null), _) => return Ordering::Greater,
        (_, None | Some(Value::Null)) => return Ordering::Less,
        (Some(Value::String(left)), Some(Value::String(right))) => left.cmp(right),
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .partial_cmp(&right.as_f64())
            .unwrap_or(Ordering::Equal),
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left.cmp(right),
        (Some(left), Some(right)) => left.to_string().cmp(&right.to_string()),
    };
    if direction == Some(&SortDirection::Desc) {
        ordering.reverse()
    } else {
        ordering
    }
}

fn dotted<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|part| !part.is_empty())
        .try_fold(value, |current, part| current.get(part))
}

fn scalar_key(value: &Value) -> Option<String> {
    match value {
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn media_url(value: &Value, file: &str, root: &Path) -> Result<Value, Vec<Diagnostic>> {
    let Some(value) = value.as_str() else {
        return Err(vec![Diagnostic::error(
            "AMAMO_MANIFEST_MEDIA_URL",
            Some(file),
            "mediaUrl projections require a string",
        )]);
    };
    if value.starts_with(['/', '#'])
        || value.starts_with("//")
        || value.starts_with("data:")
        || value
            .split('/')
            .next()
            .is_some_and(|prefix| prefix.contains(':'))
    {
        return Ok(Value::String(value.into()));
    }
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let resolved = normalize_path(
        &Path::new(file)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path),
    );
    let root = normalize_path(root);
    let relative = resolved.strip_prefix(&root).map_err(|_| {
        vec![Diagnostic::error(
            "AMAMO_MEDIA_OUTSIDE_ROOT",
            Some(file),
            format!("Manifest media path `{value}` escapes the configured project root"),
        )]
    })?;
    Ok(Value::String(format!(
        "/{}",
        relative.to_string_lossy().replace('\\', "/")
    )))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Projection, ProjectionTransform, SortDirection, compare_values, project_field};

    #[test]
    fn descending_sort_keeps_nulls_last() {
        assert_eq!(
            compare_values(None, Some(&json!(1)), Some(&SortDirection::Desc)),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_values(Some(&json!(1)), Some(&json!(2)), Some(&SortDirection::Desc)),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn sensitive_values_only_feed_allowed_derived_fields() {
        let public = json!({ "title": "Hello" });
        let sensitive = json!({ "password": "secret" });
        let hash = project_field(
            "passwordHash",
            &Projection {
                default: None,
                from: "password".into(),
                transform: Some(ProjectionTransform::Sha256),
            },
            &public,
            &sensitive,
            &["password".into()],
            "/project/post.mdx",
            std::path::Path::new("/project"),
        )
        .unwrap();

        assert_eq!(
            hash,
            json!("2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b")
        );
        let missing_hash = project_field(
            "passwordHash",
            &Projection {
                default: None,
                from: "password".into(),
                transform: Some(ProjectionTransform::Sha256),
            },
            &public,
            &json!({}),
            &["password".into()],
            "/project/post.mdx",
            std::path::Path::new("/project"),
        )
        .unwrap();
        assert_eq!(missing_hash, serde_json::Value::Null);

        let diagnostics = project_field(
            "password",
            &Projection {
                default: None,
                from: "password".into(),
                transform: None,
            },
            &public,
            &sensitive,
            &["password".into()],
            "/project/post.mdx",
            std::path::Path::new("/project"),
        )
        .unwrap_err();
        assert_eq!(diagnostics[0].code, "AMAMO_MANIFEST_SENSITIVE_FIELD");
    }
}
