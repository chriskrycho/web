//! Run the static site generator.

use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use anyhow::anyhow;
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate_to, shells::Fish};
use log::info;
use simplelog::{
   ColorChoice, Config, ConfigBuilder, LevelFilter, TermLogger, TerminalMode,
};
use syntect::highlighting::ThemeSet;
use syntect::html::{ClassStyle, css_for_theme_with_class_style};
use thiserror::Error;

mod archive;
mod build;
mod canonicalized;
mod collection;
mod data;
mod error;
mod feed;
mod md;
mod page;
mod server;
mod style;
mod templates;

use crate::build::build_in;
use crate::server::serve;

fn main() -> Result<(), anyhow::Error> {
   let mut cli = Cli::parse();

   // TODO: configure Miette or similar to print this particularly nicely. Then we can
   // just return that!
   setup_logger(&cli)?;

   let cwd = std::env::current_dir().expect(
      "Something is suuuuper borked: I cannot even get the current working directory!",
   );

   match cli.command {
      Command::Publish { site_directory } => {
         let directory = site_directory
            .unwrap_or_else(|| {
               info!(
                  "No directory passed, using current working directory ({}) instead",
                  cwd.display()
               );
               cwd
            })
            .try_into()?;

         build_in(directory)?;
         Ok(())
      }

      Command::Develop {
         site_directory,
         port,
      } => {
         let directory = site_directory.unwrap_or_else(|| {
            info!(
               "No directory passed, using current working directory ({}) instead",
               cwd.display()
            );
            cwd
         });

         if !directory.exists() {
            return Err(anyhow!(
               "Source directory '{}' does not exist",
               directory.display()
            ));
         }

         serve(&directory, port)?;
         Ok(())
      }

      Command::Convert {
         paths,
         include_metadata,
         full_html_output,
      } => {
         let (input, mut dest) = parse_paths(paths)?;
         md::convert(
            input,
            dest.writer(),
            md::Include {
               metadata: include_metadata,
               wrapping_html: full_html_output,
            },
         )
         .map_err(|source| Error::Markdown {
            dest: dest.to_string(),
            source,
         })?;
         Ok(())
      }

      Command::Styles { paths, minify } => {
         // TODO: make Mode a top-level concern
         let css = style::convert(
            &paths.input,
            if minify {
               style::Mode::Prod
            } else {
               style::Mode::Dev
            },
         )?;
         fs::write(paths.output, css)?;
         Ok(())
      }

      Command::Theme(Theme::List) => {
         let ThemeSet { themes } = ThemeSet::load_defaults();
         println!("Available themes:");
         for theme_name in themes.keys() {
            println!("\t{theme_name}");
         }
         Ok(())
      }

      Command::Theme(Theme::Emit { name, path, force }) => {
         let theme_set = ThemeSet::load_defaults();
         let theme = theme_set
            .themes
            .get(&name)
            .ok_or_else(|| Error::InvalidThemeName(name))?;

         let css = css_for_theme_with_class_style(theme, ClassStyle::Spaced)
            .map_err(|source| Error::SyntectCSS { source })?;

         let dest_cfg = path
            .map(|path| DestCfg::Path { buf: path, force })
            .unwrap_or(DestCfg::Stdout);

         let mut dest = output_buffer(&dest_cfg)?;
         dest
            .writer()
            .write_all(css.as_bytes())
            .map_err(|source| Error::Io {
               target: dest.to_string(),
               source,
            })?;

         Ok(())
      }

      Command::Completions => Ok(cli.completions()?),
   }
}

fn setup_logger(cli: &Cli) -> Result<(), log::SetLoggerError> {
   let level = if cli.verbose {
      LevelFilter::Trace
   } else if cli.debug {
      LevelFilter::Debug
   } else if cli.quiet {
      LevelFilter::Off
   } else {
      LevelFilter::Info
   };

   // If only `--verbose`, do not trace *other* crates. If `--very-verbose`,
   // trace everything.
   let config = if level == LevelFilter::Trace && !cli.very_verbose {
      let mut cfg = ConfigBuilder::new();
      for &crate_name in CRATES {
         cfg.add_filter_allow(crate_name.to_string());
      }
      cfg.build()
   } else {
      Config::default()
   };

   TermLogger::init(level, config, TerminalMode::Mixed, ColorChoice::Auto)
}

const CRATES: &[&str] = &["lx", "lx-md", "json-feed"];

