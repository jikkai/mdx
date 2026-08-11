use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use mdxjs::hast::{Element, Node, PropertyValue};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::{ParseNode, Parser};
use ratex_svg::{SvgColorSyntax, SvgOptions, render_to_svg_with_color_syntax};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;
use serde_json::Value;

use crate::Diagnostic;
use crate::config::NativeMathConfig;

const MAX_TEX_BYTES: usize = 64 * 1024;
const MAX_MACRO_BYTES: usize = 1024;
const MAX_MACROS_BYTES: usize = 16 * 1024;
const MAX_EXPANDED_TOKENS: usize = 100_000;
const MAX_AST_BYTES: usize = 8 * 1024 * 1024;
const MAX_AST_NODES: usize = 100_000;
const MAX_ARRAY_CELLS: usize = 100_000;
const MAX_DISPLAY_ITEMS: usize = 10_000;
const MAX_DIMENSION_EM: f64 = 10_000.0;
const MAX_SVG_BYTES: usize = 16 * 1024 * 1024;
const MAX_SVG_NODES: usize = 100_000;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

// RaTeX only accepts numeric colors. Its supported explicit alpha syntax uses n/255, so this
// opacity cannot collide with an authored color while still surviving six-decimal SVG output.
const INHERITED_COLOR: Color = Color::new(1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 0.123_456);
const INHERITED_PAINT: &str = "rgb(1,2,3)";
const INHERITED_OPACITY: &str = "0.123456";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MathError {
    Parse(String),
    InvalidSvg(String),
}

impl MathError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::Parse(_) => "AMAMO_MATH_PARSE",
            Self::InvalidSvg(_) => "AMAMO_MATH_RENDER",
        }
    }
}

impl fmt::Display for MathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(formatter, "Could not parse math expression: {message}"),
            Self::InvalidSvg(message) => {
                write!(
                    formatter,
                    "Could not produce self-contained math SVG: {message}"
                )
            }
        }
    }
}

impl Error for MathError {}

pub(crate) fn replace_math(
    tree: &mut Node,
    file: &str,
    config: &NativeMathConfig,
) -> Result<(), Vec<Diagnostic>> {
    if !config.enabled {
        return Ok(());
    }

    let source =
        math_source(tree).map(|(tex, display)| (tex.to_owned(), display, tree.position().cloned()));
    if let Some((tex, display, position)) = source {
        let mut replacement = render_math(&tex, display, &config.macros).map_err(|error| {
            vec![Diagnostic::error(
                error.code(),
                Some(file),
                error.to_string(),
            )]
        })?;
        replacement.position_set(position);
        *tree = replacement;
        return Ok(());
    }

    if let Some(children) = tree.children_mut() {
        for child in children {
            replace_math(child, file, config)?;
        }
    }
    Ok(())
}

