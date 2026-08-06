use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use swc_core::common::DUMMY_SP;
use swc_core::ecma::ast::{
    Expr, JSXAttr, JSXAttrName, JSXAttrValue, JSXExpr, JSXExprContainer, KeyValueProp, Lit,
    ObjectLit, Prop, PropName, PropOrSpread, Str,
};
use swc_core::ecma::visit::{VisitMut, VisitMutWith};

use crate::Diagnostic;
use crate::config::{
    NativeCollectionConfig, NativeMdxConfig, NativeMediaConfig, apply_schema_defaults_and_validate,
};
use crate::hast::{apply_hard_breaks, rewrite_media};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBlock {
    pub block_id: usize,
    pub code: String,
    pub document_id: String,
    pub lang: Option<String>,
    pub meta: Option<String>,
}

#[derive(Debug)]
pub struct PreparedMdx {
    pub code_blocks: Vec<CodeBlock>,
    pub dependencies: Vec<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    pub reading_words: usize,
    pub(crate) tree: mdxjs::hast::Node,
}

#[derive(Debug)]
pub struct ParsedFrontmatter {
    pub body: String,
    pub public: Value,
    pub sensitive: Value,
}

pub fn parse_frontmatter(
    file: &str,
    source: &str,
    collection: &NativeCollectionConfig,
) -> Result<ParsedFrontmatter, Vec<Diagnostic>> {
    let (yaml, body) = split_frontmatter(file, source)?;
    let mut public = match yaml {
        Some(value) => yaml_serde::from_str::<Value>(value).map_err(|error| {
            vec![Diagnostic::error(
                "AMAMO_FRONTMATTER_INVALID",
                Some(file),
                format!("Could not parse YAML frontmatter: {error}"),
            )]
        })?,
        None => Value::Object(Map::new()),
    };
    if public.is_null() {
        public = Value::Object(Map::new());
    }
    if !public.is_object() {
        return Err(vec![Diagnostic::error(
            "AMAMO_FRONTMATTER_INVALID",
            Some(file),
            "YAML frontmatter must be an object",
        )]);
    }

    apply_schema_defaults_and_validate(file, &mut public, collection)?;
    let mut sensitive = Map::new();
    let public_object = public.as_object_mut().expect("object checked above");
    for field in &collection.sensitive {
        if let Some(value) = public_object.remove(field) {
            sensitive.insert(field.clone(), value);
        }
    }

    Ok(ParsedFrontmatter {
        body: body.to_owned(),
        public,
        sensitive: Value::Object(sensitive),
    })
}

pub fn prepare_mdx(
    document_id: &str,
    file: &str,
    body: &str,
    config: &NativeMdxConfig,
    highlight: bool,
    root: &Path,
    media: &NativeMediaConfig,
) -> Result<PreparedMdx, Vec<Diagnostic>> {
    let options = mdx_options(file, config);
    let mdast = mdxjs::mdast_util_from_mdx(body, &options)
        .map_err(|error| vec![mdx_diagnostic(file, error)])?;
    let reading_words = reading_units(&mdast);
    let mut code_blocks = Vec::new();
    if highlight {
        collect_code_blocks(&mdast, document_id, &mut code_blocks);
    }
    let mut tree = mdxjs::mdast_util_to_hast(&mdast);
    if config.hard_breaks {
        apply_hard_breaks(&mut tree);
    }
    let media = rewrite_media(&mut tree, root, Path::new(file), media)?;

    Ok(PreparedMdx {
        code_blocks,
        dependencies: media.dependencies,
        diagnostics: media.diagnostics,
        reading_words,
        tree,
    })
}

fn reading_units(node: &markdown::mdast::Node) -> usize {
    match node {
        markdown::mdast::Node::Code(_) | markdown::mdast::Node::InlineCode(_) => 0,
        markdown::mdast::Node::Text(text) => count_reading_units(&text.value),
        _ => node
            .children()
            .map(|children| children.iter().map(reading_units).sum())
            .unwrap_or_default(),
    }
}

fn count_reading_units(text: &str) -> usize {
    let mut count = 0;
    let mut in_english_word = false;
    for character in text.chars() {
        if matches!(
            character,
            '\u{4e00}'..='\u{9fff}'
                | '\u{3040}'..='\u{309f}'
                | '\u{30a0}'..='\u{30ff}'
                | '\u{ac00}'..='\u{d7af}'
        ) {
            count += 1;
            in_english_word = false;
        } else if character.is_ascii_alphabetic() {
            if !in_english_word {
                count += 1;
                in_english_word = true;
            }
        } else {
            in_english_word = false;
        }
    }
    count
}

