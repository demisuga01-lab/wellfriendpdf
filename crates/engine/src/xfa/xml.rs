use std::collections::{BTreeMap, BTreeSet};

use super::{XfaDiagnostic, XfaLimits};
use crate::error::{OxideError, Result};

#[derive(Debug, Clone)]
pub(crate) struct XmlAttribute {
    pub name: String,
    pub local_name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub(crate) struct XmlNode {
    pub name: String,
    pub local_name: String,
    pub namespace_uri: Option<String>,
    pub attributes: Vec<XmlAttribute>,
    pub text: String,
    pub children: Vec<XmlNode>,
    pub start_offset: usize,
    pub end_offset: usize,
}

impl XmlNode {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attr| attr.name == name || attr.local_name == name)
            .map(|attr| attr.value.as_str())
    }

    pub fn child(&self, local_name: &str) -> Option<&XmlNode> {
        self.children
            .iter()
            .find(|child| child.local_name == local_name)
    }

    pub fn descendants<'a>(&'a self, local_name: &'a str) -> impl Iterator<Item = &'a XmlNode> {
        Descendants {
            stack: vec![self],
            local_name,
        }
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        collect_text(self, &mut out);
        out.trim().to_string()
    }
}

struct Descendants<'a> {
    stack: Vec<&'a XmlNode>,
    local_name: &'a str,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = &'a XmlNode;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(node) = self.stack.pop() {
            self.stack.extend(node.children.iter().rev());
            if node.local_name == self.local_name {
                return Some(node);
            }
        }
        None
    }
}