pub(crate) fn render_math(
    tex: &str,
    display: bool,
    macros: &HashMap<String, String>,
) -> Result<Node, MathError> {
    if tex.len() > MAX_TEX_BYTES {
        return Err(MathError::Parse(format!(
            "formula exceeds the {MAX_TEX_BYTES}-byte limit"
        )));
    }
    reject_mutating_commands(tex)?;
    validate_macros(macros)?;
    validate_expanded_tokens(tex, macros)?;

    let mut parser = parser_with_macros(tex, macros);
    let parsed = parser
        .parse()
        .map_err(|error| MathError::Parse(error.to_string()))?;
    validate_ast_structure(&parsed)?;

    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let options = LayoutOptions::default()
        .with_style(style)
        .with_color(INHERITED_COLOR);
    let layout_box = layout(&parsed, &options);
    let display_list = to_display_list(&layout_box);
    let total_height = display_list.height + display_list.depth;
    if !valid_dimension(display_list.width)
        || !valid_dimension(display_list.height)
        || !valid_dimension(display_list.depth)
        || !valid_dimension(total_height)
    {
        return Err(MathError::InvalidSvg(
            "RaTeX returned invalid layout dimensions".into(),
        ));
    }
    if display_list.items.len() > MAX_DISPLAY_ITEMS {
        return Err(MathError::InvalidSvg(format!(
            "formula exceeds the {MAX_DISPLAY_ITEMS}-item render limit"
        )));
    }

    let svg = render_to_svg_with_color_syntax(
        &display_list,
        &SvgOptions {
            font_size: 1.0,
            padding: 0.0,
            stroke_width: 0.04,
            embed_glyphs: true,
            font_dir: String::new(),
        },
        SvgColorSyntax::Rgb,
    );
    if svg.len() > MAX_SVG_BYTES {
        return Err(MathError::InvalidSvg(format!(
            "rendered SVG exceeds the {MAX_SVG_BYTES}-byte limit"
        )));
    }

    let svg = parse_svg(&svg, display_list.width, total_height)?;
    let depth = format_number(display_list.depth);
    let (tag_name, class_name, style) = if display {
        (
            "div",
            "amamo-math amamo-math-display",
            "overflow-x:auto;text-align:center;line-height:0".into(),
        )
    } else {
        (
            "span",
            "amamo-math amamo-math-inline",
            format!("display:inline-block;line-height:0;vertical-align:-{depth}em"),
        )
    };

    Ok(Node::Element(Element {
        tag_name: tag_name.into(),
        properties: vec![
            (
                "className".into(),
                PropertyValue::SpaceSeparated(
                    class_name.split_whitespace().map(str::to_owned).collect(),
                ),
            ),
            ("role".into(), PropertyValue::String("math".into())),
            ("ariaLabel".into(), PropertyValue::String(tex.into())),
            ("style".into(), PropertyValue::String(style)),
        ],
        children: vec![Node::Element(svg)],
        position: None,
    }))
}

fn validate_macros(macros: &HashMap<String, String>) -> Result<(), MathError> {
    let mut macro_bytes = 0usize;
    for (name, expansion) in macros {
        if !valid_macro_name(name) {
            return Err(MathError::Parse(format!(
                "macro name {name:?} must be one TeX control sequence"
            )));
        }
        if expansion.len() > MAX_MACRO_BYTES {
            return Err(MathError::Parse(format!(
                "macro {name:?} exceeds the {MAX_MACRO_BYTES}-byte limit"
            )));
        }
        macro_bytes = macro_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(expansion.len()))
            .ok_or_else(|| MathError::Parse("configured macros are too large".into()))?;
        if macro_bytes > MAX_MACROS_BYTES {
            return Err(MathError::Parse(format!(
                "configured macros exceed the {MAX_MACROS_BYTES}-byte total limit"
            )));
        }
        reject_mutating_commands(expansion)?;
        if contains_parameter_token(expansion) {
            return Err(MathError::Parse(format!(
                "parameterized macro {name:?} is not supported"
            )));
        }
    }
    validate_macro_dependencies(macros)
}

fn validate_macro_dependencies(macros: &HashMap<String, String>) -> Result<(), MathError> {
    let names: HashSet<&str> = macros.keys().map(String::as_str).collect();
    let mut dependencies: HashMap<&str, HashSet<&str>> = macros
        .iter()
        .map(|(name, expansion)| {
            (
                name.as_str(),
                TexTokens::new(expansion)
                    .filter(|token| names.contains(token))
                    .collect(),
            )
        })
        .collect();

    while !dependencies.is_empty() {
        let resolved: Vec<&str> = dependencies
            .iter()
            .filter_map(|(name, dependencies)| dependencies.is_empty().then_some(*name))
            .collect();
        if resolved.is_empty() {
            return Err(MathError::Parse(
                "configured macros contain a dependency cycle".into(),
            ));
        }
        for name in &resolved {
            dependencies.remove(name);
        }
        for remaining in dependencies.values_mut() {
            for name in &resolved {
                remaining.remove(name);
            }
        }
    }
    Ok(())
}

