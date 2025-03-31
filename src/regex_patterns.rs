use regex;

pub struct RegexPatterns {
    pub p_tag: regex::Regex,
    pub h_open: regex::Regex,
    pub h_close: regex::Regex,
    pub remaining_tags: regex::Regex,
    pub multi_space: regex::Regex,
    pub multi_newline: regex::Regex,
    pub leading_space: regex::Regex,
    pub line_leading_space: regex::Regex,
    pub empty_lines: regex::Regex,
    pub italic: regex::Regex,
    pub css_rule: regex::Regex,
}

impl RegexPatterns {
    pub fn new() -> Self {
        let p_tag = regex::Regex::new(r"<p[^>]*>")
            .expect("Failed to compile paragraph tag regex");
        let h_open = regex::Regex::new(r"<h[1-6][^>]*>")
            .expect("Failed to compile header open tag regex");
        let h_close = regex::Regex::new(r"</h[1-6]>")
            .expect("Failed to compile header close tag regex");
        let remaining_tags = regex::Regex::new(r"<[^>]*>")
            .expect("Failed to compile remaining tags regex");
        let multi_space = regex::Regex::new(r" +")
            .expect("Failed to compile multi space regex");
        let multi_newline = regex::Regex::new(r"\n{3,}")
            .expect("Failed to compile multi newline regex");
        let leading_space = regex::Regex::new(r"^ +")
            .expect("Failed to compile leading space regex");
        let line_leading_space = regex::Regex::new(r"\n +")
            .expect("Failed to compile line leading space regex");
        let empty_lines = regex::Regex::new(r"\n\s*\n\s*\n+")
            .expect("Failed to compile empty lines regex");
        let italic = regex::Regex::new(r"_([^_]+)_")
            .expect("Failed to compile italic regex");
        let css_rule = regex::Regex::new(r"[a-zA-Z0-9#\.@]+\s*\{[^}]*\}")
            .expect("Failed to compile CSS rule regex");

        Self {
            p_tag,
            h_open,
            h_close,
            remaining_tags,
            multi_space,
            multi_newline,
            leading_space,
            line_leading_space,
            empty_lines,
            italic,
            css_rule,
        }
    }
} 