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
use crate::hast::{remove_table_line_breaks, rewrite_document_anchors, rewrite_media};
use crate::math::replace_math;

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
    synthetic_jsx_spans: Vec<swc_core::common::Span>,
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
    let mut mdast = mdxjs::mdast_util_from_mdx(body, &options)
        .map_err(|error| vec![mdx_diagnostic(file, error)])?;
    let reading_words = reading_units(&mdast);
    let mut code_blocks = Vec::new();
    if highlight {
        collect_code_blocks(&mdast, document_id, &mut code_blocks);
    }
    if config.hard_breaks {
        apply_hard_breaks(&mut mdast);
    }
    let mut tree = mdxjs::mdast_util_to_hast(&mdast);
    rewrite_document_anchors(&mut tree, document_id, config.extensions.heading_ids);
    replace_math(&mut tree, file, &config.math)?;
    remove_table_line_breaks(&mut tree);
    let media = rewrite_media(&mut tree, root, Path::new(file), media)?;

    Ok(PreparedMdx {
        code_blocks,
        dependencies: media.dependencies,
        diagnostics: media.diagnostics,
        reading_words,
        synthetic_jsx_spans: media.synthetic_jsx_spans,
        tree,
    })
}

fn apply_hard_breaks(node: &mut markdown::mdast::Node) {
    let Some(children) = node.children_mut() else {
        return;
    };
    let old_children = std::mem::take(children);
    for mut child in old_children {
        if let markdown::mdast::Node::Text(text) = &child
            && (text.value.contains('\n') || text.value.contains('\r'))
        {
            let normalized = text.value.replace("\r\n", "\n").replace('\r', "\n");
            let parts = normalized.split('\n').collect::<Vec<_>>();
            for (index, part) in parts.iter().enumerate() {
                if !part.is_empty() {
                    children.push(markdown::mdast::Node::Text(markdown::mdast::Text {
                        value: (*part).into(),
                        position: None,
                    }));
                }
                if index + 1 < parts.len() {
                    children.push(markdown::mdast::Node::Break(markdown::mdast::Break {
                        position: None,
                    }));
                }
            }
            continue;
        }
        apply_hard_breaks(&mut child);
        children.push(child);
    }
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
    for span in prepared.synthetic_jsx_spans {
        explicit_jsxs.remove(&span);
    }
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
                    value: react_style_name(name).into(),
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

fn react_style_name(name: &str) -> String {
    if name.starts_with("--") {
        return name.into();
    }
    let mut result = String::with_capacity(name.len());
    let mut uppercase_next = false;
    for character in name.chars() {
        if character == '-' {
            uppercase_next = true;
        } else if uppercase_next {
            result.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            result.push(character);
        }
    }
    if name.starts_with("-ms-") {
        result.replace_range(..1, "m");
    }
    result
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
    options.parse.constructs.gfm_footnote_definition = config.extensions.footnotes;
    options.parse.constructs.gfm_label_start_footnote = config.extensions.footnotes;
    options.parse.constructs.gfm_task_list_item = config.extensions.task_lists;
    options.parse.constructs.math_flow = config.math.enabled;
    options.parse.constructs.math_text = config.math.enabled;
    options.parse.math_text_single_dollar = config.math.single_dollar;
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

    use mdxjs::hast::Node;
    use serde_json::json;

    use crate::config::{
        MediaMissing, NativeCollectionConfig, NativeMathConfig, NativeMdxConfig, NativeMediaConfig,
    };

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
            extensions: Default::default(),
            gfm: true,
            hard_breaks: true,
            jsx_import_source: "react".into(),
            math: NativeMathConfig::default(),
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
    fn gfm_tables_do_not_keep_structure_line_breaks() {
        let source = "| a | b |\n| - | - |\n| 1 | 2 |\n";
        let options = NativeMdxConfig {
            extensions: Default::default(),
            gfm: true,
            hard_breaks: true,
            jsx_import_source: "react".into(),
            math: NativeMathConfig::default(),
            provider_import_source: String::new(),
        };
        let prepared = prepare_mdx(
            "posts/table",
            "/project/table.mdx",
            source,
            &options,
            false,
            std::path::Path::new("/project"),
            &NativeMediaConfig::default(),
        )
        .unwrap();

        assert!(!has_table_structure_line_break(&prepared.tree));
    }

    #[test]
    fn hard_breaks_only_replace_newlines_inside_text_nodes() {
        let source = "first\nsecond\n\n# Heading\n\n| a | b |\n| - | - |\n| 1 | 2 |\n";
        let options = NativeMdxConfig {
            extensions: Default::default(),
            gfm: true,
            hard_breaks: true,
            jsx_import_source: "react".into(),
            math: NativeMathConfig::default(),
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
            &json!({}),
            &json!({}),
        )
        .unwrap();

        assert_eq!(module.matches("_components.br").count(), 1);
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
            extensions: Default::default(),
            gfm: true,
            hard_breaks: false,
            jsx_import_source: "react".into(),
            math: NativeMathConfig::default(),
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
        assert!(module.contains("img: \"img\""));
        assert_eq!(module.matches("_jsx(_components.img,").count(), 1);
        assert_eq!(module.matches("_jsx(\"img\",").count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_media_that_escapes_the_project_root() {
        let options = NativeMdxConfig {
            extensions: Default::default(),
            gfm: true,
            hard_breaks: false,
            jsx_import_source: "react".into(),
            math: NativeMathConfig::default(),
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

    fn has_table_structure_line_break(node: &Node) -> bool {
        let is_table_structure = matches!(
            node,
            Node::Element(element)
                if matches!(
                    element.tag_name.as_str(),
                    "table" | "thead" | "tbody" | "tfoot" | "tr" | "th" | "td"
                )
        );
        let Some(children) = node.children() else {
            return false;
        };

        (is_table_structure
            && children
                .iter()
                .any(|child| matches!(child, Node::Text(text) if text.value == "\n")))
            || children.iter().any(has_table_structure_line_break)
    }
}
