use std::{error, fmt, fs, io};

use camino::{Utf8Path, Utf8PathBuf};
use lazy_static::lazy_static;
use log::{debug, error, trace};
use rayon::{iter::Either, prelude::*};
use thiserror::Error;

use lx_md::Markdown;

use crate::{
   archive::{Archive, Order},
   canonicalized::Canonicalized,
   data::{
      config::{self, Config},
      item::cascade::{Cascade, CascadeLoadError},
   },
   error::write_to_fmt,
   page::{self, Page, Source},
   style, templates,
};

pub fn build_in(directory: Canonicalized) -> Result<(), Error> {
   let config = config_for(&directory)?;
   build(&directory, &config, &Markdown::new(None), Mode::Build)
}

pub fn config_for(source_dir: &Canonicalized) -> Result<Config, Error> {
   let config_path = source_dir.as_ref().join("config.lx.yaml");
   debug!("source path: {source_dir}");
   debug!("config path: {config_path}");
   let config = Config::from_file(&config_path)?;
   Ok(config)
}

pub enum Mode {
   Build,
   Serve,
}

// TODO: further split this apart.
pub fn build(
   directory: &Canonicalized,
   config: &Config,
   md: &Markdown,
   mode: Mode,
) -> Result<(), Error> {
   debug!("Building in {directory}");

   clear_output_dir(config, mode)?;

   let input_dir = directory.as_ref();
   let site_files = SiteFiles::in_dir(input_dir)?;
   trace!("Site files: {site_files}");

   let shared_dir = input_dir.parent().map(|parent| parent.join("_shared"));
   let mut shared_files = shared_dir
      .as_ref()
      .map(|dir| SharedFiles::in_dir(dir))
      .transpose()?;

   trace!(
      "Shared files: {}",
      match &shared_files {
         Some(files) => format!("{files}"),
         None => "none".into(),
      }
   );

   let mut all_templates = site_files.templates;
   if let Some(shared_files) = shared_files.as_mut() {
      all_templates.append(&mut shared_files.templates);
   }

   trace!("all templates: {all_templates:?}");

   let jinja_env = templates::load(all_templates, |path| {
      let site_ui_dir = input_dir.join(&*UI_DIR);
      if path.starts_with(&site_ui_dir) {
         Ok(path.strip_prefix(&site_ui_dir).unwrap())
      } else if let Some(shared_dir) = shared_dir.as_ref() {
         let shared_ui_dir = shared_dir.join(&*UI_DIR);
         if path.starts_with(&shared_ui_dir) {
            Ok(path.strip_prefix(&shared_ui_dir).unwrap())
         } else {
            Err(Error::TemplatePath {
               path: path.to_owned(),
            })?
         }
      } else {
         Err(Error::TemplatePath {
            path: path.to_owned(),
         })?
      }
   })?;

   fs::create_dir_all(&config.output).map_err(|source| Error::CreateDir {
      path: config.output.clone(),
      source,
   })?;

   let sources = load_sources(&site_files.content)?;

   debug!("loaded {count} pages", count = sources.len());

   let cascade =
      Cascade::new(&site_files.data).map_err(|source| Error::Cascade { source })?;

   let (errors, prepared_pages): (Vec<_>, Vec<_>) = sources
      .par_iter()
      // NOTE: this is where I will want to add handling for `<page>.lx.yaml` files; when
      // I add support for that, this will not be a filter but will do different things in
      // the map call depending on what kind of file it is.
      .filter(|source| source.path.extension().is_some_and(|ext| ext == "md"))
      .map(|source| {
         page::prepare(md, source, &cascade)
            .map(|prepared| (prepared, source))
            .map_err(|e| (source.path.clone(), e))
      })
      .partition_map(Either::from);

   if !errors.is_empty() {
      return Err(Error::preparing_page(errors));
   }

   debug!("prepared {count} pages", count = prepared_pages.len());

   // TODO: build taxonomies. Structurally, I *think* the best thing to do is
   // provide a top-level `Archive` and then filter on its results, since that
   // avoids having to do the sorting more than once. So build the taxonomies
   // *second*, as filtered versions of the Archive?

   let content_dir = input_dir.join("content");

   let (errors, pages): (Vec<_>, Vec<_>) = prepared_pages
      .into_par_iter()
      .map(|(prepared, source)| {
         // TODO: once the taxonomies exist, pass them here.
         prepared
            .render(md, |text, metadata| {
               let after_jinja = jinja_env
                  .render_str(text, metadata)
                  .map_err(|source| Error::rewrite(source, text))?;
               // TODO: smarten the typography!
               Ok(after_jinja)
            })
            .and_then(|rendered| Page::from_rendered(rendered, source, &content_dir))
            .map_err(|e| (source.path.clone(), e))
      })
      .partition_map(Either::from);

   if !errors.is_empty() {
      return Err(Error::rendering_page(errors));
   }

   // TODO: this is the wrong spot for this. There is enough info to generate this and
   // other such views above, now that I have split the phases apart.
   let _archive = Archive::new(&pages, Order::NewFirst);

   // TODO: this and the below are identical, except for the directory from which they
   // come. This is suggestive: maybe extract into a function for handling both, and
   // implement a trait for both to use. In that case, it would also very likely make
   // sense to include at least a reference to the source directory in the `shared_files`
   // and `site_files` structs.
   if let Some(shared) = shared_files.as_mut() {
      debug!("Copying {} shared static files", shared.static_files.len());
      for static_file in shared.static_files.iter() {
         let relative_path = static_file
            .strip_prefix(shared_dir.as_ref().unwrap().join("_static"))
            .map_err(|_| Error::StripPrefix {
               prefix: input_dir.to_owned(),
               path: static_file.clone(),
            })?;
         let path = config.output.join(relative_path);
         copy(static_file, &path)?;
      }
   }

   debug!("Copying {} static files", site_files.static_files.len());
   for static_file in site_files.static_files.iter() {
      let relative_path = static_file
         .strip_prefix(input_dir.join("_static"))
         .map_err(|_| Error::StripPrefix {
            prefix: input_dir.to_owned(),
            path: static_file.clone(),
         })?;
      let path = config.output.join(relative_path);
      copy(static_file, &path)?;
   }

   // TODO: this can and probably should use async?
   for page in pages {
      let relative_path = page.path.as_ref().join("index.html");
      let path = config.output.join(relative_path);

      trace!("writing page {} to {}", page.data.title, path);
      let containing_dir = path
         .parent()
         .unwrap_or_else(|| panic!("{path} should have a containing dir!"));

      fs::create_dir_all(containing_dir).map_err(|e| Error::CreateOutputDirectory {
         path: containing_dir.to_owned(),
         source: e,
      })?;

      let mut buf = Vec::new();
      templates::render(&jinja_env, &page, config, &mut buf)?;

      emit(&path, &buf)?;
   }

   for css_file in site_files
      .styles
      .into_iter()
      // only build the “root” files
      .filter(|path| {
         !path
            .file_name()
            .expect("all CSS files have file names")
            .starts_with("_")
      })
   {
      trace!("building CSS for {css_file}");
      let converted = style::convert(&css_file, style::OutputMode::Dev)?;
      let relative_path =
         css_file
            .strip_prefix(input_dir.join("_styles"))
            .map_err(|_| Error::StripPrefix {
               prefix: input_dir.to_owned(),
               path: css_file.clone(),
            })?;

      let path = config.output.join(relative_path).with_extension("css");
      emit(&path, &converted)?;
   }

   Ok(())
}