pub fn finish_mdx(
    prepared: PreparedMdx,
    file: &str,
    body: &str,
    config: &NativeMdxConfig,
    frontmatter: &Value,
    derived: &Value,
) -> Result<String, Vec<Diagnostic>> {
    let options = mdx_options(file, config);
    let location = markdown::Location::new(body.as_bytes());
    let mut explicit_jsxs = Default::default();
    let mut program = mdxjs::hast_util_to_swc(
        &prepared.tree,
        &options,
        Some(&location),
        &mut explicit_jsxs,
    )
    .map_err(|error| vec![mdx_diagnostic(file, error)])?;
    program.module.visit_mut_with(&mut ReactStyleProperties);
    mdxjs::mdx_plugin_recma_document(&mut program, &options, Some(&location))
        .map_err(|error| vec![mdx_diagnostic(file, error)])?;
    mdxjs::mdx_plugin_recma_jsx_rewrite(&mut program, &options, Some(&location), &explicit_jsxs)
        .map_err(|error| vec![mdx_diagnostic(file, error)])?;

    let mut module = program.serialize();
    module.push_str("export const frontmatter = ");
    module.push_str(&serde_json::to_string(frontmatter).expect("JSON values serialize"));
    module.push_str(";\n");
    if let Some(values) = derived.as_object() {
        for (name, value) in values {
            module.push_str("export const ");
            module.push_str(name);
            module.push_str(" = ");
            module.push_str(&serde_json::to_string(value).expect("JSON values serialize"));
            module.push_str(";\n");
        }
    }
    Ok(module)
}

struct ReactStyleProperties;

impl VisitMut for ReactStyleProperties {
    fn visit_mut_jsx_attr(&mut self, attribute: &mut JSXAttr) {
        let JSXAttrName::Ident(name) = &attribute.name else {
            return;
        };
        let Some(JSXAttrValue::Lit(Lit::Str(style))) = &attribute.value else {
            return;
        };
        if name.sym != *"style" {
            return;
        }
        let Some(style) = react_style_object(style.value.as_ref()) else {
            return;
        };
        attribute.value = Some(JSXAttrValue::JSXExprContainer(JSXExprContainer {
            expr: JSXExpr::Expr(Box::new(style)),
            span: DUMMY_SP,
        }));
    }
}

fn react_style_object(style: &str) -> Option<Expr> {
    let properties = style
        .split(';')
        .filter(|declaration| !declaration.trim().is_empty())
        .map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some(PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                key: PropName::Str(Str {
                    span: DUMMY_SP,
                    value: name.into(),
                    raw: None,
                }),
                value: Box::new(Expr::Lit(Lit::Str(Str {
                    span: DUMMY_SP,
                    value: value.into(),
                    raw: None,
                }))),
            }))))
        })
        .collect::<Option<Vec<_>>>()?;
    (!properties.is_empty()).then_some(Expr::Object(ObjectLit {
        span: DUMMY_SP,
        props: properties,
    }))
}

fn mdx_options(file: &str, config: &NativeMdxConfig) -> mdxjs::Options {
    let mut options = if config.gfm {
        mdxjs::Options::gfm()
    } else {
        mdxjs::Options::default()
    };
    options.filepath = Some(file.into());
    options.jsx_import_source = Some(config.jsx_import_source.clone());
    options.provider_import_source = if config.provider_import_source.is_empty() {
        None
    } else {
        Some(config.provider_import_source.clone())
    };
    options
}

fn collect_code_blocks(
    node: &markdown::mdast::Node,
    document_id: &str,
    blocks: &mut Vec<CodeBlock>,
) {
    if let markdown::mdast::Node::Code(code) = node {
        blocks.push(CodeBlock {
            block_id: blocks.len(),
            code: code.value.clone(),
            document_id: document_id.into(),
            lang: code.lang.clone(),
            meta: code.meta.clone(),
        });
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_code_blocks(child, document_id, blocks);
        }
    }
}

fn mdx_diagnostic(file: &str, error: markdown::message::Message) -> Diagnostic {
    Diagnostic::error("AMAMO_MDX_SYNTAX", Some(file), error.to_string())
}

