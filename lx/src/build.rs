pub mod vfs;

use std::{error, fmt, fs, io};

use arborium::Highlighter;
use camino::{Utf8Path, Utf8PathBuf};
use jiff::{Timestamp, fmt::temporal::DateTimePrinter};
use lazy_static::lazy_static;
use log::{debug, trace};
use rayon::{iter::Either, prelude::*};
use thiserror::Error;

use crate::{
   archive::{self, Archive},
   canonicalized::Canonicalized,
   data::{
      config::{self, Config},
      item::cascade::{Cascade, CascadeLoadError},
   },
   error::write_to_fmt,
   live_reload,
   page::{self, Item, Kind, RenderInfo, Source},
   style, templates,
};

use vfs::{Provenance, Vfs};

pub fn build_in(directory: Canonicalized) -> Result<(), Error> {
   let config = config_for(&directory)?;
   build(&directory, &config, Mode::Build, None)
}

pub fn config_for(source_dir: &Canonicalized) -> Result<Config, Error> {
   let config_path = source_dir.as_ref().join("config.lx.yaml");
   debug!("source path: {source_dir}");
   debug!("config path: {config_path}");
   let config = Config::from_file(&config_path)?;
   Ok(config)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
   Build,
   Serve,
}

// TODO: further split this apart.
pub fn build(
   directory: &Canonicalized,
   config: &Config,
   mode: Mode,
   reusable_vcs: Option<&mut Vfs>,
) -> Result<(), Error> {
   debug!("Building in {directory}");

   let mut local_only_vfs = Vfs::new();
   let vfs = reusable_vcs.unwrap_or(&mut local_only_vfs);

   clear_output_dir(config, mode)?;

   let input_dir = directory.as_ref();
   let site_files = SiteFiles::in_dir(input_dir)?;
   trace!("Site files: {site_files}");

   trace!("all templates: {:?}", site_files.templates);

   let jinja_env = templates::load(site_files.templates, |path| {
      let site_ui_dir = input_dir.join(&*UI_DIR);
      if path.starts_with(&site_ui_dir) {
         Ok(path.strip_prefix(&site_ui_dir).unwrap())
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
      .map(|source| {
         page::prepare(source, &cascade, config)
            .map(|prepared| (prepared, source))
            .map_err(|e| (source.path.clone(), e))
      })
      .partition_map(Either::from);

   if !errors.is_empty() {
      return Err(Error::preparing_page(errors));
   }

   debug!("prepared {count} pages", count = prepared_pages.len());

   let content_dir = input_dir.join("content");

   let highlighter = Highlighter::new();

   let (errors, items): (Vec<_>, Vec<_>) = prepared_pages
      .into_par_iter()
      .map(|(prepared, source)| {
         // TODO: once the taxonomies exist, pass them here.
         prepared
            .process(&mut highlighter.fork(), |text, metadata| {
               // TODO: smarten the typography!
               let after_jinja = jinja_env
                  .render_str(text, metadata)
                  .map_err(|source| Error::rewrite(source, text))?;
               Ok(after_jinja)
            })
            .and_then(|processed| Item::from_processed(processed, source, &content_dir))
            .map_err(|e| (source.path.clone(), e))
      })
      .partition_map(Either::from);

   if !errors.is_empty() {
      return Err(Error::rendering_pages(errors));
   }

   // TODO: build taxonomies.
   // TODO: Identify the taxonomical system I want to use for the site(s)!
   let built_at = Timestamp::now();
   let posts = items.iter().filter_map(|item| match &item.content {
      page::ItemContent::Post(post) => Some(post),
      page::ItemContent::Page(_) | page::ItemContent::Raw(_) => None,
   });
   let archive = Archive::new(posts).map_err(Error::from)?;
   let render_info = RenderInfo {
      config,
      built_at: DateTimePrinter::new().timestamp_to_string(&built_at),
      archive: &archive,
   };

   let styles = site_files
      .styles
      .iter()
      // only build the “root” files
      .filter(|path| {
         // Only build the “root” CSS files; everything else is a partial pulled in
         // via `@import` (and is bundled into the root file).
         !path
            .file_name()
            .expect("all CSS files have file names")
            .starts_with('_')
            && path.parent() == Some(input_dir.join("_styles").as_path())
      })
      .cloned()
      .collect::<Vec<_>>();

   for static_file in &site_files.static_files {
      let relative_path = static_file
         .strip_prefix(input_dir.join("_static"))
         .map_err(|_| Error::StripPrefix {
            prefix: input_dir.to_owned(),
            path: static_file.clone(),
         })?;

      vfs.insert(relative_path, Provenance::Static(static_file.clone()))?;
   }

   for item in &items {
      let provenance = Provenance::Content(item.source().path.clone());
      vfs.insert(&item.output_path(), provenance)?;
   }

   for css_file in &styles {
      let relative_path = css_file
         .strip_prefix(input_dir.join("_styles"))
         .map_err(|_| Error::StripPrefix {
            prefix: input_dir.to_owned(),
            path: css_file.clone(),
         })?
         .with_extension("css");

      vfs.insert(&relative_path, Provenance::Style(css_file.clone()))?;
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
   for item in &items {
      let path = config.output.join(item.output_path());

      trace!(
         "writing page {} to {}",
         item.title().unwrap_or("<untitled>"),
         path
      );
      let containing_dir = path
         .parent()
         .unwrap_or_else(|| panic!("{path} should have a containing dir!"));

      fs::create_dir_all(containing_dir).map_err(|e| Error::CreateOutputDirectory {
         path: containing_dir.to_owned(),
         source: e,
      })?;

      let mut rendered = item.render(&jinja_env, &render_info)?;

      if item.produces_html() {
         live_reload::inject_for(mode, &mut rendered).map_err(|source| {
            Error::LiveReload {
               path: path.clone(),
               source,
            }
         })?;
      }

      emit(&path, &rendered)?;
   }

   for css_file in styles {
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
            kind: Kind::from_path(path),
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
   Item(#[from] page::Error),

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

   #[error("could not walk the content directory")]
   ContentWalk { source: ignore::Error },

   // Doesn't bother with the source error because it's [`std::path::StripPrefixError`],
   // which is fundamentally uninteresting (it's just a tuple struct with a private
   // internal `()` so it cannot be constructed other than by `strip_prefix`, and `camino`
   // delegates to it.
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

   #[error(transparent)]
   Archive {
      #[from]
      source: archive::Error,
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

   #[error("could not inject live reload into '{path}': {source}")]
   LiveReload {
      path: Utf8PathBuf,
      source: live_reload::Error,
   },

   #[error(transparent)]
   Vfs(#[from] vfs::Error),
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

   fn rendering_pages(errors: Vec<(Utf8PathBuf, page::Error)>) -> Error {
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

const DATA_FILE: &str = "_data.lx.yaml";

impl SiteFiles {
   fn in_dir(in_dir: &Utf8Path) -> Result<SiteFiles, Error> {
      let content_dir = in_dir.join("content");
      trace!("content_dir: {content_dir}");

      let data = resolved_paths_for(&format!("{content_dir}/**/{DATA_FILE}"))?;
      let content = content_files(&content_dir)?;

      let site_files = SiteFiles {
         config: in_dir.join("config.lx.yaml"),
         content,
         data,
         templates: resolved_paths_for(&format!("{in_dir}/{}/**/*.jinja", *UI_DIR))?,
         static_files: resolved_paths_for(&format!("{in_dir}/_static/**/*"))?,
         styles: resolved_paths_for(&format!("{in_dir}/_styles/**/*.css"))?,
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

fn content_files(content_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
   let mut files = Vec::new();

   // Allow hidden files; leave all the other default filters in place.
   for entry in ignore::WalkBuilder::new(content_dir).hidden(false).build() {
      let entry = entry.map_err(|source| Error::ContentWalk { source })?;

      if entry
         .file_type()
         .is_some_and(|file_type| file_type.is_file())
      {
         let path = Utf8PathBuf::try_from(entry.into_path()).map_err(|source| {
            Error::NonUtf8Path {
               path: source.as_path().to_owned(),
               source,
            }
         })?;

         if path.file_name() != Some(DATA_FILE) {
            files.push(path);
         }
      }
   }

   files.sort();
   Ok(files)
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

#[cfg(test)]
mod tests {
   use std::sync::atomic::{AtomicUsize, Ordering};

   use indoc::formatdoc;

   use super::*;

   #[test]
   fn content_walk_skips_hidden_ignored_and_data_files() {
      let site = Site::new();

      site.write("_ui/base.jinja", "{{ content }}");
      site.write("content/index.md", "---\ntitle: Index\n---\nbody");
      site.write("content/.hidden.file", "---\ntitle: Hidden\n---\nbody");
      site.write("content/ignored.md", "---\ntitle: Ignored\n---\nbody");
      site.write("content/.ignore", "ignored.md\n.ignore");
      site.write(&format!("content/nested/{DATA_FILE}"), "title: Nested");

      site.build();

      assert_eq!(site.outputs(), vec![".hidden.file", "index/index.html"]);
   }

   #[test]
   fn raw_content_is_copied_verbatim() {
      let site = Site::new();
      let source = "{\n  \"a\": 1\n}\n";
      site.write("content/data/thing.json", source);

      site.build();

      let copied = fs::read_to_string(site.root.join("public/data/thing.json")).unwrap();
      assert_eq!(copied, source);
   }

   #[test]
   fn build_rejects_duplicate_outputs() {
      let site = Site::new();

      site.write("content/foo.json", "{}");
      site.write("_static/foo.json", "{}");

      let error = site.try_build().unwrap_err();

      assert!(matches!(
         error,
         Error::Vfs(vfs::Error::DuplicateOutput {
            existing: vfs::Provenance::Static(_),
            incoming: vfs::Provenance::Content(_),
            ..
         })
      ));
   }

   #[test]
   fn build_rejects_a_stylesheet_colliding_with_a_static_file() {
      let site = Site::new();

      site.write("content/notes.txt", "hi");
      site.write("_static/main.css", "body { color: red; }");
      site.write("_styles/main.css", "body { color: blue; }");

      let error = site.try_build().unwrap_err();

      assert!(matches!(
         error,
         Error::Vfs(vfs::Error::DuplicateOutput {
            existing: vfs::Provenance::Static(_),
            incoming: vfs::Provenance::Style(_),
            ..
         })
      ));
   }

   #[test]
   fn build_rejects_a_stylesheet_colliding_with_content() {
      let site = Site::new();

      site.write("content/main.css", "body { color: red; }");
      site.write("_styles/main.css", "body { color: blue; }");

      let error = site.try_build().unwrap_err();

      assert!(matches!(
         error,
         Error::Vfs(vfs::Error::DuplicateOutput {
            existing: vfs::Provenance::Content(_),
            incoming: vfs::Provenance::Style(_),
            ..
         })
      ));
   }

   #[test]
   fn build_rejects_conflicting_outputs() {
      let site = Site::new();

      site.write("_ui/base.jinja", "{{ content }}");
      site.write("content/foo", "raw");
      site.write("content/foo.md", "---\ntitle: Foo\n---\nbody");

      assert!(matches!(
         site.try_build(),
         Err(Error::Vfs(vfs::Error::ConflictingOutputs { .. }))
      ));
   }

   #[test]
   fn live_reload_applies_only_to_html_outputs() {
      let site = Site::new();

      site.write("_ui/base.jinja", "<html><body>{{ content }}</body></html>");
      site.write("_ui/feed.jinja", "<feed>{{ content }}</feed>");
      site.write("_ui/page.jinja", "<html><body>{{ content }}</body></html>");
      site.write("content/index.md", "---\ntitle: Index\n---\nbody");
      site.write("content/feed.lx.xml", "---\nlayout: feed.jinja\n---\nbody");
      site.write("content/page.lx.html", "---\nlayout: page.jinja\n---\nbody");
      site.write("content/robots.html", "<html><body>raw</body></html>");

      site.serve();

      let rendered =
         |path: &str| fs::read_to_string(site.root.join("public").join(path)).unwrap();

      assert!(rendered("index/index.html").contains("live-reload.js"));
      assert!(rendered("page.html").contains("live-reload.js"));
      assert!(!rendered("feed.xml").contains("live-reload.js"));
      assert!(!rendered("robots.html").contains("live-reload.js"));
   }

   struct Site {
      root: Utf8PathBuf,
   }

   impl Site {
      fn new() -> Self {
         static COUNTER: AtomicUsize = AtomicUsize::new(0);
         let root = Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!(
               "lx-{}-{}",
               std::process::id(),
               COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
         fs::create_dir_all(&root).unwrap();

         let site = Site { root };

         site.write(
            "config.lx.yaml",
            &formatdoc! {r#"
               url: https://example.com
               repo: https://example.com/repo
               title: {{ normal: Example }}
               description: Example
               author:
                  name: Example
                  email: hello@example.com
                  links: {{}}
               output: {}/public
               time_zone: America/Denver
               image:
                  url: https://example.com/image.png
               "#,
               site.root
            },
         );

         site
      }

      fn write(&self, path: &str, contents: &str) {
         let path = self.root.join(path);
         fs::create_dir_all(path.parent().unwrap()).unwrap();
         fs::write(&path, contents).unwrap();
      }

      fn build(&self) {
         self.build_in(Mode::Build).unwrap();
      }

      fn serve(&self) {
         self.build_in(Mode::Serve).unwrap();
      }

      fn try_build(&self) -> Result<(), Error> {
         self.build_in(Mode::Build)
      }

      fn build_in(&self, mode: Mode) -> Result<(), Error> {
         let directory = Canonicalized::try_from(self.root.clone()).unwrap();
         let config = config_for(&directory).unwrap();
         build(&directory, &config, mode, None)
      }

      fn outputs(&self) -> Vec<String> {
         let mut paths = Vec::new();
         let mut dirs = vec![self.root.join("public")];

         while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
               let path = Utf8PathBuf::from_path_buf(entry.unwrap().path()).unwrap();
               if path.is_dir() {
                  dirs.push(path);
               } else {
                  paths.push(
                     path
                        .strip_prefix(self.root.join("public"))
                        .unwrap()
                        .to_string(),
                  );
               }
            }
         }

         paths.sort();
         paths
      }
   }

   impl Drop for Site {
      fn drop(&mut self) {
         fs::remove_dir_all(&self.root).unwrap();
      }
   }
}
