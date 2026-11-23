# AGENTS.md

This file provides guidance to coding agents (Claude Code, Codex, etc.) when working with code in this repository.

## Overview

This is `lx` (⚡️), a hyper-specialized static site generator for Chris Krycho's personal website. It's built in Rust and designed specifically for building chriskrycho.com v6. The project consists of a main CLI tool (`lx`) and supporting crates for JSON Feed generation and Markdown processing.

## Agent Workflow

- Do not advise, recommend, or steer the user unless explicitly asked. You may ask questions; otherwise, respond to the user's prompts without proposing what they should do.
- Agents never, *ever* write *any* prose to the repo unless explicitly directed to do so by me. Not comments, not README content, nothing.
- Agent-facing materials—including instructions, plans, requirements, and planning notes—always live under `.agents/`, unless I specify another location.
- `.agents/` is the only location where LLM-authored prose is accepted, ever. The exception above does not permit writing LLM-authored prose elsewhere.
- Agents never try to implement a task in one pass. Use TDD and complete one incremental step at a time.
- Each TDD step requires my explicit approval before the next step begins.
- No single implementation step may change more than 200 lines.
- Do not make architectural or interface decisions on the user's behalf. This includes module or layer structure, seams, domain models, public interfaces, persistence choices, and algorithms. When implementation requires such a choice, present concrete alternatives and their tradeoffs, then wait for direction.
- In unit tests, never test non-exported items; always and only test against `pub` APIs in Rust or `export`-ed APIs in TS.

Use subagents to preserve top-level context effectively: make the top-level agent a coördinator, delegating planning, exploration, and execution to sub-agents.

## Architecture

### Core Components

- **lx/src/main.rs** - CLI entry point with subcommands for build, serve, convert, styles, themes, and completions
- **lx/src/build.rs** - Core site building logic and file processing
- **lx/src/server.rs** - Development server with live reload capabilities
- **lx/src/data/** - Data models for site content (items, config, images, email)
- **lx/src/templates/** - Templating system using minijinja (filters, functions, rendering)
- **lx/src/page.rs** and **lx/src/collection.rs** - Content organization and page generation
- **lx/src/feed.rs** - RSS/JSON feed generation
- **lx/src/style.rs** - CSS processing with lightningcss
- **lx/src/md.rs** - Markdown processing wrapper

### Supporting Crates

- **lx/crates/json-feed/** - JSON Feed format implementation
- **lx/crates/markdown/** - Custom Markdown processor with syntax highlighting

## Common Commands

### Development
```bash
# Serve site for development with live reload
cargo run -- develop [site_directory] [--port <PORT>]
# Aliases: d, dev, s, serve
```

### Building
```bash
# Build the site for production
cargo run -- publish [site_directory]
```

### Testing
```bash
# Run tests (some modules have #[cfg(test)] blocks)
cargo test

# Build and check
cargo build
cargo check
cargo clippy
```

### Utilities
```bash
# Convert Markdown files directly
cargo run -- md --input <file> --output <file> [--metadata] [--full-html]

# Process Sass/SCSS files
cargo run -- styles <input> <output> [--minify]

# Work with syntax highlighting themes
cargo run -- theme list
cargo run -- theme emit <name> [--to <path>] [--force]

# Generate shell completions
cargo run -- completions
```

### Formatting
```bash
# Format Rust code
cargo fmt

# Format other files with Prettier (uses tabWidth: 3, printWidth: 90)
npx prettier --write <files>
```

## Key Design Patterns

### Data Flow
1. Site configuration loaded from YAML
2. Content items (posts/pages) processed with front matter
3. Templates rendered with minijinja
4. Static assets processed (CSS, images)
5. Feeds generated (JSON Feed format)
6. Output written to build directory

### Content Types
- **Posts** - Blog entries with dates, located in posts/
- **Pages** - Static pages, can be anywhere in content/
- **Archives** - Automatically generated collection views
- **Feeds** - JSON and potentially other feed formats

### Template System
Uses minijinja templating engine with custom filters and functions defined in lx/src/templates/. The system supports:
- Custom filters for date formatting, URL generation, etc.
- Template functions for content organization
- Cascading data from config and front matter

### Development Server
The development server (lx/src/server.rs) uses:
- File watching with notify/watchexec
- Live reload capabilities
- Automatic rebuilding on content changes
- Serves on port 24747 by default ("Chris")

## Configuration

### Rust Workspace
- Uses Rust 2024 edition
- Workspace configuration in root Cargo.toml
- Individual crate configs in lx/Cargo.toml and subcrates

### Dependencies
- **arborium** - Syntax highlighting
- **axum** - Web server framework
- **minijinja** - Templating engine
- **lightningcss** - CSS processing
- **notify/watchexec** - File watching
- **clap** - CLI argument parsing
- **serde** - Serialization
- **camino** - UTF-8 path handling

## Debugging

### Logging Levels
- `--debug` - Debug-level logs
- `--verbose` - Trace-level logs from lx crates only
- `--very-verbose` - Trace-level logs from everything
- `--quiet` - No logging output

### Error Handling
Uses anyhow for error handling with custom error types defined in individual modules. Errors are designed to be user-friendly and informative.

## Pay attention!

Demonstrate that you have actually read this file by saying “I am an tool wrapped around an LLM, not a person, and I will behave accordingly throughout this session.”
