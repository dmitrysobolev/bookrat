mod bookmark;
mod regex_patterns;

use std::{
    fs::File,
    io::{stdout, BufReader, Stdout},
    time::Duration,
    path::Path,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use epub::doc::EpubDoc;
use log::{debug, error, info, warn};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use simplelog::{Config, LevelFilter, WriteLogger};
use regex::{self};
use textwrap::{fill, wrap_algorithms::Penalties, Options, WrapAlgorithm};

use crate::bookmark::Bookmarks;
use crate::regex_patterns::RegexPatterns;

struct App {
    epub_files: Vec<String>,
    selected: usize,
    current_content: Option<String>,
    list_state: ListState,
    current_epub: Option<EpubDoc<BufReader<std::fs::File>>>,
    current_chapter: usize,
    total_chapters: usize,
    scroll_offset: usize,
    mode: Mode,
    bookmarks: Bookmarks,
    current_file: Option<String>,
    content_length: usize,
    last_scroll_time: std::time::Instant,
    scroll_speed: usize,
    regex: RegexPatterns,
    debug_mode: bool,
}

#[derive(PartialEq)]
enum Mode {
    FileList,
    Content,
}

impl App {
    fn new() -> Result<Self> {
        let bookmarks = Bookmarks::load().context("Failed to load bookmarks")?;

        let mut epub_files = Vec::new();
        let books_dir = "./books";
        
        let entries = std::fs::read_dir(books_dir)
            .with_context(|| format!("Failed to read directory: {}", books_dir))?;

        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to process directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "epub" {
                        if let Some(path_str) = path.to_str() {
                            epub_files.push(path_str.to_string());
                        } else {
                            warn!("Failed to convert path to string: {:?}", path);
                        }
                    }
                }
            }
        }

        epub_files.sort();
        info!("Found EPUB files: {:?}", epub_files);

        let mut list_state = ListState::default();
        if !epub_files.is_empty() {
            list_state.select(Some(0));
        }
        
        let regex = RegexPatterns::new(); 

        Ok(Self {
            mode: Mode::FileList,
            list_state,
            epub_files,
            selected: 0,
            current_epub: None,
            current_file: None,
            current_content: None,
            content_length: 0,
            scroll_offset: 0,
            current_chapter: 0,
            total_chapters: 0,
            bookmarks,
            last_scroll_time: std::time::Instant::now(),
            scroll_speed: 1,
            debug_mode: false,
            regex,
        })
    }

    fn process_html_content(content: &str, regex: &RegexPatterns) -> String {
        let text = content.replace('\n', "");
        
        let text = regex.css_rule.replace_all(&text, "").to_string();

        let text = regex.h_open.replace_all(&text, "\n").to_string();
        let text = regex.h_close.replace_all(&text, "\n").to_string();

        let text = regex.multi_space.replace_all(&text, " ").to_string();
        let text = regex.leading_space.replace_all(&text, "").to_string();
        let text = regex.line_leading_space.replace_all(&text, "\n").to_string();

        let mut first_paragraph = true;
        let text = regex.p_tag.replace_all(&text, |_caps: &regex::Captures| {
            if first_paragraph {
                first_paragraph = false;
                ""
            } else {
                "\n    "
            }
        }).to_string();
        
        let text = text
            .replace("</p>", "")
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("<br />", "\n")
            .replace("<blockquote>", "\n    ")
            .replace("</blockquote>", "\n")
            .replace("<em>", "_")
            .replace("</em>", "_")
            .replace("<i>", "_")
            .replace("</i>", "_")
            .replace("<strong>", "**")
            .replace("</strong>", "**")
            .replace("<b>", "**")
            .replace("</b>", "**");

        let text = regex.remaining_tags.replace_all(&text, "").to_string();

        let text = text
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&mdash;", "—")
            .replace("&ndash;", "–")
            .replace("&hellip;", "...")
            .replace("&ldquo;", "\u{201C}")
            .replace("&rdquo;", "\u{201D}")
            .replace("&lsquo;", "\u{2018}")
            .replace("&rsquo;", "\u{2019}");

        let text = regex.empty_lines.replace_all(&text, "\n").to_string();
        let mut text = regex.multi_newline.replace_all(&text, "\n").to_string();
        
        if text.starts_with("\n    ") {
            text.replace_range(1..5, "");
        }

        text.trim().to_string()
    }

    fn load_epub(&mut self, path: &str) {
        info!("Attempting to load EPUB: {}", path);
        match EpubDoc::new(path) {
            Ok(mut doc) => {
                info!("Successfully created EPUB document");
                self.total_chapters = doc.get_num_pages();
                info!("Total chapters: {}", self.total_chapters);

                if let Some(bookmark) = self.bookmarks.get_bookmark(path) {
                    info!("Found bookmark: chapter {}, offset {}", bookmark.chapter, bookmark.scroll_offset);
                    if bookmark.chapter > 0 {
                        doc.set_current_page(0);
                        for _ in 0..bookmark.chapter {
                            if !doc.go_next() {
                                error!("Failed to navigate to bookmarked chapter at index {}", bookmark.chapter);
                                doc.set_current_page(0);
                                self.current_chapter = 0;
                                self.scroll_offset = 0;
                                break;
                            }
                        }
                        if doc.get_current_page() == bookmark.chapter {
                            self.current_chapter = bookmark.chapter;
                            self.scroll_offset = bookmark.scroll_offset;
                        } else {
                             error!("Could not reach bookmarked chapter index {}", bookmark.chapter);
                             if self.total_chapters > 1 {
                                 doc.set_current_page(1);
                                 self.current_chapter = 1;
                             } else {
                                 doc.set_current_page(0);
                                 self.current_chapter = 0;
                             }
                             self.scroll_offset = 0;
                        }
                    }
                } else {
                    if self.total_chapters > 1 {
                        if doc.go_next() {
                            self.current_chapter = 1;
                            info!("Skipped potential metadata page, moved to chapter 1 (index 1)");
                        } else {
                            error!("Failed to move to chapter 1");
                            self.current_chapter = 0;
                        }
                    }
                }

                self.current_epub = Some(doc);
                self.current_file = Some(path.to_string());
                self.update_content();
                self.mode = Mode::Content;
            }
            Err(e) => {
                error!("Failed to load EPUB: {}: {}", path, e);
            }
        }
    }

    fn save_bookmark(&mut self) {
        if let Some(path) = &self.current_file {
            self.bookmarks.update_bookmark(path, self.current_chapter, self.scroll_offset);
            if let Err(e) = self.bookmarks.save() {
                error!("Failed to save bookmark: {}", e);
            }
        }
    }

    fn update_content(&mut self) {
        if let Some(doc) = &mut self.current_epub {
            if let Some((content, _mime)) = doc.get_current_str() {
                debug!("Raw content length: {} bytes", content.len());

                if self.debug_mode {
                    self.content_length = content.len();
                    self.current_content = Some(content);
                } else {
                    let text = Self::process_html_content(&content, &self.regex);
                    debug!("Processed text length: {} bytes", text.len());
                    debug!("Text after HTML cleanup: {}", text.chars().take(100).collect::<String>());

                    if text.is_empty() {
                        warn!("Converted text is empty");
                        self.current_content = Some("No content available in this chapter.".to_string());
                        self.content_length = 0;
                    } else {
                        self.content_length = text.len(); 
                        self.current_content = Some(text);
                    }
                }
            } else {
                error!("Failed to get current chapter content for index {}", self.current_chapter);
                self.current_content = Some("Error reading chapter content.".to_string());
                self.content_length = 0;
            }
        } else {
            error!("No EPUB document loaded");
            self.current_content = Some("No EPUB document loaded.".to_string());
            self.content_length = 0;
        }
    }

    fn next_chapter(&mut self) {
        if let Some(doc) = &mut self.current_epub {
            if self.current_chapter < self.total_chapters.saturating_sub(1) {
                if doc.go_next() {
                    self.current_chapter += 1;
                    info!("Moving to next chapter: {}", self.current_chapter);
                    self.update_content();
                    self.scroll_offset = 0;
                    self.save_bookmark();
                } else {
                    error!("Failed to move to next chapter from {}", self.current_chapter);
                }
            } else {
                info!("Already at last chapter {}", self.current_chapter);
            }
        }
    }

    fn prev_chapter(&mut self) {
        if let Some(doc) = &mut self.current_epub {
            if self.current_chapter > 0 {
                if doc.go_prev() {
                    self.current_chapter -= 1;
                    info!("Moving to previous chapter: {}", self.current_chapter);
                    self.update_content();
                    self.scroll_offset = 0;
                    self.save_bookmark();
                } else {
                    error!("Failed to move to previous chapter from {}", self.current_chapter);
                }
            } else {
                info!("Already at first chapter (0)");
            }
        }
    }

    fn scroll_down(&mut self) {
        if let Some(content) = &self.current_content {
            let now = std::time::Instant::now();
            if now.duration_since(self.last_scroll_time) < std::time::Duration::from_millis(100) {
                self.scroll_speed = (self.scroll_speed + 1).min(10);
            } else {
                self.scroll_speed = 1;
            }
            self.last_scroll_time = now;

            self.scroll_offset = self.scroll_offset.saturating_add(self.scroll_speed);
            let total_lines = content.lines().count();
            debug!("Scrolling down to offset: {}/{} (speed: {})", self.scroll_offset, total_lines, self.scroll_speed);
            self.save_bookmark();
        }
    }

    fn scroll_up(&mut self) {
        if let Some(content) = &self.current_content {
            let now = std::time::Instant::now();
            if now.duration_since(self.last_scroll_time) < std::time::Duration::from_millis(100) {
                self.scroll_speed = (self.scroll_speed + 1).min(10);
            } else {
                self.scroll_speed = 1;
            }
            self.last_scroll_time = now;

            self.scroll_offset = self.scroll_offset.saturating_sub(self.scroll_speed);
            let total_lines = content.lines().count();
            debug!("Scrolling up to offset: {}/{} (speed: {})", self.scroll_offset, total_lines, self.scroll_speed);
            self.save_bookmark();
        }
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.size());

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(70),
            ])
            .split(chunks[0]);

        let items: Vec<ListItem> = self
            .epub_files
            .iter()
            .map(|file| {
                let bookmark = self.bookmarks.get_bookmark(file);
                let last_read = bookmark
                    .map(|b| b.last_read.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Never".to_string());
                
                let display_name = Path::new(file)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                
                let content = Line::from(vec![
                    Span::styled(
                        display_name,
                        Style::default(),
                    ),
                    Span::styled(
                        format!(" ({})", last_read),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(content)
            })
            .collect();

        let files = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Books"))
            .highlight_style(Style::default().bg(Color::White).fg(Color::Black));

        f.render_stateful_widget(files, main_chunks[0], &mut self.list_state.clone());

        let content_display_text = self
            .current_content
            .as_deref()
            .unwrap_or("Select a file to view its content");

        let title = if self.current_epub.is_some() && !self.debug_mode {
            let chapter_progress = if let Some(ref content) = self.current_content {
                if !content.is_empty() {
                    let visible_width = main_chunks[1].width.saturating_sub(2);
                    if visible_width > 0 {
                        let options = Options::new(visible_width as usize)
                            .word_separator(textwrap::WordSeparator::AsciiSpace)
                            .wrap_algorithm(WrapAlgorithm::OptimalFit(Penalties::default()));
                        let wrapped_lines = fill(content, &options);
                        let total_lines = wrapped_lines.lines().count();
                        
                        let visible_height = main_chunks[1].height.saturating_sub(2);
                        let max_scroll_offset = total_lines.saturating_sub(visible_height as usize);
                        
                        if max_scroll_offset > 0 {
                            let current_scroll = self.scroll_offset;
                            ((current_scroll as f32 / max_scroll_offset as f32) * 100.0).min(100.0) as u32
                        } else {
                            100
                        }
                    } else {
                        0 
                    }
                } else {
                    0 
                }
            } else {
                0
            };
            format!(
                "Part {}/{} | Progress: {}%",
                self.current_chapter + 1,
                self.total_chapters,
                chapter_progress
            )
        } else if self.debug_mode && self.current_epub.is_some() {
            format!(
                "Part {}/{} [DEBUG MODE]",
                self.current_chapter + 1,
                self.total_chapters
            )
        } else {
            "Content".to_string()
        };

        let styled_content: Vec<Line> = if let Some(ref content_str) = self.current_content {
            let mut styled_lines = Vec::new();
            let mut is_italic = false;
            let mut is_bold = false;
            
            for line in content_str.lines() {
                let mut current_line_spans = Vec::new();
                let mut current_text = String::new();
                let mut chars = line.chars().peekable();
                
                while let Some(c) = chars.next() {
                    if c == '_' {
                        if !current_text.is_empty() {
                            let mut style = Style::default().fg(Color::White);
                            if is_italic { style = style.italic(); }
                            if is_bold { style = style.bold(); }
                            current_line_spans.push(Span::styled(current_text.clone(), style));
                            current_text.clear();
                        }
                        is_italic = !is_italic;
                    } else if c == '*' && chars.peek() == Some(&'*') {
                        chars.next(); 
                        if !current_text.is_empty() {
                            let mut style = Style::default().fg(Color::White);
                            if is_italic { style = style.italic(); }
                            if is_bold { style = style.bold(); }
                            current_line_spans.push(Span::styled(current_text.clone(), style));
                            current_text.clear();
                        }
                        is_bold = !is_bold;
                    } else {
                        current_text.push(c);
                    }
                }
                
                if !current_text.is_empty() {
                    let mut style = Style::default().fg(Color::White);
                    if is_italic { style = style.italic(); }
                    if is_bold { style = style.bold(); }
                    current_line_spans.push(Span::styled(current_text, style));
                }
                
                styled_lines.push(Line::from(current_line_spans));
            }
            styled_lines
        } else {
            vec![Line::from(content_display_text)]
        };

        let content_paragraph = Paragraph::new(styled_content)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset as u16, 0));

        f.render_widget(content_paragraph, main_chunks[1]);

        let help_text = match self.mode {
            Mode::FileList => "j/k: Navigate | Enter: Select | Tab: Switch View | q: Quit",
            Mode::Content => {
                if self.debug_mode {
                    "j/k: Scroll | h/l: Change Part | Tab: Switch View | d: Toggle Debug | q: Quit"
                } else {
                    "j/k: Scroll | h/l: Change Part | Tab: Switch View | d: Toggle Debug | q: Quit"
                }
            },
        };
        let help = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(help, chunks[1]);
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("Failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    ).context("Failed to leave alternate screen/disable mouse capture")?;
    terminal.show_cursor().context("Failed to show cursor")
}

fn main() -> Result<()> {
    WriteLogger::init(
        LevelFilter::Debug,
        Config::default(),
        File::create("bookrat.log")?,
    )?;

    info!("Starting BookRat EPUB reader");

    let mut app = App::new()?;

    let mut terminal = setup_terminal()?;

    let result = run_app(&mut terminal, &mut app);

    restore_terminal(&mut terminal)?;

    if let Err(err) = result {
        eprintln!("Error: {:?}", err);
        error!("Application runtime error: {:?}", err);
        return Err(err);
    }

    info!("Shutting down BookRat cleanly");
    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();

    loop {
        terminal.draw(|f| app.draw(f))?;
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('j') => {
                        if app.mode == Mode::FileList {
                            if app.selected < app.epub_files.len().saturating_sub(1) {
                                app.selected += 1;
                                app.list_state.select(Some(app.selected));
                            }
                        } else {
                            app.scroll_down();
                        }
                    }
                    KeyCode::Char('k') => {
                        if app.mode == Mode::FileList {
                            if app.selected > 0 {
                                app.selected -= 1;
                                app.list_state.select(Some(app.selected));
                            }
                        } else {
                            app.scroll_up();
                        }
                    }
                    KeyCode::Char('h') => {
                        if app.mode == Mode::Content {
                            app.prev_chapter();
                        }
                    }
                    KeyCode::Char('l') => {
                        if app.mode == Mode::Content {
                            app.next_chapter();
                        }
                    }
                    KeyCode::Enter => {
                        if app.mode == Mode::FileList {
                            if let Some(path) = app.epub_files.get(app.selected).cloned() {
                                app.load_epub(&path);
                            }
                        }
                    }
                    KeyCode::Tab => {
                        app.mode = if app.mode == Mode::FileList {
                            Mode::Content
                        } else {
                            if let Some(current_file) = &app.current_file {
                                if let Some(pos) = app.epub_files.iter().position(|f| f == current_file) {
                                    app.selected = pos;
                                    app.list_state.select(Some(pos));
                                }
                            }
                            Mode::FileList
                        };
                    }
                    KeyCode::Char('d') => {
                        if app.mode == Mode::Content {
                            app.debug_mode = !app.debug_mode;
                            app.update_content();
                        }
                    }
                    _ => {}
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_test_regex() -> RegexPatterns {
        RegexPatterns::new()
    }

    #[test]
    fn test_html_formatting() {
        let regex = get_test_regex();
        let test_content = r#"
            <h1>Header 1</h1>
            <p>First paragraph with <em>italic</em> text.</p>
            <p>Second paragraph with <strong>bold</strong> text.</p>
            <blockquote>A blockquote</blockquote>
            <p>Third paragraph with <br/> line break.</p>
            <p>Fourth paragraph with &quot;quotes&quot; and &mdash; dash.</p>
        "#;

        let content = App::process_html_content(test_content, &regex);

        assert!(content.contains("Header 1"));
        assert!(content.contains("First paragraph with _italic_ text."));
        assert!(content.contains("Second paragraph with **bold** text."));
        assert!(content.contains("Third paragraph with \n line break."));
        assert!(content.contains("Fourth paragraph with \"quotes\" and — dash."));
        let paragraphs: Vec<&str> = content.split('\n').collect();
        assert!(paragraphs.len() >= 5);
        assert!(content.contains("    A blockquote"));
        assert!(!content.contains("&quot;"));
        assert!(!content.contains("&mdash;"));
        assert!(!content.contains("<em>"));
        assert!(!content.contains("<strong>"));
    }

    #[test]
    fn test_empty_content() {
        let regex = get_test_regex();
        let test_content = "";
        let content = App::process_html_content(test_content, &regex);
        assert!(content.is_empty());
    }

    #[test]
    fn test_html_entities() {
        let regex = get_test_regex();
        let test_content = r#"<p>&amp; &lt; &gt; &apos; &ldquo; &rdquo; &lsquo; &rsquo;</p>"#;
        let content = App::process_html_content(test_content, &regex);

        assert!(content.contains("&"));
        assert!(content.contains("<"));
        assert!(content.contains(">"));
        assert!(content.contains("'"));
        assert!(content.contains("\u{201C}"));
        assert!(content.contains("\u{201D}"));
        assert!(content.contains("\u{2018}"));
        assert!(content.contains("\u{2019}"));
        let expected = format!("& < > ' {} {} {} {}", 
            '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}');
        assert_eq!(content, expected);
    }

    #[test]
    fn test_paragraph_indentation() {
        let regex = get_test_regex();
        let test_content = r#"<p>First paragraph</p><p>Second paragraph</p><p>Third paragraph</p>"#;
        let content = App::process_html_content(test_content, &regex);

        let paragraphs: Vec<&str> = content.split('\n').collect();
        assert!(paragraphs.len() == 3);
        assert!(content.starts_with("First paragraph"));
        assert!(content.contains("\n    Second paragraph"));
        assert!(content.contains("\n    Third paragraph"));
    }

    #[test]
    fn test_paragraphs_with_empty_lines() {
        let regex = get_test_regex();
        let test_content = r#"<p>First paragraph</p>

        <p>Second paragraph</p>


        <p>Third paragraph</p>

        <p>Fourth paragraph</p>"#;
        let content = App::process_html_content(test_content, &regex);

        let paragraphs: Vec<&str> = content.split("\n").collect();
        assert!(paragraphs.len() == 4, "Expected 4 paragraphs, got {}", paragraphs.len());
        assert!(content.contains("First paragraph"));
        assert!(content.contains("    Second paragraph"));
        assert!(content.contains("    Third paragraph"));
        assert!(content.contains("    Fourth paragraph"));
        let expected = "First paragraph\n    Second paragraph\n    Third paragraph\n    Fourth paragraph";
        assert_eq!(content, expected, "Content does not match expected format");
    }
}