fn collect_text(node: &XmlNode, out: &mut String) {
    if !node.text.trim().is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(node.text.trim());
    }
    for child in &node.children {
        collect_text(child, out);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct XmlMetrics {
    pub nodes: usize,
    pub attributes: usize,
    pub namespace_declarations: usize,
    pub max_depth: usize,
    pub text_bytes: usize,
    pub entity_references: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedXml {
    pub root: XmlNode,
    pub metrics: XmlMetrics,
    pub diagnostics: Vec<XfaDiagnostic>,
}

struct OpenNode {
    node: XmlNode,
    namespaces: BTreeMap<String, String>,
}

pub(crate) fn parse_xml(bytes: &[u8], limits: &XfaLimits) -> Result<ParsedXml> {
    if bytes.len() > limits.max_xml_bytes {
        return Err(resource_limit(format!(
            "XFA XML bytes {} exceed cap {}",
            bytes.len(),
            limits.max_xml_bytes
        )));
    }
    let input = std::str::from_utf8(bytes)
        .map_err(|err| OxideError::MalformedPdf(format!("XFA XML is not valid UTF-8: {err}")))?;
    let lower = input.to_ascii_lowercase();
    if lower.contains("<!doctype") || lower.contains("<!entity") {
        return Err(OxideError::MalformedPdf(
            "XFA XML DTD/entity declarations are forbidden".to_string(),
        ));
    }

    let mut metrics = XmlMetrics::default();
    let mut diagnostics = Vec::new();
    let mut stack = Vec::<OpenNode>::new();
    let mut root = None;
    let mut pos = 0usize;

    while pos < input.len() {
        if input.as_bytes()[pos] != b'<' {
            let end = input[pos..]
                .find('<')
                .map(|offset| pos + offset)
                .unwrap_or(input.len());
            let raw = &input[pos..end];
            if !raw.trim().is_empty() {
                let text = decode_entities(raw, limits, &mut metrics)?;
                if text.len() > limits.max_text_node_bytes {
                    return Err(resource_limit(format!(
                        "XFA XML text node exceeds cap {}",
                        limits.max_text_node_bytes
                    )));
                }
                metrics.text_bytes = metrics.text_bytes.saturating_add(text.len());
                if let Some(current) = stack.last_mut() {
                    current.node.text.push_str(&text);
                } else {
                    return Err(OxideError::MalformedPdf(
                        "XFA XML has non-whitespace text outside the root".to_string(),
                    ));
                }
            }
            pos = end;
            continue;
        }

        if input[pos..].starts_with("<!--") {
            let end = input[pos + 4..]
                .find("-->")
                .map(|offset| pos + 4 + offset + 3)
                .ok_or_else(|| OxideError::MalformedPdf("unterminated XFA XML comment".into()))?;
            pos = end;
            continue;
        }
        if input[pos..].starts_with("<?") {
            let end = input[pos + 2..]
                .find("?>")
                .map(|offset| pos + 2 + offset + 2)
                .ok_or_else(|| {
                    OxideError::MalformedPdf("unterminated XFA XML processing instruction".into())
                })?;
            pos = end;
            continue;
        }
        if input[pos..].starts_with("<![CDATA[") {
            let end_start = input[pos + 9..]
                .find("]]>")
                .map(|offset| pos + 9 + offset)
                .ok_or_else(|| OxideError::MalformedPdf("unterminated XFA XML CDATA".into()))?;
            let text = &input[pos + 9..end_start];
            if text.len() > limits.max_text_node_bytes {
                return Err(resource_limit(format!(
                    "XFA XML CDATA exceeds cap {}",
                    limits.max_text_node_bytes
                )));
            }
            metrics.text_bytes = metrics.text_bytes.saturating_add(text.len());
            let current = stack.last_mut().ok_or_else(|| {
                OxideError::MalformedPdf("XFA XML CDATA appears outside the root".into())
            })?;
            current.node.text.push_str(text);
            pos = end_start + 3;
            continue;
        }
        if input[pos..].starts_with("<!") {
            return Err(OxideError::MalformedPdf(
                "unsupported XFA XML declaration; DTD/entity expansion is disabled".to_string(),
            ));
        }
        if input[pos..].starts_with("</") {
            let close = input[pos + 2..]
                .find('>')
                .map(|offset| pos + 2 + offset)
                .ok_or_else(|| OxideError::MalformedPdf("unterminated XFA XML end tag".into()))?;
            let name = input[pos + 2..close].trim();
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return Err(OxideError::MalformedPdf("invalid XFA XML end tag".into()));
            }
            let mut finished = stack.pop().ok_or_else(|| {
                OxideError::MalformedPdf(format!("unexpected XFA XML end tag </{name}>"))
            })?;
            if finished.node.name != name {
                return Err(OxideError::MalformedPdf(format!(
                    "mismatched XFA XML end tag: expected </{}>, found </{name}>",
                    finished.node.name
                )));
            }
            finished.node.end_offset = close + 1;
            attach_finished(finished.node, &mut stack, &mut root)?;
            pos = close + 1;
            continue;
        }

        let (name, attributes, self_closing, end) =
            parse_start_tag(input, pos, limits, &mut metrics)?;
        metrics.nodes = metrics.nodes.saturating_add(1);
        if metrics.nodes > limits.max_xml_nodes {
            return Err(resource_limit(format!(
                "XFA XML node count exceeds cap {}",
                limits.max_xml_nodes
            )));
        }
        let depth = stack.len() + 1;
        metrics.max_depth = metrics.max_depth.max(depth);
        if depth > limits.max_xml_depth {
            return Err(resource_limit(format!(
                "XFA XML depth exceeds cap {}",
                limits.max_xml_depth
            )));
        }

        let mut namespaces = stack
            .last()
            .map(|open| open.namespaces.clone())
            .unwrap_or_default();
        for attr in &attributes {
            if attr.name == "xmlns" {
                namespaces.insert(String::new(), attr.value.clone());
            } else if let Some(prefix) = attr.name.strip_prefix("xmlns:") {
                namespaces.insert(prefix.to_string(), attr.value.clone());
            }
        }
        let prefix = name.split_once(':').map(|(prefix, _)| prefix).unwrap_or("");
        let namespace_uri = namespaces.get(prefix).cloned();
        let node = XmlNode {
            local_name: local_name(&name).to_string(),
            name,
            namespace_uri,
            attributes,
            text: String::new(),
            children: Vec::new(),
            start_offset: pos,
            end_offset: end,
        };
        if self_closing {
            attach_finished(node, &mut stack, &mut root)?;
        } else {
            stack.push(OpenNode { node, namespaces });
        }
        pos = end;
    }

    if let Some(open) = stack.last() {
        return Err(OxideError::MalformedPdf(format!(
            "unterminated XFA XML element <{}>",
            open.node.name
        )));
    }
    let root =
        root.ok_or_else(|| OxideError::MalformedPdf("XFA XML has no root element".into()))?;
    diagnostics.push(XfaDiagnostic::info(
        "xfa.xml.external_access_disabled",
        "external entities, DTD retrieval, network, and filesystem access are disabled",
        None,
    ));
    Ok(ParsedXml {
        root,
        metrics,
        diagnostics,
    })
}

fn parse_start_tag(
    input: &str,
    start: usize,
    limits: &XfaLimits,
    metrics: &mut XmlMetrics,
) -> Result<(String, Vec<XmlAttribute>, bool, usize)> {
    let bytes = input.as_bytes();
    let mut pos = start + 1;
    skip_ws(bytes, &mut pos);
    let name_start = pos;
    while pos < bytes.len() && !is_name_delimiter(bytes[pos]) {
        pos += 1;
    }
    if pos == name_start {
        return Err(OxideError::MalformedPdf("invalid XFA XML start tag".into()));
    }
    let name = input[name_start..pos].to_string();
    let mut attributes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut self_closing = false;

    loop {
        skip_ws(bytes, &mut pos);
        if pos >= bytes.len() {
            return Err(OxideError::MalformedPdf(
                "unterminated XFA XML start tag".into(),
            ));
        }
        if bytes[pos] == b'>' {
            pos += 1;
            break;
        }
        if bytes[pos] == b'/' && bytes.get(pos + 1) == Some(&b'>') {
            self_closing = true;
            pos += 2;
            break;
        }
        let attr_start = pos;
        while pos < bytes.len() && !is_attr_name_delimiter(bytes[pos]) {
            pos += 1;
        }
        if pos == attr_start {
            return Err(OxideError::MalformedPdf(format!(
                "invalid attribute in XFA XML element <{name}>"
            )));
        }
        let attr_name = input[attr_start..pos].to_string();
        if !seen.insert(attr_name.clone()) {
            return Err(OxideError::MalformedPdf(format!(
                "duplicate XFA XML attribute {attr_name}"
            )));
        }
        skip_ws(bytes, &mut pos);
        if bytes.get(pos) != Some(&b'=') {
            return Err(OxideError::MalformedPdf(format!(
                "XFA XML attribute {attr_name} has no value"
            )));
        }
        pos += 1;
        skip_ws(bytes, &mut pos);
        let quote = *bytes.get(pos).ok_or_else(|| {
            OxideError::MalformedPdf(format!("XFA XML attribute {attr_name} has no quote"))
        })?;
        if quote != b'\'' && quote != b'"' {
            return Err(OxideError::MalformedPdf(format!(
                "XFA XML attribute {attr_name} must be quoted"
            )));
        }
        pos += 1;
        let value_start = pos;
        while pos < bytes.len() && bytes[pos] != quote {
            pos += 1;
        }
        if pos >= bytes.len() {
            return Err(OxideError::MalformedPdf(format!(
                "unterminated XFA XML attribute {attr_name}"
            )));
        }
        let value = decode_entities(&input[value_start..pos], limits, metrics)?;
        if value.len() > limits.max_xml_attribute_value_bytes {
            return Err(resource_limit(format!(
                "XFA XML attribute value exceeds cap {}",
                limits.max_xml_attribute_value_bytes
            )));
        }
        pos += 1;
        metrics.attributes = metrics.attributes.saturating_add(1);
        if metrics.attributes > limits.max_xml_attributes {
            return Err(resource_limit(format!(
                "XFA XML attribute count exceeds cap {}",
                limits.max_xml_attributes
            )));
        }
        if attr_name == "xmlns" || attr_name.starts_with("xmlns:") {
            metrics.namespace_declarations = metrics.namespace_declarations.saturating_add(1);
            if metrics.namespace_declarations > limits.max_namespace_declarations {
                return Err(resource_limit(format!(
                    "XFA XML namespace declaration count exceeds cap {}",
                    limits.max_namespace_declarations
                )));
            }
        }
        attributes.push(XmlAttribute {
            local_name: local_name(&attr_name).to_string(),
            name: attr_name,
            value,
        });
    }
    Ok((name, attributes, self_closing, pos))
}

fn attach_finished(
    node: XmlNode,
    stack: &mut [OpenNode],
    root: &mut Option<XmlNode>,
) -> Result<()> {
    if let Some(parent) = stack.last_mut() {
        parent.node.children.push(node);
    } else if root.replace(node).is_some() {
        return Err(OxideError::MalformedPdf(
            "XFA XML contains multiple root elements".to_string(),
        ));
    }
    Ok(())
}

fn decode_entities(raw: &str, limits: &XfaLimits, metrics: &mut XmlMetrics) -> Result<String> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0usize;
    while let Some(relative) = raw[pos..].find('&') {
        let amp = pos + relative;
        out.push_str(&raw[pos..amp]);
        let semicolon = raw[amp + 1..]
            .find(';')
            .map(|offset| amp + 1 + offset)
            .ok_or_else(|| {
                OxideError::MalformedPdf("unterminated XFA XML entity reference".into())
            })?;
        metrics.entity_references = metrics.entity_references.saturating_add(1);
        if metrics.entity_references > limits.max_entity_references {
            return Err(resource_limit(format!(
                "XFA XML entity/reference count exceeds cap {}",
                limits.max_entity_references
            )));
        }
        let entity = &raw[amp + 1..semicolon];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "apos" => out.push('\''),
            "quot" => out.push('"'),
            numeric if numeric.starts_with("#x") => {
                push_numeric_entity(&mut out, &numeric[2..], 16)?
            }
            numeric if numeric.starts_with('#') => {
                push_numeric_entity(&mut out, &numeric[1..], 10)?
            }
            _ => {
                return Err(OxideError::MalformedPdf(format!(
                    "unsupported XFA XML entity reference &{entity};"
                )))
            }
        }
        pos = semicolon + 1;
    }
    out.push_str(&raw[pos..]);
    Ok(out)
}