fn clear_output_dir(config: &Config, _mode: Mode) -> Result<(), Error> {
   // TODO: only do this if in `Mode::Build`; in `Mode::Serve`, clear in-memory cache
   //   instead.
   debug!("Removing output directory {}", config.output);
   if let Err(io_err) = fs::remove_dir_all(&config.output)
      && io_err.kind() != io::ErrorKind::NotFound
   {
      Err(Error::RemoveDir {
         source: io_err,
         path: config.output.clone(),
      })
   } else {
      Ok(())
   }
}

fn copy(from: &Utf8Path, to: &Utf8Path) -> Result<(), Error> {
   let output_dir = to.parent().expect("must have a real parent");
   fs::create_dir_all(output_dir).map_err(|source| Error::CreateOutputDirectory {
      path: output_dir.to_owned(),
      source,
   })?;
   fs::copy(from, to).map_err(|source| Error::CopyFile {
      from: from.to_owned(),
      to: to.to_owned(),
      source,
   })?;
   Ok(())
}

fn emit(path: &Utf8Path, content: impl AsRef<[u8]>) -> Result<(), Error> {
   let output_dir = path.parent().expect("must have a real parent");
   fs::create_dir_all(output_dir).map_err(|source| Error::CreateOutputDirectory {
      path: output_dir.to_owned(),
      source,
   })?;
   fs::write(path, content).map_err(|source| Error::WriteFile {
      path: path.to_owned(),
      source,
   })?;
   Ok(())
}