fn parser_with_macros<'a>(tex: &'a str, macros: &HashMap<String, String>) -> Parser<'a> {
    let mut parser = Parser::new(tex);
    for (name, expansion) in macros {
        parser.gullet.set_text_macro(name, expansion);
    }
    parser
}

fn validate_expanded_tokens(tex: &str, macros: &HashMap<String, String>) -> Result<(), MathError> {
    let mut parser = parser_with_macros(tex, macros);
    let mut token_count = 0usize;
    loop {
        let token = parser
            .gullet
            .expand_next_token()
            .map_err(|error| MathError::Parse(error.to_string()))?;
        if token.is_eof() {
            return Ok(());
        }
        token_count += 1;
        if token_count > MAX_EXPANDED_TOKENS {
            return Err(MathError::Parse(format!(
                "formula exceeds the {MAX_EXPANDED_TOKENS}-expanded token limit"
            )));
        }
    }
}

const MUTATING_COMMANDS: &[&str] = &[
    "\\def",
    "\\edef",
    "\\futurelet",
    "\\gdef",
    "\\global",
    "\\let",
    "\\long",
    "\\newcommand",
    "\\providecommand",
    "\\renewcommand",
    "\\xdef",
];

fn reject_mutating_commands(source: &str) -> Result<(), MathError> {
    if let Some(command) = TexTokens::new(source).find(|token| MUTATING_COMMANDS.contains(token)) {
        return Err(MathError::Parse(format!(
            "dynamic macro command {command:?} is not supported"
        )));
    }
    Ok(())
}

fn contains_parameter_token(source: &str) -> bool {
    let mut tokens = TexTokens::new(source).peekable();
    while let Some(token) = tokens.next() {
        if token == "#"
            && tokens.peek().is_some_and(|token| {
                matches!(*token, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        {
            return true;
        }
    }
    false
}

struct TexTokens<'a> {
    offset: usize,
    source: &'a str,
}

impl<'a> TexTokens<'a> {
    const fn new(source: &'a str) -> Self {
        Self { offset: 0, source }
    }
}

impl<'a> Iterator for TexTokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.offset;
        let character = self.source.get(start..)?.chars().next()?;
        self.offset += character.len_utf8();
        if character == '\\' {
            let next = self.source.get(self.offset..)?.chars().next();
            if let Some(next) = next {
                self.offset += next.len_utf8();
                if next.is_ascii_alphabetic() || next == '@' {
                    while let Some(character) = self.source.get(self.offset..)?.chars().next() {
                        if !character.is_ascii_alphabetic() && character != '@' {
                            break;
                        }
                        self.offset += character.len_utf8();
                    }
                }
            }
        }
        self.source.get(start..self.offset)
    }
}