fn split_frontmatter<'a>(
    file: &str,
    source: &'a str,
) -> Result<(Option<&'a str>, &'a str), Vec<Diagnostic>> {
    let first_line_end = source.find('\n').map_or(source.len(), |index| index + 1);
    if source[..first_line_end].trim_end_matches(['\r', '\n']) != "---" {
        return Ok((None, source));
    }

    let mut offset = first_line_end;
    for line in source[first_line_end..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let yaml = &source[first_line_end..offset];
            let body_start = offset + line.len();
            return Ok((Some(yaml), &source[body_start..]));
        }
        offset += line.len();
    }

    Err(vec![Diagnostic::error(
        "AMAMO_FRONTMATTER_INVALID",
        Some(file),
        "Frontmatter opening delimiter has no closing delimiter",
    )])
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use crate::config::{MediaMissing, NativeCollectionConfig, NativeMdxConfig, NativeMediaConfig};

    use super::{finish_mdx, parse_frontmatter, prepare_mdx};

    #[test]
    fn validates_defaults_without_leaking_sensitive_fields() {
        let collection = NativeCollectionConfig {
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "draft": { "type": "boolean", "default": false },
                    "password": { "type": "string" }
                },
                "required": ["title"]
            }),
            sensitive: vec!["password".into()],
        };
        let source = "---\ntitle: Hello\npassword: secret\n---\n# Hello\n";

        let parsed = parse_frontmatter("/project/post.mdx", source, &collection).unwrap();

        assert_eq!(parsed.public["draft"], false);
        assert!(parsed.public.get("password").is_none());
        assert_eq!(parsed.sensitive["password"], "secret");
        assert_eq!(parsed.body, "# Hello\n");
    }

    #[test]
    fn rejects_missing_required_frontmatter() {
        let collection = NativeCollectionConfig {
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "title": { "type": "string" } },
                "required": ["title"]
            }),
            sensitive: vec![],
        };

        let diagnostics =
            parse_frontmatter("/project/post.mdx", "---\ndraft: true\n---\n", &collection)
                .unwrap_err();

        assert_eq!(diagnostics[0].code, "AMAMO_SCHEMA_INVALID");
    }

    #[test]
    fn compiles_mdx_and_exports_safe_metadata() {
        let source = "import Badge from './badge.js'\n\n# Hello\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nfirst\nsecond\n\n<Badge />\n";
        let options = NativeMdxConfig {
            gfm: true,
            hard_breaks: true,
            jsx_import_source: "react".into(),
            provider_import_source: String::new(),
        };
        let prepared = prepare_mdx(
            "posts/hello",
            "/project/hello.mdx",
            source,
            &options,
            false,
            std::path::Path::new("/project"),
            &NativeMediaConfig::default(),
        )
        .unwrap();
        let module = finish_mdx(
            prepared,
            "/project/hello.mdx",
            source,
            &options,
            &json!({ "title": "Hello" }),
            &json!({ "readingTime": { "words": 6, "minutes": 1 } }),
        )
        .unwrap();

        assert!(module.contains("react/jsx-runtime"));
        assert!(module.contains("import Badge from './badge.js'"));
        assert!(module.contains("_components.table"));
        assert!(module.contains("_components.br"));
        assert!(module.contains("export const frontmatter"));
        assert!(module.contains("export const readingTime"));
        assert!(!module.contains("secret"));
    }

    #[test]
    fn rewrites_markdown_media_but_not_authored_jsx() {
        let root = std::env::temp_dir().join(format!("amamo-mdx-media-{}", std::process::id()));
        let content = root.join("content");
        fs::create_dir_all(&content).unwrap();
        fs::write(content.join("image.png"), b"image").unwrap();
        let file = content.join("post.mdx");
        let source = "![alt](./image.png)\n\n<img src=\"./authored.png\" />\n";
        let options = NativeMdxConfig {
            gfm: true,
            hard_breaks: false,
            jsx_import_source: "react".into(),
            provider_import_source: String::new(),
        };
        let media = NativeMediaConfig::default();

        let prepared = prepare_mdx(
            "posts/post",
            file.to_str().unwrap(),
            source,
            &options,
            false,
            &root,
            &media,
        )
        .unwrap();
        assert_eq!(
            prepared.dependencies,
            vec![fs::canonicalize(content.join("image.png")).unwrap()]
        );
        let module = finish_mdx(
            prepared,
            file.to_str().unwrap(),
            source,
            &options,
            &json!({}),
            &json!({}),
        )
        .unwrap();

        assert!(module.contains("_amamoMedia0"));
        assert!(module.contains("./image.png"));
        assert!(module.contains("./authored.png"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_media_that_escapes_the_project_root() {
        let options = NativeMdxConfig {
            gfm: true,
            hard_breaks: false,
            jsx_import_source: "react".into(),
            provider_import_source: String::new(),
        };
        let media = NativeMediaConfig {
            missing: MediaMissing::Error,
            ..NativeMediaConfig::default()
        };
        let diagnostics = prepare_mdx(
            "posts/post",
            "/project/content/post.mdx",
            "![](../../outside.png)",
            &options,
            false,
            std::path::Path::new("/project"),
            &media,
        )
        .unwrap_err();

        assert_eq!(diagnostics[0].code, "AMAMO_MEDIA_OUTSIDE_ROOT");
    }
}