#[derive(Parser, Debug)]
#[clap(
   name = "lx ⚡️",
   about = "A very fast, very opinionated static site generator",
   version = "1.0",
   author = "Chris Krycho <hello@chriskrycho.com>"
)]
#[command(author, version, about, arg_required_else_help(true))]
struct Cli {
   #[command(subcommand)]
   command: Command,

   /// Include debug-level logs
   #[arg(short, long, global = true, conflicts_with = "quiet")]
   debug: bool,

   /// Include trace-level logs from lx.
   #[arg(
      short,
      long,
      global = true,
      requires = "debug",
      conflicts_with = "quiet"
   )]
   verbose: bool,

   /// Include trace-level logs from *everything*.
   #[arg(long, global = true, conflicts_with = "quiet")]
   very_verbose: bool,

   /// Don't include *any* logging. None. Zip. Zero. Nada.
   #[arg(
      short,
      long,
      global = true,
      conflicts_with = "debug",
      conflicts_with = "verbose",
      conflicts_with = "very_verbose"
   )]
   quiet: bool,
}

#[derive(Error, Debug)]
enum Error {
   #[error("Somehow you don't have a home dir. lolwut")]
   NoHomeDir,

   #[error(transparent)]
   Completions { source: std::io::Error },

   #[error("`--force` is only allowed with `--output`")]
   InvalidArgs,

   #[error("could not open file at '{path}' {reason}")]
   CouldNotOpenFile {
      path: PathBuf,
      reason: FileOpenReason,
      source: std::io::Error,
   },

   #[error("invalid file path with no parent directory: '{path}'")]
   InvalidDirectory { path: PathBuf },

   #[error("could not create directory '{dir}' to write file '{path}")]
   CreateDirectory {
      dir: PathBuf,
      path: PathBuf,
      source: std::io::Error,
   },

   #[error(transparent)]
   CheckFileExists { source: std::io::Error },

   #[error("the file '{0}' already exists")]
   FileExists(PathBuf),

   #[error(transparent)]
   Logger(#[from] log::SetLoggerError),

   #[error("could not convert (for {dest})")]
   Markdown { dest: String, source: md::Error },

   #[error("invalid theme name: {0}")]
   InvalidThemeName(String),

   #[error(transparent)]
   SyntectCSS { source: syntect::Error },

   #[error("IO (for {target})")]
   Io {
      target: String,
      source: std::io::Error,
   },
}

impl Cli {
   fn completions(&mut self) -> Result<(), Error> {
      let mut config_dir = dirs::home_dir().ok_or_else(|| Error::NoHomeDir)?;
      config_dir.extend([".config", "fish", "completions"]);
      let mut cmd = Self::command();
      generate_to(Fish, &mut cmd, "lx", config_dir)
         .map(|_| ())
         .map_err(|source| Error::Completions { source })
   }
}

#[derive(Subcommand, Debug, PartialEq, Clone)]
enum Command {
   /// Go live
   Publish {
      /// The root of the site (if different from the current directory).
      site_directory: Option<PathBuf>,
   },

   /// Build and serve the site for development
   #[clap(aliases = ["d", "dev", "s", "serve"])]
   Develop {
      site_directory: Option<PathBuf>,

      /// Port to serve the site on. Defaults to `24747`, i.e, "Chris"
      #[arg(short, long)]
      port: Option<u16>,
   },

   /// Straight to the config. Give me completions for my own dang tool
   Completions,

   /// Emit Markdown *exactly* the same way `lx build|serve` does
   #[command(name = "md")]
   Convert {
      #[clap(flatten)]
      paths: Paths,

      /// Output any supplied metadata as a table (a la GitHub).
      #[arg(short = 'm', long = "metadata", default_value("false"))]
      include_metadata: bool,

      #[arg(
         long = "full-html",
         default_value("false"),
         default_missing_value("true")
      )]
      full_html_output: bool,
   },

   /// Work with syntax highlighting theme SCSS.
   #[command(subcommand)]
   Theme(Theme),

   /// Process one or more Sass/SCSS files exactly the same way `lx` does.
   ///
   /// (Does not compress styles the way a prod build does.)
   Styles {
      /// The entry points to process.
      #[clap(flatten)]
      paths: StylePaths,

      #[arg(short, long)]
      minify: bool,
   },
}

#[derive(Debug, PartialEq, Clone, Subcommand)]
enum Theme {
   /// List all themes,
   List,