fn valid_macro_name(name: &str) -> bool {
    if name.len() < 2 || name.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    let mut tokens = TexTokens::new(name);
    tokens.next() == Some(name) && name.starts_with('\\') && tokens.next().is_none()
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedBuffer {
    const fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(
                "serialized math AST exceeds its byte limit",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn validate_ast_structure(parsed: &[ParseNode]) -> Result<(), MathError> {
    let mut buffer = LimitedBuffer::new(MAX_AST_BYTES);
    serde_json::to_writer(&mut buffer, parsed).map_err(|_| {
        MathError::Parse(format!(
            "expanded formula exceeds the {MAX_AST_BYTES}-byte AST limit"
        ))
    })?;
    let value: Value = serde_json::from_slice(&buffer.bytes)
        .map_err(|_| MathError::Parse("RaTeX returned an invalid math AST".into()))?;
    let mut stack = vec![&value];
    let mut node_count = 0usize;
    let mut array_cells = 0usize;

    while let Some(value) = stack.pop() {
        match value {
            Value::Array(values) => stack.extend(values),
            Value::Object(object) => {
                if object.contains_key("mode") && object.get("type").is_some_and(Value::is_string) {
                    node_count += 1;
                    if node_count > MAX_AST_NODES {
                        return Err(MathError::Parse(format!(
                            "expanded formula exceeds the {MAX_AST_NODES}-node AST limit"
                        )));
                    }
                }
                if object.get("type").and_then(Value::as_str) == Some("array") {
                    let rows = object
                        .get("body")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            MathError::Parse("RaTeX returned an invalid array AST".into())
                        })?;
                    let max_columns = rows
                        .iter()
                        .map(|row| row.as_array().map_or(0, Vec::len))
                        .max()
                        .unwrap_or(0);
                    let cells = rows.len().saturating_mul(max_columns);
                    array_cells = array_cells.saturating_add(cells);
                    if array_cells > MAX_ARRAY_CELLS {
                        return Err(MathError::Parse(format!(
                            "arrays exceed the {MAX_ARRAY_CELLS}-cell layout limit"
                        )));
                    }
                }
                stack.extend(object.values());
            }
            _ => {}
        }
    }
    Ok(())
}

fn math_source(node: &Node) -> Option<(&str, bool)> {
    let Node::Element(element) = node else {
        return None;
    };
    if element.tag_name == "code" && has_math_classes(element, "math-inline") {
        return only_text(element).map(|tex| (tex, false));
    }
    if element.tag_name != "pre" || !element.properties.is_empty() {
        return None;
    }
    let [Node::Element(code)] = element.children.as_slice() else {
        return None;
    };
    if code.tag_name != "code" || !has_math_classes(code, "math-display") {
        return None;
    }
    only_text(code)
        .and_then(|tex| tex.strip_suffix('\n'))
        .map(|tex| (tex, true))
}

fn has_math_classes(element: &Element, marker: &str) -> bool {
    matches!(
        element.properties.as_slice(),
        [(name, PropertyValue::SpaceSeparated(classes))]
            if name == "className"
                && classes.as_slice() == ["language-math", marker]
    )
}

fn only_text(element: &Element) -> Option<&str> {
    let [Node::Text(text)] = element.children.as_slice() else {
        return None;
    };
    Some(&text.value)
}

fn parse_svg(source: &str, width: f64, height: f64) -> Result<Element, MathError> {
    let document = roxmltree::Document::parse(source)
        .map_err(|error| MathError::InvalidSvg(format!("invalid XML: {error}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(MathError::InvalidSvg(
            "rendered document must have an SVG root".into(),
        ));
    }
    for attribute in root.attributes() {
        if attribute.namespace().is_some()
            || !matches!(attribute.name(), "viewBox" | "width" | "height")
        {
            return Err(unsafe_attribute("svg", attribute.name()));
        }
    }
    let view_box = root
        .attribute("viewBox")
        .filter(|value| valid_number_list(value, 4))
        .ok_or_else(|| MathError::InvalidSvg("SVG root has an invalid viewBox".into()))?;

    let mut node_count = 1;
    let children = root
        .children()
        .map(|child| convert_svg_child(child, &mut node_count))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();

    Ok(Element {
        tag_name: "svg".into(),
        properties: vec![
            ("xmlns".into(), PropertyValue::String(SVG_NAMESPACE.into())),
            ("viewBox".into(), PropertyValue::String(view_box.into())),
            (
                "width".into(),
                PropertyValue::String(format!("{}em", format_number(width))),
            ),
            (
                "height".into(),
                PropertyValue::String(format!("{}em", format_number(height))),
            ),
            ("ariaHidden".into(), PropertyValue::String("true".into())),
            ("focusable".into(), PropertyValue::String("false".into())),
        ],
        children,
        position: None,
    })
}

fn convert_svg_child(
    node: roxmltree::Node<'_, '_>,
    node_count: &mut usize,
) -> Result<Option<Node>, MathError> {
    if node.is_text() {
        return if node.text().is_none_or(|text| text.trim().is_empty()) {
            Ok(None)
        } else {
            Err(MathError::InvalidSvg(
                "SVG contains text instead of embedded glyph paths".into(),
            ))
        };
    }
    if !node.is_element() {
        return Err(MathError::InvalidSvg(
            "SVG contains comments or processing instructions".into(),
        ));
    }
    *node_count += 1;
    if *node_count > MAX_SVG_NODES {
        return Err(MathError::InvalidSvg(format!(
            "SVG exceeds the {MAX_SVG_NODES}-node limit"
        )));
    }
    if node.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(MathError::InvalidSvg(
            "SVG contains an element from another namespace".into(),
        ));
    }

    let tag_name = node.tag_name().name();
    if !matches!(tag_name, "path" | "line" | "rect" | "image") {
        return Err(MathError::InvalidSvg(format!(
            "SVG element <{tag_name}> is not allowed"
        )));
    }
    let inherited_fill = inherited_paint(&node, "fill", "fill-opacity");
    let inherited_stroke = inherited_paint(&node, "stroke", "stroke-opacity");
    let inherited_opacity =
        tag_name == "image" && node.attribute("opacity") == Some(INHERITED_OPACITY);
    let properties = node
        .attributes()
        .map(|attribute| {
            convert_svg_attribute(
                tag_name,
                attribute,
                inherited_fill,
                inherited_stroke,
                inherited_opacity,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let children = node
        .children()
        .map(|child| convert_svg_child(child, node_count))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();

    Ok(Some(Node::Element(Element {
        tag_name: tag_name.into(),
        properties,
        children,
        position: None,
    })))
}

fn convert_svg_attribute(
    tag_name: &str,
    attribute: roxmltree::Attribute<'_, '_>,
    inherited_fill: bool,
    inherited_stroke: bool,
    inherited_opacity: bool,
) -> Result<Option<(String, PropertyValue)>, MathError> {
    let name = attribute.name();
    let value = attribute.value();
    if attribute.namespace().is_some()
        || name.to_ascii_lowercase().starts_with("on")
        || !allowed_attribute(tag_name, name)
    {
        return Err(unsafe_attribute(tag_name, name));
    }
    if name == "href" && !valid_embedded_png(value) {
        return Err(MathError::InvalidSvg(
            "SVG image must use an embedded PNG data URL".into(),
        ));
    }
    validate_attribute_value(name, value)?;

    if (name == "fill-opacity" && inherited_fill)
        || (name == "stroke-opacity" && inherited_stroke)
        || (name == "opacity" && inherited_opacity)
    {
        return Ok(None);
    }
    let value = if (name == "fill" && inherited_fill) || (name == "stroke" && inherited_stroke) {
        "currentColor"
    } else {
        value
    };
    Ok(Some((
        hast_attribute_name(name).into(),
        PropertyValue::String(value.into()),
    )))
}

fn inherited_paint(node: &roxmltree::Node<'_, '_>, paint: &str, opacity: &str) -> bool {
    node.attribute(paint) == Some(INHERITED_PAINT)
        && node.attribute(opacity) == Some(INHERITED_OPACITY)
}

fn allowed_attribute(tag_name: &str, name: &str) -> bool {
    match tag_name {
        "path" => matches!(
            name,
            "d" | "fill"
                | "fill-opacity"
                | "fill-rule"
                | "stroke"
                | "stroke-opacity"
                | "stroke-width"
                | "stroke-linecap"
                | "stroke-linejoin"
        ),
        "line" => matches!(
            name,
            "x1" | "y1"
                | "x2"
                | "y2"
                | "stroke"
                | "stroke-opacity"
                | "stroke-width"
                | "stroke-dasharray"
        ),
        "rect" => matches!(
            name,
            "x" | "y" | "width" | "height" | "fill" | "fill-opacity"
        ),
        "image" => matches!(
            name,
            "href" | "x" | "y" | "width" | "height" | "opacity" | "preserveAspectRatio"
        ),
        _ => false,
    }
}

fn validate_attribute_value(name: &str, value: &str) -> Result<(), MathError> {
    let valid = match name {
        "fill" | "stroke" => value == "none" || valid_rgb(value),
        "fill-opacity" | "stroke-opacity" | "opacity" => value
            .parse::<f64>()
            .is_ok_and(|number| number.is_finite() && (0.0..=1.0).contains(&number)),
        "d" => valid_path(value),
        "fill-rule" => value == "nonzero",
        "stroke-linecap" | "stroke-linejoin" => value == "round",
        "preserveAspectRatio" => value == "none",
        "stroke-dasharray" => valid_size_list(value, 2),
        "href" => true,
        "height" | "stroke-width" | "width" => value.parse::<f64>().is_ok_and(valid_dimension),
        _ => valid_coordinate(value),
    };
    if valid {
        Ok(())
    } else {
        Err(MathError::InvalidSvg(format!(
            "SVG attribute {name:?} has an invalid value"
        )))
    }
}

fn valid_rgb(value: &str) -> bool {
    value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
        .is_some_and(|channels| {
            let mut count = 0;
            let valid = channels.split(',').all(|channel| {
                count += 1;
                channel.parse::<u8>().is_ok()
            });
            valid && count == 3
        })
}

fn valid_number_list(value: &str, expected: usize) -> bool {
    let mut count = 0;
    let valid = value.split_ascii_whitespace().all(|number| {
        count += 1;
        valid_coordinate(number)
    });
    valid && count == expected
}

fn valid_size_list(value: &str, expected: usize) -> bool {
    let mut count = 0;
    let valid = value.split_ascii_whitespace().all(|number| {
        count += 1;
        number.parse::<f64>().is_ok_and(valid_dimension)
    });
    valid && count == expected
}

fn valid_path(value: &str) -> bool {
    let mut remaining = 0;
    for token in value.split_ascii_whitespace() {
        if token == "Z" {
            if remaining != 0 {
                return false;
            }
            continue;
        }

        let bytes = token.as_bytes();
        let number = if let Some(command) = bytes.first() {
            let coordinates = match command {
                b'M' | b'L' => 2,
                b'Q' => 4,
                b'C' => 6,
                _ => 0,
            };
            if coordinates > 0 {
                if remaining != 0 {
                    return false;
                }
                remaining = coordinates;
                &token[1..]
            } else {
                token
            }
        } else {
            return false;
        };
        if remaining == 0 || !valid_coordinate(number) {
            return false;
        }
        remaining -= 1;
    }
    remaining == 0
}

fn valid_coordinate(value: &str) -> bool {
    value
        .parse::<f64>()
        .is_ok_and(|number| number.is_finite() && number.abs() <= MAX_DIMENSION_EM)
}

fn valid_embedded_png(value: &str) -> bool {
    value
        .strip_prefix("data:image/png;base64,")
        .is_some_and(|payload| {
            !payload.is_empty()
                && payload
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        })
}

fn hast_attribute_name(name: &str) -> &str {
    match name {
        "fill-opacity" => "fillOpacity",
        "fill-rule" => "fillRule",
        "stroke-dasharray" => "strokeDashArray",
        "stroke-linecap" => "strokeLineCap",
        "stroke-linejoin" => "strokeLineJoin",
        "stroke-opacity" => "strokeOpacity",
        "stroke-width" => "strokeWidth",
        name => name,
    }
}

fn unsafe_attribute(tag_name: &str, attribute: &str) -> MathError {
    MathError::InvalidSvg(format!(
        "SVG attribute {attribute:?} is not allowed on <{tag_name}>"
    ))
}

fn valid_dimension(value: f64) -> bool {
    value.is_finite() && (0.0..=MAX_DIMENSION_EM).contains(&value)
}

fn format_number(value: f64) -> String {
    let value = format!("{value:.6}");
    let value = value.trim_end_matches('0').trim_end_matches('.');
    if value.is_empty() || value == "-0" {
        "0".into()
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mdxjs::hast::{Node, PropertyValue};

    use super::{
        MAX_DIMENSION_EM, MAX_MACRO_BYTES, MAX_TEX_BYTES, MathError, parse_svg, render_math,
        valid_dimension, valid_number_list, valid_rgb,
    };

    #[test]
    fn renders_inline_math_as_accessible_self_contained_svg() {
        let node = render_math("x_2", false, &HashMap::new()).unwrap();
        let Node::Element(wrapper) = node else {
            panic!("math must render as an element");
        };
        assert_eq!(wrapper.tag_name, "span");
        assert!(has_property(&wrapper, "role", "math"));
        assert!(has_property(&wrapper, "ariaLabel", "x_2"));
        assert!(
            property(&wrapper, "style").is_some_and(|style| style.contains("vertical-align:-"))
        );

        let Some(Node::Element(svg)) = wrapper.children.first() else {
            panic!("math wrapper must contain an SVG");
        };
        assert_eq!(svg.tag_name, "svg");
        assert!(has_property(svg, "ariaHidden", "true"));
        assert!(has_property(svg, "focusable", "false"));
        assert!(contains_property_value(
            &Node::Element(svg.clone()),
            "currentColor"
        ));
        assert!(!contains_tag(&Node::Element(svg.clone()), "text"));
    }

    #[test]
    fn preserves_explicit_tex_color_that_matches_the_inherited_rgb() {
        let node = render_math(r"\color{#01020301}x", true, &HashMap::new()).unwrap();

        assert!(contains_property_value(&node, "rgb(1,2,3)"));
        assert!(contains_property_value(&node, "0.003922"));
        assert!(!contains_property_value(&node, "currentColor"));
    }

    #[test]
    fn restores_inherited_opacity_for_embedded_images() {
        let svg = parse_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><image href="data:image/png;base64,AA==" x="0" y="0" width="1" height="1" opacity="0.123456" preserveAspectRatio="none"/></svg>"#,
            1.0,
            1.0,
        )
        .unwrap();
        let Some(Node::Element(image)) = svg.children.first() else {
            panic!("SVG must contain an image");
        };

        assert!(property(image, "opacity").is_none());
    }

    #[test]
    fn configured_macros_are_local_to_one_render() {
        let macros = HashMap::from([(r"\RR".into(), r"\mathbb{R}".into())]);
        render_math(r"\RR", false, &macros).unwrap();

        assert!(matches!(
            render_math(r"\RR", false, &HashMap::new()),
            Err(MathError::Parse(_))
        ));
    }

    #[test]
    fn rejects_dynamic_oversized_or_amplified_macros_before_layout() {
        for source in [r"\def\A{x}\A", r"\newcommand{\A}{x}\A", r"\let\A\alpha\A"] {
            assert!(matches!(
                render_math(source, false, &HashMap::new()),
                Err(MathError::Parse(_))
            ));
        }
        assert!(render_math(r"\\def", false, &HashMap::new()).is_ok());
        assert!(matches!(
            render_math(r"\url{x%y}\def\A{x}\A", false, &HashMap::new()),
            Err(MathError::Parse(_))
        ));

        let oversized = HashMap::from([(r"\A".into(), "x".repeat(MAX_MACRO_BYTES + 1))]);
        assert!(matches!(
            render_math(r"\A", false, &oversized),
            Err(MathError::Parse(_))
        ));
        let parameterized = HashMap::from([(r"\dup".into(), "#1#1".into())]);
        assert!(matches!(
            render_math(r"\dup{x}", false, &parameterized),
            Err(MathError::Parse(_))
        ));
        let color = HashMap::from([(r"\red".into(), r"\color{#f00}".into())]);
        assert!(render_math(r"\red x", false, &color).is_ok());

        let amplified = HashMap::from([(r"\many".into(), "x".repeat(MAX_MACRO_BYTES))]);
        let error = render_math(&r"\many".repeat(101), false, &amplified).unwrap_err();
        assert!(matches!(
            error,
            MathError::Parse(message) if message.contains("expanded token")
        ));

        let cyclic = HashMap::from([(r"\cycle".into(), r"\cycle\cycle".into())]);
        let error = render_math(r"\cycle", false, &cyclic).unwrap_err();
        assert!(matches!(
            error,
            MathError::Parse(message) if message.contains("dependency cycle")
        ));
    }

    #[test]
    fn rejects_ragged_arrays_that_cumulatively_exceed_the_layout_cell_budget() {
        let columns = 501;
        let rows = 101;
        let alignment = "c".repeat(columns);
        let first_row = std::iter::repeat_n("x", columns)
            .collect::<Vec<_>>()
            .join("&");
        let empty_rows = r"\\".repeat(rows - 1);
        let array = format!(r"\begin{{array}}{{{alignment}}}{first_row}{empty_rows}\end{{array}}");
        let source = format!("{array}{array}");

        assert!(matches!(
            render_math(&source, true, &HashMap::new()),
            Err(MathError::Parse(_))
        ));
    }

    #[test]
    fn rejects_active_or_external_svg_content() {
        for source in [
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><script/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><text>x</text></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><image href="https://example.com/x.png" x="0" y="0" width="1" height="1" preserveAspectRatio="none"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><path d="M0 0Z" fill="rgb(0,0,0)" onload="alert(1)"/></svg>"#,
        ] {
            assert!(matches!(
                parse_svg(source, 1.0, 1.0),
                Err(MathError::InvalidSvg(_))
            ));
        }
    }

    #[test]
    fn rejects_oversized_formulas_and_malformed_numeric_attributes() {
        let oversized = "x".repeat(MAX_TEX_BYTES + 1);

        assert!(matches!(
            render_math(&oversized, false, &HashMap::new()),
            Err(MathError::Parse(_))
        ));
        assert!(!valid_rgb("rgb(1,2)"));
        assert!(!valid_number_list("0 0 1", 4));
        assert!(!valid_dimension(MAX_DIMENSION_EM + 1.0));
        assert!(matches!(
            render_math(r"\kern100000em x", false, &HashMap::new()),
            Err(MathError::InvalidSvg(_))
        ));
        assert!(matches!(
            render_math(r"\mathrlap{\rule{100000em}{1em}}x", false, &HashMap::new()),
            Err(MathError::InvalidSvg(_))
        ));
        assert!(matches!(
            parse_svg(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><path d="M10001 0 Z" fill="rgb(0,0,0)"/></svg>"#,
                1.0,
                1.0,
            ),
            Err(MathError::InvalidSvg(_))
        ));
    }

    fn property<'a>(element: &'a mdxjs::hast::Element, name: &str) -> Option<&'a str> {
        element
            .properties
            .iter()
            .find(|(property, _)| property == name)
            .and_then(|(_, value)| match value {
                PropertyValue::String(value) => Some(value.as_str()),
                _ => None,
            })
    }

    fn has_property(element: &mdxjs::hast::Element, name: &str, expected: &str) -> bool {
        property(element, name) == Some(expected)
    }

    fn contains_tag(node: &Node, expected: &str) -> bool {
        match node {
            Node::Element(element) => {
                element.tag_name == expected
                    || element
                        .children
                        .iter()
                        .any(|child| contains_tag(child, expected))
            }
            _ => false,
        }
    }

    fn contains_property_value(node: &Node, expected: &str) -> bool {
        match node {
            Node::Element(element) => {
                element.properties.iter().any(
                    |(_, value)| matches!(value, PropertyValue::String(value) if value == expected),
                ) || element
                    .children
                    .iter()
                    .any(|child| contains_property_value(child, expected))
            }
            _ => false,
        }
    }
}
