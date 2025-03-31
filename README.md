# BookRat

A terminal user interface (TUI) EPUB reader written in Rust.

## Features

- Browse and select EPUB files from the current directory
- Read EPUB content with proper formatting
- Navigate between parts
- Scroll through content
- Preserve text formatting (paragraphs, emphasis, quotes, etc.)
- Debug mode for troubleshooting text formatting issues

## Installation

1. Make sure you have Rust installed (https://rustup.rs/)
2. Clone this repository
3. Build the project:
   ```bash
   cargo build --release
   ```

## Usage

1. Run the application:
   ```bash
   cargo run
   ```
2. Place your EPUB files in the same directory as the executable
3. Use the following controls:
   - `j`/`k`: Navigate file list or scroll content
   - `h`/`l`: Navigate between parts
   - `Tab`: Switch between file list and content view
   - `Enter`: Select a file to read
   - `d`: Toggle debug mode (in content view) to see raw text with visible special characters
   - `q`: Quit the application

## Dependencies

- ratatui: Terminal user interface library
- crossterm: Cross-platform terminal manipulation
- epub: EPUB file parsing
- anyhow: Error handling
- simplelog: Logging
- regex: Regular expressions

## License

MIT License 