fn push_numeric_entity(out: &mut String, digits: &str, radix: u32) -> Result<()> {
    let value = u32::from_str_radix(digits, radix)
        .map_err(|_| OxideError::MalformedPdf("invalid XFA XML numeric entity".into()))?;
    let ch = char::from_u32(value)
        .filter(|ch| !matches!(*ch as u32, 0xD800..=0xDFFF))
        .ok_or_else(|| OxideError::MalformedPdf("invalid XFA XML code point".into()))?;
    out.push(ch);
    Ok(())
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while bytes.get(*pos).is_some_and(u8::is_ascii_whitespace) {
        *pos += 1;
    }
}

fn is_name_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>' | b'=' | b'<')
}

fn is_attr_name_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'=' | b'/' | b'>' | b'<')
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
}

fn resource_limit(message: String) -> OxideError {
    OxideError::ResourceLimit(message)
}

pub(crate) fn serialize_sanitized(
    root: &XmlNode,
    remove_scripts_events: bool,
    remove_connections: bool,
    remove_external_references: bool,
) -> Vec<u8> {
    let mut out = String::new();
    write_node(
        root,
        &mut out,
        remove_scripts_events,
        remove_connections,
        remove_external_references,
    );
    out.into_bytes()
}

fn write_node(
    node: &XmlNode,
    out: &mut String,
    remove_scripts_events: bool,
    remove_connections: bool,
    remove_external_references: bool,
) {
    if remove_scripts_events
        && matches!(
            node.local_name.as_str(),
            "script" | "event" | "calculate" | "validate"
        )
    {
        return;
    }
    if remove_connections
        && matches!(
            node.local_name.as_str(),
            "connectionSet" | "sourceSet" | "connect"
        )
    {
        return;
    }
    out.push('<');
    out.push_str(&node.name);
    for attr in &node.attributes {
        if remove_external_references
            && matches!(
                attr.local_name.as_str(),
                "href" | "uri" | "url" | "connection" | "target"
            )
            && is_external_reference(&attr.value)
        {
            continue;
        }
        out.push(' ');
        out.push_str(&attr.name);
        out.push_str("=\"");
        escape_xml(&attr.value, out);
        out.push('"');
    }
    if node.children.is_empty() && node.text.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    escape_xml(&node.text, out);
    for child in &node.children {
        write_node(
            child,
            out,
            remove_scripts_events,
            remove_connections,
            remove_external_references,
        );
    }
    out.push_str("</");
    out.push_str(&node.name);
    out.push('>');
}

