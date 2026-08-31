use std::collections::HashMap;

use markdown::mdast::{AttributeContent, AttributeValue, Node};
use mdxjs::hast::{Node as HastNode, PropertyValue};
use serde::Serialize;
use unicode_general_category::{GeneralCategory, get_general_category};

#[derive(Debug)]
pub struct PreparedContent {
    pub headings: Vec<Heading>,
    pub structured_data: StructuredData,
}

#[derive(Debug)]
pub struct Heading {
    depth: u8,
    id: String,
}

#[derive(Debug)]
pub struct TocItem {
    pub depth: u8,
    pub title: Vec<HastNode>,
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredData {
    contents: Vec<StructuredDataContent>,
    headings: Vec<StructuredDataHeading>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StructuredDataHeading {
    content: String,
    id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StructuredDataContent {
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    heading: Option<String>,
}

#[derive(Default)]
struct Slugger {
    occurrences: HashMap<String, usize>,
}

impl Slugger {
    fn slug(&mut self, value: &str) -> String {
        let original = github_slug(value);
        let mut result = original.clone();
        while self.occurrences.contains_key(&result) {
            let occurrence = self.occurrences.entry(original.clone()).or_default();
            *occurrence += 1;
            result = format!("{original}-{occurrence}");
        }
        self.occurrences.insert(result.clone(), 0);
        result
    }
}

pub fn prepare_content(tree: &mut Node, heading_ids: bool) -> PreparedContent {
    let mut headings = Vec::new();
    if heading_ids {
        collect_headings(tree, &mut Slugger::default(), &mut headings);
    }
    let structured_data = extract_structured_data(tree, &headings);
    PreparedContent {
        headings,
        structured_data,
    }
}

pub fn apply_heading_ids(tree: &mut HastNode, headings: &[Heading]) -> Vec<TocItem> {
    let mut index = 0;
    let mut toc = Vec::with_capacity(headings.len());
    apply_heading_ids_inner(tree, headings, &mut index, &mut toc);
    toc
}

fn collect_headings(node: &mut Node, slugger: &mut Slugger, headings: &mut Vec<Heading>) {
    if let Node::Heading(heading) = node {
        let custom_id = take_custom_id(&mut heading.children);
        let title = flatten_children(&heading.children);
        headings.push(Heading {
            depth: heading.depth,
            id: custom_id.unwrap_or_else(|| slugger.slug(&title)),
        });
        return;
    }

    if let Some(children) = node.children_mut() {
        for child in children {
            collect_headings(child, slugger, headings);
        }
    }
}

fn take_custom_id(children: &mut [Node]) -> Option<String> {
    let Node::Text(last) = children.last_mut()? else {
        return None;
    };
    let trimmed = last.value.trim_end();
    if !trimmed.ends_with(']') {
        return None;
    }
    let start = trimmed.find("[#")?;
    let id = &trimmed[start + 2..trimmed.len() - 1];
    if id.is_empty() {
        return None;
    }
    let prefix = trimmed[..start].trim_end();
    let prefix_len = prefix.len();
    let id = id.to_owned();
    last.value.truncate(prefix_len);
    Some(id)
}

fn github_slug(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|character| {
            if character == ' ' {
                return Some('-');
            }
            if character == '-' || character == '_' || is_slug_character(character) {
                return Some(character);
            }
            None
        })
        .collect()
}

fn is_slug_character(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::DecimalNumber
            | GeneralCategory::EnclosingMark
            | GeneralCategory::LetterNumber
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::NonspacingMark
            | GeneralCategory::OtherLetter
            | GeneralCategory::OtherNumber
            | GeneralCategory::SpacingMark
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::UppercaseLetter
    )
}

fn flatten_children(children: &[Node]) -> String {
    children.iter().map(flatten_node).collect()
}

fn flatten_node(node: &Node) -> String {
    if let Some(children) = node.children() {
        return flatten_children(children);
    }
    match node {
        Node::Code(node) => node.value.clone(),
        Node::Html(node) => node.value.clone(),
        Node::InlineCode(node) => node.value.clone(),
        Node::InlineMath(node) => node.value.clone(),
        Node::Math(node) => node.value.clone(),
        Node::MdxFlowExpression(node) => node.value.clone(),
        Node::MdxTextExpression(node) => node.value.clone(),
        Node::MdxjsEsm(node) => node.value.clone(),
        Node::Text(node) => node.value.clone(),
        Node::Toml(node) => node.value.clone(),
        Node::Yaml(node) => node.value.clone(),
        _ => String::new(),
    }
}

fn extract_structured_data(tree: &Node, headings: &[Heading]) -> StructuredData {
    let mut data = StructuredData {
        contents: Vec::new(),
        headings: Vec::new(),
    };
    let mut heading_index = 0;
    let mut last_heading = None;
    extract_structured_data_inner(
        tree,
        headings,
        &mut heading_index,
        &mut last_heading,
        &mut data,
    );
    data
}

fn extract_structured_data_inner(
    node: &Node,
    headings: &[Heading],
    heading_index: &mut usize,
    last_heading: &mut Option<String>,
    data: &mut StructuredData,
) {
    if let Node::Heading(heading) = node {
        let info = headings.get(*heading_index);
        *heading_index += 1;
        if let Some(info) = info {
            let content = render_children(&heading.children).trim().to_owned();
            if !content.is_empty() {
                data.headings.push(StructuredDataHeading {
                    content,
                    id: info.id.clone(),
                });
            }
            *last_heading = Some(info.id.clone());
        }
        return;
    }

    let selected = matches!(
        node,
        Node::Blockquote(_) | Node::Paragraph(_) | Node::TableCell(_)
    ) || matches!(node, Node::MdxJsxFlowElement(element) if element.children.is_empty());
    if selected {
        let content = render_node(node).trim().to_owned();
        if !content.is_empty() {
            data.contents.push(StructuredDataContent {
                content,
                heading: last_heading.clone(),
            });
        }
        return;
    }

    if let Some(children) = node.children() {
        for child in children {
            extract_structured_data_inner(child, headings, heading_index, last_heading, data);
        }
    }
}

fn render_children(children: &[Node]) -> String {
    children.iter().map(render_node).collect()
}

fn render_node(node: &Node) -> String {
    match node {
        Node::Blockquote(node) => {
            let content = render_children(&node.children).trim().replace('\n', "\n> ");
            format!("> {content}")
        }
        Node::Break(_) => "\n".into(),
        Node::Code(node) => format!(
            "```{}\n{}\n```",
            node.lang.as_deref().unwrap_or(""),
            node.value
        ),
        Node::Delete(node) => format!("~~{}~~", render_children(&node.children)),
        Node::Emphasis(node) => format!("*{}*", render_children(&node.children)),
        Node::Html(node) => node.value.clone(),
        Node::Image(_) | Node::ImageReference(_) => String::new(),
        Node::InlineCode(node) => format!("`{}`", node.value),
        Node::InlineMath(node) => format!("${}$", node.value),
        Node::Link(node) => render_children(&node.children),
        Node::LinkReference(node) => render_children(&node.children),
        Node::List(node) => node
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let marker = if node.ordered {
                    format!("{}. ", node.start.unwrap_or(1) as usize + index)
                } else {
                    "- ".into()
                };
                format!("{marker}{}", render_node(child).trim())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Node::Math(node) => format!("$$\n{}\n$$", node.value),
        Node::MdxFlowExpression(node) => format!("{{{}}}", node.value),
        Node::MdxJsxFlowElement(node) => {
            render_mdx_element(node.name.as_deref(), &node.attributes, &node.children)
        }
        Node::MdxJsxTextElement(node) => {
            render_mdx_element(node.name.as_deref(), &node.attributes, &node.children)
        }
        Node::MdxTextExpression(node) => format!("{{{}}}", node.value),
        Node::Paragraph(node) => render_children(&node.children),
        Node::Strong(node) => format!("**{}**", render_children(&node.children)),
        Node::Text(node) => node.value.clone(),
        _ => node
            .children()
            .map_or_else(String::new, |children| render_children(children)),
    }
}

fn render_mdx_element(
    name: Option<&str>,
    attributes: &[AttributeContent],
    children: &[Node],
) -> String {
    let content = render_children(children);
    let Some(name) = name else {
        return content;
    };
    if !matches!(name, "File" | "TypeTable" | "Callout" | "Card") {
        return content;
    }
    let attributes = attributes
        .iter()
        .filter_map(|attribute| {
            let AttributeContent::Property(attribute) = attribute else {
                return None;
            };
            let value = match &attribute.value {
                None => return None,
                Some(AttributeValue::Literal(value)) => value,
                Some(AttributeValue::Expression(value)) => &value.value,
            };
            if value.is_empty() {
                return None;
            }
            Some(format!(
                " {}=\"{}\"",
                attribute.name,
                escape_attribute(value)
            ))
        })
        .collect::<String>();
    if children.is_empty() {
        format!("<{name}{attributes} />")
    } else {
        format!("<{name}{attributes}>{content}</{name}>")
    }
}

fn escape_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

fn apply_heading_ids_inner(
    node: &mut HastNode,
    headings: &[Heading],
    index: &mut usize,
    toc: &mut Vec<TocItem>,
) {
    if let HastNode::Element(element) = node
        && matches!(
            element.tag_name.as_str(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        )
        && let Some(heading) = headings.get(*index)
    {
        *index += 1;
        if let Some((_, value)) = element.properties.iter_mut().find(|(name, _)| name == "id") {
            *value = PropertyValue::String(heading.id.clone());
        } else {
            element
                .properties
                .push(("id".into(), PropertyValue::String(heading.id.clone())));
        }
        toc.push(TocItem {
            depth: heading.depth,
            title: element.children.clone(),
            url: format!("#{}", heading.id),
        });
        return;
    }

    if let Some(children) = node.children_mut() {
        for child in children {
            apply_heading_ids_inner(child, headings, index, toc);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_heading_ids, prepare_content};

    #[test]
    fn matches_fumadocs_heading_ids_and_custom_ids() {
        let source = "## Route *Planning* [#route]\n\n## A/B\n\n## A  B\n\n## foo_bar\n\n## Repeat\n\n## Repeat\n\n## ![Map](map.png)\n";
        let options = mdxjs::Options::gfm();
        let mut mdast = mdxjs::mdast_util_from_mdx(source, &options).unwrap();
        let content = prepare_content(&mut mdast, true);
        let mut hast = mdxjs::mdast_util_to_hast(&mdast);
        let toc = apply_heading_ids(&mut hast, &content.headings);

        assert_eq!(
            toc.iter().map(|item| item.url.as_str()).collect::<Vec<_>>(),
            [
                "#route",
                "#ab",
                "#a--b",
                "#foo_bar",
                "#repeat",
                "#repeat-1",
                "#"
            ]
        );
        assert_eq!(toc[0].depth, 2);
        assert_eq!(
            toc[0]
                .title
                .iter()
                .map(ToString::to_string)
                .collect::<String>(),
            "Route Planning"
        );
    }

    #[test]
    fn extracts_fumadocs_compatible_structured_data() {
        let source = "## Route *Planning* [#route]\n\nA **bold** [link](https://example.com) with `code` and ![alt](x.png).\n\n> Quote *text*.\n\n| A | B |\n| - | - |\n| one | two |\n\n<Tabs>Inside **tabs**.</Tabs>\n\n<Callout type=\"warn\" foo={bar} enabled />\n";
        let options = mdxjs::Options::gfm();
        let mut mdast = mdxjs::mdast_util_from_mdx(source, &options).unwrap();
        let content = prepare_content(&mut mdast, true);

        assert_eq!(
            serde_json::to_value(content.structured_data).unwrap(),
            json!({
                "contents": [
                    { "heading": "route", "content": "A **bold** link with `code` and ." },
                    { "heading": "route", "content": "> Quote *text*." },
                    { "heading": "route", "content": "A" },
                    { "heading": "route", "content": "B" },
                    { "heading": "route", "content": "one" },
                    { "heading": "route", "content": "two" },
                    { "heading": "route", "content": "Inside **tabs**." },
                    { "heading": "route", "content": "<Callout type=\"warn\" foo=\"bar\" />" }
                ],
                "headings": [{ "id": "route", "content": "Route *Planning*" }]
            })
        );
    }
}
