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
use html2text::from_read;
use log::{debug, error, info, warn};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use simplelog::{Config, LevelFilter, WriteLogger};
use regex;

use crate::bookmark::{Bookmark, Bookmarks};

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
}

#[derive(PartialEq)]
enum Mode {
    FileList,
    Content,
}

impl App {
    fn new() -> Self {
        let epub_files = std::fs::read_dir(".")
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()?.to_str()? == "epub" {
                    // Get filename without extension
                    let filename = path.file_stem()?.to_string_lossy().to_string();
                    // Store full path for internal use
                    Some(path.to_str()?.to_string())
                } else {
                    None
                }
            })
            .collect();

        let mut list_state = ListState::default();
        list_state.select(None);

        let bookmarks = Bookmarks::load().unwrap_or_else(|e| {
            error!("Failed to load bookmarks: {}", e);
            Bookmarks::new()
        });

        Self {
            epub_files,
            selected: 0,
            current_content: None,
            list_state,
            current_epub: None,
            current_chapter: 0,
            total_chapters: 0,
            scroll_offset: 0,
            mode: Mode::FileList,
            bookmarks,
            current_file: None,
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

                // Second pass: Convert semantic HTML elements to plain text with proper formatting
                let p_tag_re = regex::Regex::new(r"<p[^>]*>").unwrap();
                let text = p_tag_re.replace_all(&text, "").to_string();
                
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

                // Handle headers
                let h_open_re = regex::Regex::new(r"<h[1-6][^>]*>").unwrap();
                let h_close_re = regex::Regex::new(r"</h[1-6]>").unwrap();
                let text = h_open_re.replace_all(&text, "\n\n").to_string();
                let text = h_close_re.replace_all(&text, "\n\n").to_string();

                // Third pass: Remove any remaining HTML tags
                let remaining_tags = regex::Regex::new(r"<[^>]*>").unwrap();
                let text = remaining_tags.replace_all(&text, "").to_string();

                // Fourth pass: Clean up whitespace while preserving intentional formatting
                let multi_space_re = regex::Regex::new(r" +").unwrap();
                let multi_newline_re = regex::Regex::new(r"\n{3,}").unwrap();
                let leading_space_re = regex::Regex::new(r"^ +").unwrap();
                let line_leading_space_re = regex::Regex::new(r"\n +").unwrap();

                let text = multi_space_re.replace_all(&text, " ").to_string();
                let text = multi_newline_re.replace_all(&text, "\n\n").to_string();
                let text = leading_space_re.replace_all(&text, "").to_string();
                let text = line_leading_space_re.replace_all(&text, "\n").to_string();
                let text = text.trim().to_string();

                debug!("Text after HTML cleanup: {}", text.chars().take(100).collect::<String>());
                
                if text.is_empty() {
                    warn!("Converted text is empty");
                    self.current_content = Some("No content available in this chapter.".to_string());
                } else {
                    debug!("Final text length: {} bytes", text.len());
                    self.current_content = Some(text);
                }
            } else {
                error!("Failed to get current chapter content");
                self.current_content = Some("Error reading chapter content.".to_string());
            }
        } else {
            error!("No EPUB document loaded");
            self.current_content = Some("No EPUB document loaded.".to_string());
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
        if self.current_content.is_some() {
            self.scroll_offset = self.scroll_offset.saturating_add(1);
            debug!("Scrolling down to offset: {}", self.scroll_offset);
            self.save_bookmark();
        }
    }

    fn scroll_up(&mut self) {
        if self.current_content.is_some() {
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
            debug!("Scrolling up to offset: {}", self.scroll_offset);
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
            .block(Block::default().borders(Borders::ALL).title("EPUB Files"))
            .highlight_style(Style::default().bg(Color::White).fg(Color::Black));

        f.render_stateful_widget(files, main_chunks[0], &mut self.list_state.clone());

        // Draw content
        let content = self
            .current_content
            .as_deref()
            .unwrap_or("Select a file to view its content");
        
        let title = if self.current_epub.is_some() {
            format!("Content (Chapter {}/{})", self.current_chapter + 1, self.total_chapters)
        } else {
            "Content".to_string()
        };

        let content = Paragraph::new(content)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true })
            .scroll((self.scroll_offset as u16, 0))
            .style(Style::default().fg(Color::White));

        f.render_widget(content, main_chunks[1]);

        // Draw help bar
        let help_text = match self.mode {
            Mode::FileList => "j/k: Navigate | Enter: Select | Tab: Switch View | q: Quit",
            Mode::Content => "j/k: Scroll | h/l: Change Chapter | Tab: Switch View | q: Quit",
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
