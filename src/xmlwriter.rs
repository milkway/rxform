//! Minimal XML tree + pretty printer matching pyxform's output conventions:
//! 2-space indentation, alphabetically sorted attributes, self-closing tags
//! for childless elements without text, and `<tag></tag>` for empty text.

/// Text content of an element. `Raw` fragments are emitted verbatim and must
/// already be escaped (used for labels containing `<output .../>` elements).
#[derive(Debug, Clone)]
pub enum Text {
    Plain(String),
    Raw(String),
}

#[derive(Debug, Clone)]
pub struct XmlNode {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    pub text: Option<Text>,
}

impl XmlNode {
    pub fn new(tag: &str) -> Self {
        XmlNode {
            tag: tag.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
            text: None,
        }
    }

    pub fn attr(mut self, name: &str, value: &str) -> Self {
        self.attrs.push((name.to_string(), value.to_string()));
        self
    }

    pub fn text(mut self, text: &str) -> Self {
        self.text = Some(Text::Plain(text.to_string()));
        self
    }

    pub fn raw_text(mut self, text: &str) -> Self {
        self.text = Some(Text::Raw(text.to_string()));
        self
    }

    pub fn child(mut self, node: XmlNode) -> Self {
        self.children.push(node);
        self
    }

    pub fn push(&mut self, node: XmlNode) {
        self.children.push(node);
    }

    /// Render the tree as a standalone document.
    pub fn to_document(&self) -> String {
        let mut out = String::from("<?xml version=\"1.0\"?>\n");
        self.render(&mut out, 0);
        out
    }

    fn render(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        out.push_str(&indent);
        out.push('<');
        out.push_str(&self.tag);
        let mut attrs = self.attrs.clone();
        attrs.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, value) in &attrs {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            out.push_str(&escape_attr(value));
            out.push('"');
        }
        match (&self.text, self.children.is_empty()) {
            (None, true) => out.push_str("/>\n"),
            (Some(text), true) => {
                out.push('>');
                match text {
                    Text::Plain(t) => out.push_str(&escape_text(t)),
                    Text::Raw(t) => out.push_str(t),
                }
                out.push_str("</");
                out.push_str(&self.tag);
                out.push_str(">\n");
            }
            (text, false) => {
                out.push_str(">\n");
                if let Some(text) = text {
                    out.push_str(&"  ".repeat(depth + 1));
                    match text {
                        Text::Plain(t) => out.push_str(&escape_text(t)),
                        Text::Raw(t) => out.push_str(t),
                    }
                    out.push('\n');
                }
                for child in &self.children {
                    child.render(out, depth + 1);
                }
                out.push_str(&indent);
                out.push_str("</");
                out.push_str(&self.tag);
                out.push_str(">\n");
            }
        }
    }
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sorted_attrs_and_nesting() {
        let node = XmlNode::new("bind")
            .attr("type", "string")
            .attr("nodeset", "/f/x")
            .attr("calculate", "1");
        assert_eq!(
            node.to_document(),
            "<?xml version=\"1.0\"?>\n<bind calculate=\"1\" nodeset=\"/f/x\" type=\"string\"/>\n"
        );
    }

    #[test]
    fn renders_empty_text_as_paired_tag() {
        let node = XmlNode::new("label").text("");
        assert_eq!(
            node.to_document(),
            "<?xml version=\"1.0\"?>\n<label></label>\n"
        );
    }

    #[test]
    fn escapes_text_and_attrs() {
        let node = XmlNode::new("label").attr("a", "x\"<y").text("a<b & c");
        assert_eq!(
            node.to_document(),
            "<?xml version=\"1.0\"?>\n<label a=\"x&quot;&lt;y\">a&lt;b &amp; c</label>\n"
        );
    }
}
