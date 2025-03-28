mod bookmark;

use std::{
    fs::File,
    io::{stdout, BufReader},
    time::Duration,
    path::Path,
};

use anyhow::Result;
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
use regex;

use crate::bookmark::Bookmarks;

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
    p_tag_re: regex::Regex,
    h_open_re: regex::Regex,
    h_close_re: regex::Regex,
    remaining_tags_re: regex::Regex,
    multi_space_re: regex::Regex,
    multi_newline_re: regex::Regex,
    leading_space_re: regex::Regex,
    line_leading_space_re: regex::Regex,
    empty_lines_re: regex::Regex,
    italic_re: regex::Regex,
    css_rule_re: regex::Regex,
}

#[derive(PartialEq)]
enum Mode {
    FileList,
    Content,
}

impl App {
    fn new() -> Self {
        let p_tag_re = regex::Regex::new(r"<p[^>]*>")
            .expect("Failed to compile paragraph tag regex");
        let h_open_re = regex::Regex::new(r"<h[1-6][^>]*>")
            .expect("Failed to compile header open tag regex");
        let h_close_re = regex::Regex::new(r"</h[1-6]>")
            .expect("Failed to compile header close tag regex");
        let remaining_tags_re = regex::Regex::new(r"<[^>]*>")
            .expect("Failed to compile remaining tags regex");
        let multi_space_re = regex::Regex::new(r" +")
            .expect("Failed to compile multi space regex");
        let multi_newline_re = regex::Regex::new(r"\n{3,}")
            .expect("Failed to compile multi newline regex");
        let leading_space_re = regex::Regex::new(r"^ +")
            .expect("Failed to compile leading space regex");
        let line_leading_space_re = regex::Regex::new(r"\n +")
            .expect("Failed to compile line leading space regex");
        let empty_lines_re = regex::Regex::new(r"\n\s*\n\s*\n+")
            .expect("Failed to compile empty lines regex");
        let italic_re = regex::Regex::new(r"_([^_]+)_")
            .expect("Failed to compile italic regex");
        let css_rule_re = regex::Regex::new(r"[a-zA-Z0-9#\.@]+\s*\{[^}]*\}")
            .expect("Failed to compile CSS rule regex");

        let epub_files: Vec<String> = std::fs::read_dir(".")
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()?.to_str()? == "epub" {
                    // Store full path for internal use
                    Some(path.to_str()?.to_string())
                } else {
                    None
                }
            })
            .collect();

        let mut list_state = ListState::default();
        // Select first book if available
        let has_files = !epub_files.is_empty();
        if has_files {
            list_state.select(Some(0));
        }

        let bookmarks = Bookmarks::load().unwrap_or_else(|e| {
            error!("Failed to load bookmarks: {}", e);
            Bookmarks::new()
        });

        Self {
            epub_files: epub_files.clone(),
            selected: if has_files { 0 } else { 0 },
            current_content: None,
            list_state,
            current_epub: None,
            current_chapter: 0,
            total_chapters: 0,
            scroll_offset: 0,
            mode: Mode::FileList,
            bookmarks,
            current_file: None,
            content_length: 0,
            last_scroll_time: std::time::Instant::now(),
            scroll_speed: 1,
            p_tag_re,
            h_open_re,
            h_close_re,
            remaining_tags_re,
            multi_space_re,
            multi_newline_re,
            leading_space_re,
            line_leading_space_re,
            empty_lines_re,
            italic_re,
            css_rule_re,
        }
    }

    fn load_epub(&mut self, path: &str) {
        info!("Attempting to load EPUB: {}", path);
        if let Ok(mut doc) = EpubDoc::new(path) {
            info!("Successfully created EPUB document");
            self.total_chapters = doc.get_num_pages();
            info!("Total chapters: {}", self.total_chapters);
            
            // Try to load bookmark
            if let Some(bookmark) = self.bookmarks.get_bookmark(path) {
                info!("Found bookmark: chapter {}, offset {}", bookmark.chapter, bookmark.scroll_offset);
                // Skip metadata page if needed
                if bookmark.chapter > 0 {
                    for _ in 0..bookmark.chapter {
                        if doc.go_next().is_err() {
                            error!("Failed to navigate to bookmarked chapter");
                            break;
                        }
                    }
                    self.current_chapter = bookmark.chapter;
                    self.scroll_offset = bookmark.scroll_offset;
                }
            } else {
                // Skip the first chapter if it's just metadata
                if self.total_chapters > 1 {
                    if doc.go_next().is_ok() {
                        self.current_chapter = 1;
                        info!("Skipped metadata page, moved to chapter 2");
                    } else {
                        error!("Failed to move to next chapter");
                    }
                }
            }
            
            self.current_epub = Some(doc);
            self.current_file = Some(path.to_string());
            self.update_content();
            self.mode = Mode::Content;
        } else {
            error!("Failed to load EPUB: {}", path);
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
            if let Ok(content) = doc.get_current_str() {
                debug!("Raw content length: {} bytes", content.len());
                
                // First pass: Replace HTML entities
                let text = content
                    .replace("&nbsp;", " ")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&apos;", "'")
                    .replace("&mdash;", "—")
                    .replace("&ndash;", "–")
                    .replace("&hellip;", "...")
                    .replace("&ldquo;", "\u{201C}")  // Opening double quote
                    .replace("&rdquo;", "\u{201D}")  // Closing double quote
                    .replace("&lsquo;", "\u{2018}")  // Opening single quote
                    .replace("&rsquo;", "\u{2019}"); // Closing single quote

                // Remove CSS rules
                let text = self.css_rule_re.replace_all(&text, "").to_string();

                // Second pass: Convert semantic HTML elements to plain text with proper formatting
                let text = self.p_tag_re.replace_all(&text, "").to_string();
                
                let text = text
                    .replace("</p>", "\n\n")
                    // Preserve line breaks
                    .replace("<br>", "\n")
                    .replace("<br/>", "\n")
                    .replace("<br />", "\n")
                    // Handle blockquotes (for direct speech or citations)
                    .replace("<blockquote>", "\n    ")
                    .replace("</blockquote>", "\n")
                    // Handle emphasis
                    .replace("<em>", "_")
                    .replace("</em>", "_")
                    .replace("<i>", "_")
                    .replace("</i>", "_")
                    // Handle strong emphasis
                    .replace("<strong>", "**")
                    .replace("</strong>", "**")
                    .replace("<b>", "**")
                    .replace("</b>", "**");

                // Handle text wrapped in underscores
                let text = self.italic_re.replace_all(&text, |caps: &regex::Captures| {
                    format!("_{}_", caps.get(1).unwrap().as_str())
                }).to_string();

                // Handle headers
                let text = self.h_open_re.replace_all(&text, "\n\n").to_string();
                let text = self.h_close_re.replace_all(&text, "\n\n").to_string();

                // Third pass: Remove any remaining HTML tags
                let text = self.remaining_tags_re.replace_all(&text, "").to_string();

                // Fourth pass: Clean up whitespace while preserving intentional formatting
                let text = self.multi_space_re.replace_all(&text, " ").to_string();
                let text = self.multi_newline_re.replace_all(&text, "\n\n").to_string();
                let text = self.leading_space_re.replace_all(&text, "").to_string();
                let text = self.line_leading_space_re.replace_all(&text, "\n").to_string();
                
                // Fifth pass: Collapse multiple empty lines into a single one
                let text = self.empty_lines_re.replace_all(&text, "\n\n");
                
                let text = text.trim().to_string();

                debug!("Text after HTML cleanup: {}", text.chars().take(100).collect::<String>());
                
                if text.is_empty() {
                    warn!("Converted text is empty");
                    self.current_content = Some("No content available in this chapter.".to_string());
                    self.content_length = 0;
                } else {
                    debug!("Final text length: {} bytes", text.len());
                    self.current_content = Some(text);
                    self.content_length = self.current_content.as_ref().unwrap().len();
                }
            } else {
                error!("Failed to get current chapter content");
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
            if self.current_chapter < self.total_chapters - 1 {
                if doc.go_next().is_ok() {
                    self.current_chapter += 1;
                    info!("Moving to next chapter: {}", self.current_chapter + 1);
                    self.update_content();
                    self.scroll_offset = 0;
                    self.save_bookmark();
                } else {
                    error!("Failed to move to next chapter");
                }
            } else {
                info!("Already at last chapter");
            }
        }
    }

    fn prev_chapter(&mut self) {
        if let Some(doc) = &mut self.current_epub {
            if self.current_chapter > 0 {
                if doc.go_prev().is_ok() {
                    self.current_chapter -= 1;
                    info!("Moving to previous chapter: {}", self.current_chapter + 1);
                    self.update_content();
                    self.scroll_offset = 0;
                    self.save_bookmark();
                } else {
                    error!("Failed to move to previous chapter");
                }
            } else {
                info!("Already at first chapter");
            }
        }
    }

    fn scroll_down(&mut self) {
        if let Some(content) = &self.current_content {
            // Check if we're scrolling continuously
            let now = std::time::Instant::now();
            if now.duration_since(self.last_scroll_time) < std::time::Duration::from_millis(100) {
                // Increase scroll speed up to a maximum
                self.scroll_speed = (self.scroll_speed + 1).min(10);
            } else {
                // Reset scroll speed if there was a pause
                self.scroll_speed = 1;
            }
            self.last_scroll_time = now;

            // Apply scroll with current speed
            self.scroll_offset = self.scroll_offset.saturating_add(self.scroll_speed);
            let total_lines = content.lines().count();
            debug!("Scrolling down to offset: {}/{} (speed: {})", self.scroll_offset, total_lines, self.scroll_speed);
            self.save_bookmark();
        }
    }

    fn scroll_up(&mut self) {
        if let Some(content) = &self.current_content {
            // Check if we're scrolling continuously
            let now = std::time::Instant::now();
            if now.duration_since(self.last_scroll_time) < std::time::Duration::from_millis(100) {
                // Increase scroll speed up to a maximum
                self.scroll_speed = (self.scroll_speed + 1).min(10);
            } else {
                // Reset scroll speed if there was a pause
                self.scroll_speed = 1;
            }
            self.last_scroll_time = now;

            // Apply scroll with current speed
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

        // Draw file list
        let items: Vec<ListItem> = self
            .epub_files
            .iter()
            .map(|file| {
                let bookmark = self.bookmarks.get_bookmark(file);
                let last_read = bookmark
                    .map(|b| b.last_read.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Never".to_string());
                
                // Get filename without path and extension for display
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

        // Draw content
        let content = self
            .current_content
            .as_deref()
            .unwrap_or("Select a file to view its content");
        
        let title = if self.current_epub.is_some() {
            let chapter_progress = if self.content_length > 0 {
                // Get the visible area width and height
                let visible_width = main_chunks[1].width as usize;
                let visible_height = main_chunks[1].height as usize;
                
                // Calculate total scrollable lines by counting actual content lines
                let content = self.current_content.as_ref().unwrap();
                let total_lines = content
                    .lines()
                    .filter(|line| !line.trim().is_empty()) // Skip empty lines
                    .map(|line| {
                        // Calculate how many terminal lines this content line will take
                        (line.len() as f32 / visible_width as f32).ceil() as usize
                    })
                    .sum::<usize>();
                
                // Calculate current visible line based on scroll offset
                let current_line = self.scroll_offset;
                
                // Calculate the maximum scroll position (when last line becomes visible at the bottom)
                let max_scroll = if total_lines > visible_height {
                    total_lines - visible_height
                } else {
                    0
                };
                
                // Calculate percentage based on how far we've scrolled to the max position
                let progress = if max_scroll > 0 {
                    ((current_line as f32 / max_scroll as f32) * 100.0).min(100.0) as u32
                } else {
                    100 // If content fits in one screen, we're at 100%
                };
                
                progress
            } else {
                0
            };
            format!("Part {}/{} | Progress: {}%", self.current_chapter + 1, self.total_chapters, chapter_progress)
        } else {
            "Content".to_string()
        };

        // Parse content and apply styling
        let mut styled_content = Vec::new();
        let mut is_italic = false;
        let mut is_bold = false;
        
        for line in content.lines() {
            let mut current_line_spans = Vec::new();
            let mut current_text = String::new();
            let mut chars = line.chars().peekable();
            
            while let Some(c) = chars.next() {
                if c == '_' {
                    // If we have accumulated text, add it with current style
                    if !current_text.is_empty() {
                        let mut style = Style::default().fg(Color::White);
                        if is_italic {
                            style = style.italic();
                        }
                        if is_bold {
                            style = style.bold();
                        }
                        current_line_spans.push(Span::styled(current_text.clone(), style));
                        current_text.clear();
                    }
                    // Toggle italic state
                    is_italic = !is_italic;
                } else if c == '*' && chars.peek() == Some(&'*') {
                    // Skip the next * since we've already handled it
                    chars.next();
                    // If we have accumulated text, add it with current style
                    if !current_text.is_empty() {
                        let mut style = Style::default().fg(Color::White);
                        if is_italic {
                            style = style.italic();
                        }
                        if is_bold {
                            style = style.bold();
                        }
                        current_line_spans.push(Span::styled(current_text.clone(), style));
                        current_text.clear();
                    }
                    // Toggle bold state
                    is_bold = !is_bold;
                } else {
                    current_text.push(c);
                }
            }
            
            // Add any remaining text with current style
            if !current_text.is_empty() {
                let mut style = Style::default().fg(Color::White);
                if is_italic {
                    style = style.italic();
                }
                if is_bold {
                    style = style.bold();
                }
                current_line_spans.push(Span::styled(current_text, style));
            }
            
            styled_content.push(Line::from(current_line_spans));
        }

        let content = Paragraph::new(styled_content)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true })
            .scroll((self.scroll_offset as u16, 0));

        f.render_widget(content, main_chunks[1]);

        // Draw help bar
        let help_text = match self.mode {
            Mode::FileList => "j/k: Navigate | Enter: Select | Tab: Switch View | q: Quit",
            Mode::Content => "j/k: Scroll | h/l: Change Part | Tab: Switch View | q: Quit",
        };
        let help = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(help, chunks[1]);
    }
}

fn main() -> Result<()> {
    // Initialize logging
    WriteLogger::init(
        LevelFilter::Debug,
        Config::default(),
        File::create("bookrat.log")?,
    )?;

    info!("Starting BookRat EPUB reader");

    // Terminal initialization
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run it
    let mut app = App::new();
    let res = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        error!("Application error: {:?}", err);
        println!("{err:?}");
    }

    info!("Shutting down BookRat");
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
                            // When switching back to file list, restore selection to current file
                            if let Some(current_file) = &app.current_file {
                                if let Some(pos) = app.epub_files.iter().position(|f| f == current_file) {
                                    app.selected = pos;
                                    app.list_state.select(Some(pos));
                                }
                            }
                            Mode::FileList
                        };
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