   /// Emit a named theme
   #[arg()]
   Emit {
      /// The theme name to use. To see all themes, use `lx theme list`.
      name: String,

      /// Where to emit the theme CSS. If absent, will use `stdout`.
      #[arg(long = "to")]
      path: Option<PathBuf>,

      /// Overwrite any existing file at the path specified.
      #[arg(long, requires = "path")]
      force: bool,
   },
}

#[derive(Args, Debug, PartialEq, Clone)]
struct Paths {
   /// Path to the file to convert. Will use `stdin` if not supplied.
   #[arg(short, long)]
   input: Option<PathBuf>,

   /// Where to print the output. Will use `stdout` if not supplied.
   #[arg(short, long)]
   output: Option<PathBuf>,

   /// If the supplied `output` file is present, overwrite it.
   #[arg(long, default_missing_value("true"), num_args(0..=1), require_equals(true))]
   force: Option<bool>,
}

#[derive(Args, Debug, PartialEq, Clone)]
struct StylePaths {
   #[arg()]
   input: PathBuf,

   #[arg()]
   output: PathBuf,

   /// If the supplied `output` file is present, overwrite it.
   #[arg(long, default_missing_value("true"), num_args(0..=1), require_equals(true))]
   force: Option<bool>,
}

enum Dest {
   File { path: PathBuf, buf: Box<dyn Write> },
   Stdout(Box<dyn Write>),
}

impl std::fmt::Display for Dest {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      match self {
         Dest::File { path, .. } => write!(f, "{}", path.display()),
         Dest::Stdout(_) => f.write_str("stdin"),
      }
   }
}

impl Dest {
   fn writer(&mut self) -> &mut Box<dyn Write> {
      match self {
         Dest::File { buf, .. } => buf,
         Dest::Stdout(buf) => buf,
      }
   }
}

pub(crate) enum DestCfg {
   Path { buf: PathBuf, force: bool },
   Stdout,
}

type ParsedPaths = (Box<dyn Read>, Dest);

fn parse_paths(paths: Paths) -> Result<ParsedPaths, anyhow::Error> {
   let dest_cfg = match (paths.output, paths.force.unwrap_or(false)) {
      (Some(buf), force) => DestCfg::Path { buf, force },
      (None, false) => DestCfg::Stdout,
      (None, true) => return Err(Error::InvalidArgs)?,
   };
   let input = input_buffer(paths.input.as_ref())?;
   let dest = output_buffer(&dest_cfg)?;
   Ok((input, dest))
}

pub(crate) fn input_buffer(path: Option<&PathBuf>) -> Result<Box<dyn Read>, Error> {
   let buf = match path {
      Some(path) => {
         let file = fs::File::open(path).map_err(|source| Error::CouldNotOpenFile {
            path: path.to_owned(),
            reason: FileOpenReason::Read,
            source,
         })?;

         Box::new(BufReader::new(file)) as Box<dyn Read>
      }
      None => Box::new(BufReader::new(std::io::stdin())) as Box<dyn Read>,
   };

   Ok(buf)
}

fn output_buffer(dest_cfg: &DestCfg) -> Result<Dest, Error> {
   match dest_cfg {
      DestCfg::Stdout => Ok(Dest::Stdout(Box::new(std::io::stdout()) as Box<dyn Write>)),

      DestCfg::Path { buf: path, force } => {
         let dir = path.parent().ok_or_else(|| Error::InvalidDirectory {
            path: path.to_owned(),
         })?;

         fs::create_dir_all(dir).map_err(|source| Error::CreateDirectory {
            dir: dir.to_owned(),
            path: path.to_owned(),
            source,
         })?;

         // TODO: can I, without doing a TOCTOU, avoid overwriting an existing
         // file? (That's mostly academic, but since the point of this is to
         // learn, I want to learn that.)
         let file_exists = path
            .try_exists()
            .map_err(|source| Error::CheckFileExists { source })?;

         if file_exists && !force {
            return Err(Error::FileExists(path.to_owned()));
         }

         let file = fs::File::create(path).map_err(|source| Error::CouldNotOpenFile {
            path: path.clone(),
            reason: FileOpenReason::Write,
            source,
         })?;

         Ok(Dest::File {
            path: path.clone(),
            buf: Box::new(file) as Box<dyn Write>,
         })
      }
   }
}

#[derive(Debug)]
enum FileOpenReason {
   Read,
   Write,
}

impl std::fmt::Display for FileOpenReason {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      match self {
         FileOpenReason::Read => write!(f, "to read it"),
         FileOpenReason::Write => write!(f, "to write to it"),
      }
   }
}