fn load_sources<S>(source_files: S) -> Result<Vec<Source>, Error>
where
   S: IntoIterator,
   S::Item: AsRef<Utf8Path>,
{
   let mut sources = Vec::new();
   let mut errors = Vec::new();
   for path in source_files {
      let path = path.as_ref();
      match fs::read_to_string(path) {
         Ok(contents) => sources.push(Source {
            path: path.to_owned(),
            contents,
         }),
         Err(e) => errors.push(ContentError {
            path: path.to_owned(),
            source: e,
         }),
      }
   }

   if errors.is_empty() {
      Ok(sources)
   } else {
      Err(Error::Content(errors))
   }
}

#[derive(Error, Debug)]
pub enum Error {
   #[error(transparent)]
   LoadTemplates {
      #[from]
      source: templates::Error,
   },

   #[error("could not rewrite {text} with minijinja")]
   Rewrite {
      text: String,
      source: minijinja::Error,
   },

   #[error("could not load data cascade")]
   Cascade {
      #[from]
      source: CascadeLoadError,
   },

   #[error("could not load site config: {source}")]
   Config {
      #[from]
      source: config::Error,
   },

   #[error("could not load one or more site content sources")]
   Content(Vec<ContentError>),

   #[error(transparent)]
   Page(PageError),

   #[error("could not create output directory '{path}'")]
   CreateOutputDirectory {
      path: Utf8PathBuf,
      source: io::Error,
   },

   #[error("could not copy from {from} to {to}")]
   CopyFile {
      from: Utf8PathBuf,
      to: Utf8PathBuf,
      source: io::Error,
   },

   #[error("could not write to {path}")]
   WriteFile {
      path: Utf8PathBuf,
      source: io::Error,
   },

   #[error("bad glob pattern: '{pattern}'")]
   GlobPattern {
      pattern: String,
      source: glob::PatternError,
   },

   #[error(transparent)]
   Glob { source: glob::GlobError },

   #[error("could not strip prefix '{prefix}' from path '{path}'")]
   StripPrefix {
      prefix: Utf8PathBuf,
      path: Utf8PathBuf,
   },

   #[error("error compiling CSS")]
   Styles {
      #[from]
      source: style::Error,
   },

   #[error("invalid template path {path}")]
   TemplatePath { path: Utf8PathBuf },

   #[error("could not delete directory '{path}'")]
   RemoveDir {
      path: Utf8PathBuf,
      source: io::Error,
   },

   #[error("could not create directory '{path}'")]
   CreateDir {
      path: Utf8PathBuf,
      source: io::Error,
   },

   #[error("Could not convert path '{}' to unicode: {source}", path.display())]
   NonUtf8Path {
      source: camino::FromPathBufError,
      path: std::path::PathBuf,
   },
}

impl Error {
   fn rewrite(
      source: minijinja::Error,
      text: &str,
   ) -> Box<dyn error::Error + Send + Sync> {
      Box::new(Error::Rewrite {
         source,
         text: text.to_owned(),
      })
   }

   fn preparing_page(errors: Vec<(Utf8PathBuf, page::Error)>) -> Error {
      Error::Page(PageError {
         errors,
         kind: PageErrorKind::Prepare,
      })
   }

   fn rendering_page(errors: Vec<(Utf8PathBuf, page::Error)>) -> Error {
      Error::Page(PageError {
         errors,
         kind: PageErrorKind::Render,
      })
   }
}

#[derive(Debug)]
enum PageErrorKind {
   Prepare,
   Render,
}

// This has its own type definition so that it can have its own `impl` for `fmt::Display`
// (see immediately below) so that `Error` can simply `#[derive(thiserror::Error)]`.
#[derive(Error, Debug)]
pub struct PageError {
   errors: Vec<(Utf8PathBuf, page::Error)>,
   kind: PageErrorKind,
}

impl fmt::Display for PageError {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let count = self.errors.len();
      match self.kind {
         PageErrorKind::Prepare => {
            writeln!(f, "could not prepare {count} pages for rendering")?
         }
         PageErrorKind::Render => writeln!(f, "could not render {count} pages")?,
      };

      for (path, error) in &self.errors {
         writeln!(f, "{path}:\n\t{error}")?;
         write_to_fmt(f, error)?;
      }

      Ok(())
   }
}

#[derive(Error, Debug)]
pub struct RewriteErrors(Vec<(Utf8PathBuf, minijinja::Error)>);

