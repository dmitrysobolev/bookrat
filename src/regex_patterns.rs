use regex;
use anyhow::{Context, Result};

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
    pub fn new() -> Result<Self> {
        let p_tag = regex::Regex::new(r"<p[^>]*>")
            .context("Failed to compile paragraph tag regex")?;
        let h_open = regex::Regex::new(r"<h[1-6][^>]*>")
            .context("Failed to compile header open tag regex")?;
        let h_close = regex::Regex::new(r"</h[1-6]>")
            .context("Failed to compile header close tag regex")?;
        let remaining_tags = regex::Regex::new(r"<[^>]*>")
            .context("Failed to compile remaining tags regex")?;
        let multi_space = regex::Regex::new(r" +")
            .context("Failed to compile multi space regex")?;
        let multi_newline = regex::Regex::new(r"\n{3,}")
            .context("Failed to compile multi newline regex")?;
        let leading_space = regex::Regex::new(r"^ +")
            .context("Failed to compile leading space regex")?;
        let line_leading_space = regex::Regex::new(r"\n +")
            .context("Failed to compile line leading space regex")?;
        let empty_lines = regex::Regex::new(r"\n\s*\n\s*\n+")
            .context("Failed to compile empty lines regex")?;
        let italic = regex::Regex::new(r"_([^_]+)_")
            .context("Failed to compile italic regex")?;
        let css_rule = regex::Regex::new(r"[a-zA-Z0-9#\.@]+\s*\{[^}]*\}")
            .context("Failed to compile CSS rule regex")?;

        Ok(Self {
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
        })
    }
} 