fn escape_xml(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

pub(crate) fn is_external_reference(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("http:")
        || lower.starts_with("https:")
        || lower.starts_with("ftp:")
        || lower.starts_with("file:")
        || lower.starts_with("\\\\")
        || lower.starts_with("//")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_parser_resolves_namespaces_and_entities() {
        let parsed = parse_xml(
            br#"<xdp:xdp xmlns:xdp="urn:xdp"><template><field name="a&amp;b"/></template></xdp:xdp>"#,
            &XfaLimits::default(),
        )
        .unwrap();
        assert_eq!(parsed.root.local_name, "xdp");
        assert_eq!(parsed.root.namespace_uri.as_deref(), Some("urn:xdp"));
        assert_eq!(
            parsed.root.children[0].children[0].attr("name"),
            Some("a&b")
        );
    }

    #[test]
    fn dtd_and_depth_bombs_fail_closed() {
        assert!(parse_xml(
            br#"<!DOCTYPE x [<!ENTITY y SYSTEM "file:///etc/passwd">]><x>&y;</x>"#,
            &XfaLimits::default(),
        )
        .is_err());
        let limits = XfaLimits {
            max_xml_depth: 2,
            ..XfaLimits::default()
        };
        assert!(parse_xml(b"<a><b><c/></b></a>", &limits).is_err());
    }
}
