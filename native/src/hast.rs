use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use markdown::mdast::{
    AttributeContent, AttributeValue, AttributeValueExpression, MdxJsxAttribute,
};
use mdxjs::hast::{Element, MdxJsxElement, MdxjsEsm, Node, PropertyValue, Text};
use serde::Deserialize;
use serde_json::Value;

use crate::Diagnostic;
use crate::config::{MediaMissing, NativeMediaConfig};

#[derive(Debug, Default)]
pub struct MediaRewrite {
    pub dependencies: Vec<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
}

struct MediaContext<'a> {
    bindings: HashMap<String, String>,
    config: &'a NativeMediaConfig,
    dependencies: Vec<PathBuf>,
    diagnostics: Vec<Diagnostic>,
    file: &'a Path,
    imports: Vec<(String, String)>,
    root: &'a Path,
}

#[derive(Debug)]
pub struct HighlightReplacement {
    pub block_id: usize,
    pub document_id: String,
    pub node: Node,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HighlightWire {
    block_id: usize,
    document_id: String,
    hast: HastWire,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum HastWire {
    #[serde(rename = "root")]
    Root { children: Vec<HastWire> },
    #[serde(rename = "element", rename_all = "camelCase")]
    Element {
        children: Vec<HastWire>,
        properties: serde_json::Map<String, Value>,
        tag_name: String,
    },
    #[serde(rename = "text")]
    Text { value: String },
}

pub fn decode_highlights(json: &str) -> Result<Vec<HighlightReplacement>, Vec<Diagnostic>> {
    let wires = serde_json::from_str::<Vec<HighlightWire>>(json).map_err(|error| {
        vec![Diagnostic::error(
            "AMAMO_SHIKI_INVALID_HAST",
            None,
            format!("Could not decode highlighted HAST: {error}"),
        )]
    })?;
    wires
        .into_iter()
        .map(|wire| {
            let HastWire::Root { mut children } = wire.hast else {
                return Err(vec![Diagnostic::error(
                    "AMAMO_SHIKI_INVALID_HAST",
                    None,
                    "Highlighted HAST must have a root node",
                )]);
            };
            if children.len() != 1 {
                return Err(vec![Diagnostic::error(
                    "AMAMO_SHIKI_INVALID_HAST",
                    None,
                    "Highlighted HAST root must contain one pre element",
                )]);
            }
            let node = wire_node(children.pop().expect("length checked"))?;
            if !matches!(&node, Node::Element(element) if element.tag_name == "pre") {
                return Err(vec![Diagnostic::error(
                    "AMAMO_SHIKI_INVALID_HAST",
                    None,
                    "Highlighted HAST root child must be a pre element",
                )]);
            }
            Ok(HighlightReplacement {
                block_id: wire.block_id,
                document_id: wire.document_id,
                node,
            })
        })
        .collect()
}

pub fn inject_highlights(tree: &mut Node, replacements: &HashMap<usize, Node>) -> usize {
    let mut block_id = 0;
    replace_code_blocks(tree, replacements, &mut block_id)
}

pub fn remove_table_line_breaks(node: &mut Node) {
    let remove_line_breaks = matches!(
        node,
        Node::Element(element)
            if matches!(
                element.tag_name.as_str(),
                "table" | "thead" | "tbody" | "tfoot" | "tr" | "th" | "td"
            )
    );
    let Some(children) = node.children_mut() else {
        return;
    };

    if remove_line_breaks {
        children.retain(|child| !matches!(child, Node::Text(text) if text.value == "\n"));
    }
    children.iter_mut().for_each(remove_table_line_breaks);
}

fn wire_node(wire: HastWire) -> Result<Node, Vec<Diagnostic>> {
    match wire {
        HastWire::Root { .. } => Err(vec![Diagnostic::error(
            "AMAMO_SHIKI_INVALID_HAST",
            None,
            "Nested highlighted HAST roots are not supported",
        )]),
        HastWire::Text { value } => Ok(Node::Text(Text {
            value,
            position: None,
        })),
        HastWire::Element {
            children,
            properties,
            tag_name,
        } => {
            let properties = properties
                .into_iter()
                .map(|(name, value)| wire_property(name, value))
                .collect::<Result<Vec<_>, _>>()?;
            let children = children
                .into_iter()
                .map(wire_node)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Node::Element(Element {
                tag_name,
                properties,
                children,
                position: None,
            }))
        }
    }
}

fn wire_property(name: String, value: Value) -> Result<(String, PropertyValue), Vec<Diagnostic>> {
    let name = match name.as_str() {
        "class" => "className".into(),
        "tabindex" => "tabIndex".into(),
        _ => name,
    };
    let value = match value {
        Value::Bool(value) => PropertyValue::Boolean(value),
        Value::String(value) if name == "className" => {
            PropertyValue::SpaceSeparated(value.split_whitespace().map(str::to_owned).collect())
        }
        Value::String(value) => PropertyValue::String(value),
        Value::Array(values) => PropertyValue::SpaceSeparated(
            values
                .into_iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        vec![Diagnostic::error(
                            "AMAMO_SHIKI_INVALID_HAST",
                            None,
                            "Highlighted HAST property arrays must contain strings",
                        )]
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => {
            return Err(vec![Diagnostic::error(
                "AMAMO_SHIKI_INVALID_HAST",
                None,
                "Highlighted HAST properties must be strings, booleans, or string arrays",
            )]);
        }
    };
    Ok((name, value))
}

fn replace_code_blocks(
    node: &mut Node,
    replacements: &HashMap<usize, Node>,
    block_id: &mut usize,
) -> usize {
    if is_code_block(node) {
        let current = *block_id;
        *block_id += 1;
        if let Some(replacement) = replacements.get(&current) {
            *node = replacement.clone();
            return 1;
        }
        return 0;
    }
    node.children_mut().map_or(0, |children| {
        children
            .iter_mut()
            .map(|child| replace_code_blocks(child, replacements, block_id))
            .sum()
    })
}

fn is_code_block(node: &Node) -> bool {
    matches!(
        node,
        Node::Element(element)
            if element.tag_name == "pre"
                && matches!(element.children.first(), Some(Node::Element(code)) if code.tag_name == "code")
    )
}

pub fn rewrite_media(
    tree: &mut Node,
    root: &Path,
    file: &Path,
    config: &NativeMediaConfig,
) -> Result<MediaRewrite, Vec<Diagnostic>> {
    if !config.enabled {
        return Ok(MediaRewrite::default());
    }

    let normalized_root = normalize_path(root);
    let mut context = MediaContext {
        bindings: HashMap::new(),
        config,
        dependencies: vec![],
        diagnostics: vec![],
        file,
        imports: vec![],
        root: &normalized_root,
    };
    rewrite_node(tree, &mut context)?;
    if !context.imports.is_empty()
        && let Node::Root(root) = tree
    {
        let value = context
            .imports
            .iter()
            .map(|(specifier, binding)| {
                format!(
                    "import {binding} from {};",
                    serde_json::to_string(specifier).expect("string serializes")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        root.children.insert(
            0,
            Node::MdxjsEsm(MdxjsEsm {
                value,
                position: None,
                stops: vec![],
            }),
        );
    }

    Ok(MediaRewrite {
        dependencies: context.dependencies,
        diagnostics: context.diagnostics,
    })
}

fn rewrite_node(node: &mut Node, context: &mut MediaContext<'_>) -> Result<(), Vec<Diagnostic>> {
    if let Some(children) = node.children_mut() {
        for child in children {
            rewrite_node(child, context)?;
        }
    }

    let Node::Element(element) = node else {
        return Ok(());
    };
    let Some(media_attributes) = context.config.attributes.get(&element.tag_name).cloned() else {
        return Ok(());
    };

    let mut changed = false;
    let mut attributes = Vec::with_capacity(element.properties.len());
    for (name, value) in &element.properties {
        let lower_name = name.to_ascii_lowercase();
        if media_attributes.contains(&lower_name)
            && let PropertyValue::String(url) = value
        {
            let expression = if lower_name == "srcset" {
                rewrite_srcset(url, context)?
            } else {
                rewrite_url(url, context)?
            };
            if let Some(value) = expression {
                changed = true;
                attributes.push(expression_attribute(name, value));
                continue;
            }
        }
        if let Some(attribute) = literal_attribute(name, value) {
            attributes.push(attribute);
        }
    }

    if changed {
        *node = Node::MdxJsxElement(MdxJsxElement {
            name: Some(element.tag_name.clone()),
            attributes,
            children: std::mem::take(&mut element.children),
            position: element.position.clone(),
        });
    }
    Ok(())
}

fn rewrite_url(
    url: &str,
    context: &mut MediaContext<'_>,
) -> Result<Option<String>, Vec<Diagnostic>> {
    if !is_relative_url(url) {
        return Ok(None);
    }
    if let Some(binding) = context.bindings.get(url) {
        return Ok(Some(binding.clone()));
    }

    let file_part = url.split(['?', '#']).next().unwrap_or(url);
    let parent = context.file.parent().unwrap_or_else(|| Path::new("."));
    let target = normalize_path(&parent.join(file_part));
    if !target.starts_with(context.root) {
        return Err(vec![Diagnostic::error(
            "AMAMO_MEDIA_OUTSIDE_ROOT",
            context.file.to_str(),
            format!("Media path `{url}` escapes the configured project root"),
        )]);
    }

    let dependency = match fs::canonicalize(&target) {
        Ok(canonical_target) => {
            let canonical_root =
                fs::canonicalize(context.root).unwrap_or_else(|_| context.root.to_path_buf());
            if !canonical_target.starts_with(canonical_root) {
                return Err(vec![Diagnostic::error(
                    "AMAMO_MEDIA_OUTSIDE_ROOT",
                    context.file.to_str(),
                    format!("Media path `{url}` resolves outside the configured project root"),
                )]);
            }
            canonical_target
        }
        Err(_) if context.config.missing == MediaMissing::Warn => {
            context.diagnostics.push(Diagnostic::warning(
                "AMAMO_MEDIA_MISSING",
                context.file.to_str(),
                format!("Media file `{url}` does not exist"),
            ));
            return Ok(None);
        }
        Err(_) => {
            return Err(vec![Diagnostic::error(
                "AMAMO_MEDIA_MISSING",
                context.file.to_str(),
                format!("Media file `{url}` does not exist"),
            )]);
        }
    };

    let binding = format!("_amamoMedia{}", context.imports.len());
    context.bindings.insert(url.into(), binding.clone());
    context.imports.push((url.into(), binding.clone()));
    if !context.dependencies.contains(&dependency) {
        context.dependencies.push(dependency);
    }
    Ok(Some(binding))
}

fn rewrite_srcset(
    value: &str,
    context: &mut MediaContext<'_>,
) -> Result<Option<String>, Vec<Diagnostic>> {
    let mut imported = false;
    let mut entries = Vec::new();
    for raw_entry in value.split(',') {
        let entry = raw_entry.trim();
        let split = entry.find(char::is_whitespace).unwrap_or(entry.len());
        let (url, descriptor) = entry.split_at(split);
        if let Some(binding) = rewrite_url(url, context)? {
            imported = true;
            entries.push(format!(
                "{binding} + {}",
                serde_json::to_string(descriptor).expect("string serializes")
            ));
        } else {
            entries.push(serde_json::to_string(entry).expect("string serializes"));
        }
    }
    if imported {
        Ok(Some(entries.join(" + \", \" + ")))
    } else {
        Ok(None)
    }
}

fn is_relative_url(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['/', '#'])
        && !value.starts_with("//")
        && !value.starts_with("data:")
        && !value
            .split('/')
            .next()
            .is_some_and(|prefix| prefix.contains(':'))
}

fn expression_attribute(name: &str, value: String) -> AttributeContent {
    AttributeContent::Property(MdxJsxAttribute {
        name: name.into(),
        value: Some(AttributeValue::Expression(AttributeValueExpression {
            value,
            stops: vec![],
        })),
    })
}

fn literal_attribute(name: &str, value: &PropertyValue) -> Option<AttributeContent> {
    let value = match value {
        PropertyValue::Boolean(true) => None,
        PropertyValue::Boolean(false) => return None,
        PropertyValue::String(value) => Some(AttributeValue::Literal(value.clone())),
        PropertyValue::CommaSeparated(values) => Some(AttributeValue::Literal(values.join(", "))),
        PropertyValue::SpaceSeparated(values) => Some(AttributeValue::Literal(values.join(" "))),
    };
    Some(AttributeContent::Property(MdxJsxAttribute {
        name: name.into(),
        value,
    }))
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
            Component::Prefix(value) => normalized.push(value.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
        }
    }
    normalized
}