impl fmt::Display for RewriteErrors {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let errors = &self.0;
      writeln!(f, "could not rewrite {} pages", errors.len())?;
      for (path, error) in errors {
         writeln!(f, "{path}:\n\t{error}")?;
         write_to_fmt(f, error)?;
      }

      Ok(())
   }
}

#[derive(Error, Debug)]
#[error("Could not load file {path}")]
pub struct ContentError {
   source: io::Error,
   path: Utf8PathBuf,
}

lazy_static! {
   static ref UI_DIR: Utf8PathBuf = Utf8PathBuf::from("_ui");
}

struct SiteFiles {
   config: Utf8PathBuf,
   content: Vec<Utf8PathBuf>,
   data: Vec<Utf8PathBuf>,
   templates: Vec<Utf8PathBuf>,
   static_files: Vec<Utf8PathBuf>,
   styles: Vec<Utf8PathBuf>,
}

impl SiteFiles {
   fn in_dir(in_dir: &Utf8Path) -> Result<SiteFiles, Error> {
      let content_dir = in_dir.join("content");
      trace!("content_dir: {content_dir}");

      let data = resolved_paths_for(&format!("{content_dir}/**/_data.lx.yaml"))?;
      let content = resolved_paths_for(&format!("{content_dir}/**/*.md"))?
         .into_iter()
         .filter(|p| !data.contains(p))
         .collect();

      let site_files = SiteFiles {
         config: in_dir.join("config.lx.yaml"),
         content,
         data,
         templates: resolved_paths_for(&format!("{in_dir}/{}/*.jinja", *UI_DIR))?,
         static_files: resolved_paths_for(&format!("{in_dir}/_static/**/*"))?,
         styles: resolved_paths_for(&format!("{in_dir}/_styles/*.css"))?,
      };

      Ok(site_files)
   }
}

impl fmt::Display for SiteFiles {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      // Yes, I could do these alignments with format strings; maybe at some
      // point I will switch to that.
      writeln!(f)?;
      writeln!(f, "  config files:{}", self.config)?;
      writeln!(f, "  content files:{}", display_paths(&self.content))?;
      writeln!(f, "  data files:{}", display_paths(&self.data))?;
      writeln!(f, "  style files:{}", display_paths(&self.styles))?;
      writeln!(f, "  template files:{}", display_paths(&self.templates))?;
      Ok(())
   }
}

struct SharedFiles {
   templates: Vec<Utf8PathBuf>,
   static_files: Vec<Utf8PathBuf>,
   styles: Vec<Utf8PathBuf>,
}

impl SharedFiles {
   fn in_dir(dir: &Utf8Path) -> Result<SharedFiles, Error> {
      let site_files = SharedFiles {
         templates: resolved_paths_for(&format!("{dir}/{}/*.jinja", *UI_DIR))?,
         static_files: resolved_paths_for(&format!("{dir}/_static/**/*"))?,
         styles: resolved_paths_for(&format!("{dir}/_styles/**/*.scss"))?,
      };

      Ok(site_files)
   }
}

impl fmt::Display for SharedFiles {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let sep = String::from("\n      ");
      let empty = String::from(" (none)");

      // Yes, I could do these alignments with format strings; maybe at some
      // point I will switch to that.
      writeln!(f)?;
      writeln!(f, "  style files:{}", display_paths(&self.styles))?;
      writeln!(f, "  template files:{}", display_paths(&self.templates))?;
      Ok(())
   }
}

fn display_paths(paths: &[Utf8PathBuf]) -> String {
   if paths.is_empty() {
      return String::from(" (none)");
   }

   let sep = String::from("\n      ");
   let path_strings = paths
      .iter()
      .map(|path| path.to_string())
      .collect::<Vec<_>>()
      .join(&sep);

   sep + &path_strings
}

fn resolved_paths_for(glob_src: &str) -> Result<Vec<Utf8PathBuf>, Error> {
   glob::glob(glob_src)
      .map_err(|source| Error::GlobPattern {
         pattern: glob_src.to_string(),
         source,
      })?
      .try_fold(Vec::new(), |mut good, result| match result {
         Ok(path) => match Utf8PathBuf::try_from(path.clone()) {
            Ok(utf8_path) => {
               good.push(utf8_path);
               Ok(good)
            }
            Err(source) => Err(Error::NonUtf8Path { source, path }),
         },
         Err(source) => Err(Error::Glob { source }),
      })
      .map(|paths| paths.into_iter().filter(|path| path.is_file()).collect())
}
