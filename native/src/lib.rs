mod cache;
mod config;
mod document;
mod hast;
mod manifest;
mod math;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use napi::bindgen_prelude::{Error, Result, Status};
use napi_derive::napi;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::{
    key as cache_key, prune as prune_cache_entries, read as read_cache, remove as remove_cache,
    write as write_cache,
};
use crate::config::{NativeCacheConfig, NativeConfig, NativeMdxConfig};
use crate::document::{ParsedFrontmatter, PreparedMdx, finish_mdx, parse_frontmatter, prepare_mdx};
use crate::hast::{decode_highlights, inject_highlights};
use crate::manifest::{project_document, render_manifests as render_manifest_outputs};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePoint {
    pub line: u32,
    pub column: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    pub start: SourcePoint,
    pub end: SourcePoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub file: Option<String>,
    pub range: Option<SourceRange>,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, file: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: Severity::Error,
            file: file.map(str::to_owned),
            range: None,
            message: message.into(),
            hint: None,
        }
    }

    pub fn warning(code: &str, file: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: Severity::Warning,
            file: file.map(str::to_owned),
            range: None,
            message: message.into(),
            hint: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentInput {
    collection: String,
    key: String,
    file: String,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    source: String,
    modified_at: Option<String>,
}

struct PendingDocument {
    cache_key: String,
    cache_warning: Option<Diagnostic>,
    derived: Value,
    input: DocumentInput,
    parsed: ParsedFrontmatter,
    prepared: PreparedMdx,
    projections: Value,
}

enum PreparedDocument {
    Cached(Box<DocumentRecord>),
    Pending(Box<PendingDocument>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentRecord {
    cached: bool,
    collection: String,
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    locale: Option<String>,
    file: String,
    hash: String,
    cache_key: String,
    frontmatter: Value,
    derived: Value,
    dependencies: Vec<String>,
    projections: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    module: String,
    diagnostics: Vec<Diagnostic>,
}

#[napi]
pub struct PreparedBatch {
    cache: NativeCacheConfig,
    documents: Vec<PreparedDocument>,
    mdx: NativeMdxConfig,
}

#[napi]
impl PreparedBatch {
    #[napi(getter)]
    pub fn code_blocks_json(&self) -> String {
        let blocks = self
            .documents
            .iter()
            .filter_map(|document| match document {
                PreparedDocument::Cached(_) => None,
                PreparedDocument::Pending(document) => Some(&document.prepared.code_blocks),
            })
            .flatten()
            .collect::<Vec<_>>();
        serde_json::to_string(&blocks).unwrap_or_else(|_| "[]".into())
    }

    #[napi]
    pub fn finish(&mut self, highlights_json: String) -> Result<String> {
        let highlights = decode_highlights(&highlights_json).map_err(diagnostic_error)?;
        let expected = self
            .documents
            .iter()
            .filter_map(|document| match document {
                PreparedDocument::Cached(_) => None,
                PreparedDocument::Pending(document) => Some(&document.prepared.code_blocks),
            })
            .flatten()
            .map(|block| (block.document_id.clone(), block.block_id))
            .collect::<HashSet<_>>();
        let mut replacements = HashMap::with_capacity(highlights.len());
        for highlight in highlights {
            let key = (highlight.document_id, highlight.block_id);
            if !expected.contains(&key) || replacements.insert(key, highlight.node).is_some() {
                return Err(diagnostic_error(vec![Diagnostic::error(
                    "AMAMO_SHIKI_INVALID_HAST",
                    None,
                    "Highlighted HAST contains an unknown or duplicate code block",
                )]));
            }
        }
        if replacements.len() != expected.len() {
            return Err(diagnostic_error(vec![Diagnostic::error(
                "AMAMO_SHIKI_INVALID_HAST",
                None,
                "Highlighted HAST is missing one or more code blocks",
            )]));
        }

        let mdx = self.mdx.clone();
        let cache = self.cache.clone();
        let records = std::mem::take(&mut self.documents)
            .into_iter()
            .map(
                |document| -> std::result::Result<DocumentRecord, Vec<Diagnostic>> {
                    let mut document = match document {
                        PreparedDocument::Cached(record) => return Ok(*record),
                        PreparedDocument::Pending(document) => document,
                    };
                    let document_id =
                        format!("{}/{}", document.input.collection, document.input.key);
                    let local_replacements = document
                        .prepared
                        .code_blocks
                        .iter()
                        .filter_map(|block| {
                            replacements
                                .remove(&(document_id.clone(), block.block_id))
                                .map(|node| (block.block_id, node))
                        })
                        .collect::<HashMap<_, _>>();
                    let injected =
                        inject_highlights(&mut document.prepared.tree, &local_replacements);
                    if injected != local_replacements.len() {
                        return Err(vec![Diagnostic::error(
                            "AMAMO_SHIKI_INVALID_HAST",
                            Some(&document.input.file),
                            "Highlighted HAST does not match the document code blocks",
                        )]);
                    }
                    let hash = blake3::hash(document.input.source.as_bytes())
                        .to_hex()
                        .to_string();
                    let dependencies = document
                        .prepared
                        .dependencies
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    let diagnostics = std::mem::take(&mut document.prepared.diagnostics);
                    let module = finish_mdx(
                        document.prepared,
                        &document.input.file,
                        &document.parsed.body,
                        &mdx,
                        &document.parsed.frontmatter,
                        &document.derived,
                    )?;
                    let mut record = DocumentRecord {
                        cached: false,
                        collection: document.input.collection,
                        key: document.input.key,
                        locale: document.input.locale,
                        file: document.input.file,
                        cache_key: document.cache_key.clone(),
                        hash,
                        frontmatter: document.parsed.frontmatter,
                        derived: document.derived,
                        dependencies,
                        projections: document.projections,
                        slug: document.input.slug,
                        module,
                        diagnostics,
                    };
                    if cache.enabled {
                        let value = serde_json::to_value(&record).map_err(|error| {
                            vec![Diagnostic::error(
                                "AMAMO_CACHE_WRITE",
                                Some(&record.file),
                                format!("Could not encode cache record: {error}"),
                            )]
                        })?;
                        write_cache(Path::new(&cache.directory), &document.cache_key, &value)?;
                    }
                    if let Some(warning) = document.cache_warning {
                        record.diagnostics.push(warning);
                    }
                    Ok(record)
                },
            )
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(diagnostic_error)?;

        serde_json::to_string(&records).map_err(|error| {
            Error::new(
                Status::GenericFailure,
                format!("AMAMO_NATIVE_SERIALIZE: {error}"),
            )
        })
    }
}

#[napi]
pub fn prepare_batch(config_json: String, inputs_json: String) -> Result<PreparedBatch> {
    let config: NativeConfig = decode_json(&config_json, "configuration")?;
    let inputs: Vec<DocumentInput> = decode_json(&inputs_json, "document inputs")?;
    let mut documents = Vec::with_capacity(inputs.len());

    for input in inputs {
        let entry_key = cache_key(
            &config_json,
            &input.file,
            &input.source,
            input.modified_at.as_deref(),
            "module",
        );
        let mut cache_warning = None;
        if config.cache.enabled {
            let cache_read = read_cache(Path::new(&config.cache.directory), &entry_key)
                .map_err(diagnostic_error)?;
            cache_warning = cache_read.diagnostic;
            if let Some(value) = cache_read.value {
                match serde_json::from_value::<DocumentRecord>(value) {
                    Ok(mut record)
                        if record.cache_key == entry_key
                            && record.file == input.file
                            && record
                                .dependencies
                                .iter()
                                .all(|path| Path::new(path).exists()) =>
                    {
                        record.cached = true;
                        documents.push(PreparedDocument::Cached(Box::new(record)));
                        continue;
                    }
                    Ok(_) => {
                        remove_cache(Path::new(&config.cache.directory), &entry_key)
                            .map_err(diagnostic_error)?;
                    }
                    Err(error) => {
                        remove_cache(Path::new(&config.cache.directory), &entry_key)
                            .map_err(diagnostic_error)?;
                        cache_warning = Some(Diagnostic::warning(
                            "AMAMO_CACHE_CORRUPT",
                            Some(&input.file),
                            format!("Removed incompatible cache record: {error}"),
                        ));
                    }
                }
            }
        }
        let collection = config.collections.get(&input.collection).ok_or_else(|| {
            diagnostic_error(vec![Diagnostic::error(
                "AMAMO_CONFIG_INVALID",
                Some(&input.file),
                format!("Unknown collection `{}`", input.collection),
            )])
        })?;
        let parsed =
            parse_frontmatter(&input.file, &input.source, collection).map_err(diagnostic_error)?;
        let document_id = format!("{}/{}", input.collection, input.key);
        let prepared = prepare_mdx(
            &document_id,
            &input.file,
            &parsed.body,
            &config.mdx,
            config.highlight.enabled,
            Path::new(&config.root),
            &config.media,
        )
        .map_err(diagnostic_error)?;
        let mut derived = serde_json::Map::new();
        if config.derived.reading_time {
            let words = prepared.reading_words;
            derived.insert(
                "readingTime".into(),
                serde_json::json!({
                    "words": words,
                    "minutes": words.div_ceil(300).max(1),
                }),
            );
        }
        if config.derived.last_modified
            && let Some(modified_at) = &input.modified_at
        {
            derived.insert("lastModified".into(), Value::String(modified_at.clone()));
        }
        let derived = Value::Object(derived);
        let projections = project_document(
            &config,
            &input.collection,
            &input.key,
            input.locale.as_deref(),
            input.slug.as_deref(),
            &input.file,
            &parsed.frontmatter,
            &derived,
        )
        .map_err(diagnostic_error)?;
        documents.push(PreparedDocument::Pending(Box::new(PendingDocument {
            cache_key: entry_key,
            cache_warning,
            derived,
            input,
            parsed,
            prepared,
            projections,
        })));
    }

    Ok(PreparedBatch {
        cache: config.cache,
        documents,
        mdx: config.mdx,
    })
}

#[napi]
pub fn render_manifests(config_json: String, records_json: String) -> Result<String> {
    let config: NativeConfig = decode_json(&config_json, "configuration")?;
    let records: Vec<DocumentRecord> = decode_json(&records_json, "document records")?;
    let outputs = render_manifest_outputs(&config, &records).map_err(diagnostic_error)?;
    serde_json::to_string(&outputs).map_err(|error| {
        Error::new(
            Status::GenericFailure,
            format!("AMAMO_NATIVE_SERIALIZE: {error}"),
        )
    })
}

#[napi]
pub fn prune_cache(cache_directory: String, keep_keys_json: String) -> Result<u32> {
    let keep: HashSet<String> = decode_json(&keep_keys_json, "cache keys")?;
    let removed =
        prune_cache_entries(Path::new(&cache_directory), &keep).map_err(diagnostic_error)?;
    u32::try_from(removed).map_err(|error| {
        Error::new(
            Status::GenericFailure,
            format!("AMAMO_CACHE_PRUNE: {error}"),
        )
    })
}

fn decode_json<T: for<'de> Deserialize<'de>>(json: &str, label: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|error| {
        diagnostic_error(vec![Diagnostic::error(
            "AMAMO_CONFIG_INVALID",
            None,
            format!("Could not decode {label}: {error}"),
        )])
    })
}

fn diagnostic_error(diagnostics: Vec<Diagnostic>) -> Error {
    let json = serde_json::to_string(&diagnostics).unwrap_or_else(|_| "[]".into());
    Error::new(
        Status::GenericFailure,
        format!("AMAMO_MDX_DIAGNOSTICS:{json}"),
    )
}
