use crate::{
   archive::Archive,
   data::{
      config::Config,
      item::{self, Metadata, Slug, cascade::Cascade, serial},
   },
   datetime::Date,
   templates,
};
use arborium::Highlighter;
use camino::{Utf8Path, Utf8PathBuf};
use json_feed::Author;
use lx_md::{self, RenderError, ToRender};
use minijinja::Environment;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, error};
use std::{collections::HashMap, fmt, hash::Hash, os::unix::prelude::OsStrExt};
use thiserror::Error;
use uuid::Uuid;

pub fn prepare<'e>(
   source: &'e Source,
   cascade: &Cascade,
   config: &Config,
) -> Result<Prepared<'e>, Error> {
   match &source.kind {
      Kind::Markdown => prepare_markdown(source, cascade, config).map(Prepared::page),
      Kind::Templated(_) => prepare_template(source, cascade, config).map(Prepared::page),
      Kind::Raw => Ok(Prepared::Raw(source)),
   }
}

fn prepare_markdown<'e>(
   source: &'e Source,
   cascade: &Cascade,
   config: &Config,
) -> Result<PreparedPage<'e>, Error> {
   let lx_md::Prepared {
      metadata_src,
      to_render,
   } = lx_md::prepare(&source.contents)?;

   let (data, date) = metadata_src
      .ok_or(Error::MissingMetadata)
      .and_then(|src| serial::Item::try_parse(&src).map_err(Error::from))
      .and_then(|item_metadata| {
         Metadata::resolved(
            item_metadata,
            source,
            cascade,
            Some(String::from("base.jinja")), // TODO: not this
            config,
         )
         .map_err(Error::from)
      })?;

   if data.title.is_none() {
      return Err(item::Error::MissingRequiredField { name: "title" }.into());
   }

   Ok(PreparedPage {
      data,
      date,
      body: Body::Markdown(to_render),
   })
}

fn prepare_template<'e>(
   source: &'e Source,
   cascade: &Cascade,
   config: &Config,
) -> Result<PreparedPage<'e>, Error> {
   let front_matter = FrontMatter::parse(&source.contents)?;

   let item_metadata = match front_matter.metadata {
      Some(src) => serial::Item::try_parse(src)?,
      None => serial::Item::default(),
   };

   let (data, date) = Metadata::resolved(item_metadata, source, cascade, None, config)
      .map_err(Error::from)?;

   Ok(PreparedPage {
      data,
      date,
      body: Body::Template(front_matter.body),
   })
}

struct FrontMatter<'s> {
   metadata: Option<&'s str>,
   body: &'s str,
}

impl<'s> FrontMatter<'s> {
   fn parse(source: &'s str) -> Result<Self, Error> {
      let mut lines = source.split_inclusive('\n');
      let opening = lines.next().unwrap_or(source);

      if !is_fence_line(opening) {
         return Ok(Self {
            metadata: None,
            body: source,
         });
      }

      let metadata_start = opening.len();
      let mut offset = metadata_start;

      for line in lines {
         let end = offset + line.len();

         if is_fence_line(line) {
            let metadata = &source[metadata_start..offset];

            return Ok(Self {
               metadata: (!metadata.trim().is_empty()).then_some(metadata),
               body: &source[end..],
            });
         }

         offset = end;
      }

      Err(Error::UnclosedFrontMatter)
   }
}

fn is_fence_line(line: &str) -> bool {
   line.strip_suffix('\n').unwrap_or(line) == "---"
}

pub enum Body<'e> {
   Markdown(ToRender<'e>),
   Template(&'e str),
}

pub enum Prepared<'e> {
   Page(Box<PreparedPage<'e>>),
   Raw(&'e Source),
}

impl<'e> Prepared<'e> {
   fn page(page: PreparedPage<'e>) -> Self {
      Prepared::Page(Box::new(page))
   }
}

impl<'e> Prepared<'e> {
   pub fn process(
      self,
      highlighter: &mut Highlighter,
      postprocess: impl Fn(
         &str,
         &Metadata,
      ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
   ) -> Result<Processed, Error> {
      match self {
         Prepared::Page(page) => {
            let PreparedPage { data, date, body } = *page;

            let content = match body {
               Body::Markdown(to_render) => {
                  lx_md::emit(to_render, Some(highlighter), |text| {
                     postprocess(text, &data)
                  })
                  .map(Content::FromMd)?
               }

               Body::Template(template) => postprocess(template, &data)
                  .map_err(|source| Error::Postprocess {
                     source,
                     original: template.to_string(),
                  })
                  .map(Content::FromTemplate)?,
            };

            Ok(Processed::item(content, date, data))
         }

         Prepared::Raw(source) => Ok(Processed::Raw {
            content: source.contents.to_string(),
         }),
      }
   }
}

pub struct PreparedPage<'e> {
   /// The fully-parsed metadata associated with the item.
   data: Metadata,

   /// The date and time of the item, if any.
   date: Option<Date>,

   body: Body<'e>,
}

pub enum Processed {
   Item(Box<ProcessedItem>),
   Raw { content: String },
}

impl Processed {
   fn item(content: Content, date: Option<Date>, data: Metadata) -> Processed {
      Processed::Item(Box::new(ProcessedItem {
         content,
         date,
         data,
      }))
   }
}

pub struct ProcessedItem {
   content: Content,
   date: Option<Date>,
   data: Metadata,
}

#[derive(Debug)]
pub enum Content {
   FromMd(lx_md::Rendered),
   FromTemplate(String),
}

impl AsRef<str> for Content {
   fn as_ref(&self) -> &str {
      match self {
         Content::FromMd(rendered_md) => rendered_md.html(),
         Content::FromTemplate(body) => body,
      }
   }
}

impl serde::Serialize for Content {
   fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
   where
      S: serde::Serializer,
   {
      serializer.serialize_str(self.as_ref())
   }
}

/// Source data for a file: where it came from, and its original contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
   /// Original source location for the file.
   pub path: Utf8PathBuf,
   /// Original contents of the file.
   pub contents: String,
   /// What kind of file are we working with?
   pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
   Markdown,
   Templated(Utf8PathBuf),
   Raw,
}

impl Kind {
   pub fn from_path<P: AsRef<Utf8Path>>(path: P) -> Self {
      let path = path.as_ref();
      let extensions: Vec<&str> = path
         .file_name()
         .map(|file_name| file_name.split('.').skip(1).collect())
         .unwrap_or_default();

      if extensions.last() == Some(&"md") {
         Kind::Markdown
      } else if extensions.contains(&"lx") {
         let stem = Utf8PathBuf::from(path.file_prefix().unwrap_or_default());
         let extension = Utf8PathBuf::from(
            extensions
               .into_iter()
               .filter(|&ext| ext != "lx")
               .collect::<Vec<_>>()
               .join("."),
         );
         let file_name = stem.with_added_extension(&extension);
         let output_path = path.with_file_name(file_name);
         Kind::Templated(output_path)
      } else {
         Kind::Raw
      }
   }
}

/// A unique identifier for an item (page, post, etc.).
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Deserialize, Serialize)]
pub struct Id(Uuid);

impl fmt::Display for Id {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "{}", self.0)
   }
}

pub struct Item<'s> {
   source: &'s Source,
   path: RootedPath,
   pub content: ItemContent,
}

pub enum ItemContent {
   Page(Page),
   Post(Post),
   Raw(String),
}

/// A fully resolved representation of a page.
///
/// In this struct, the metadata has been parsed and resolved, and the content has been
/// converted from Markdown to HTML and preprocessed with both the templating engine and
/// my typography tooling. It is ready to render into the target layout template specified
/// by its [`Metadata`] and then to print to the file system.
#[derive(Debug, Serialize)]
pub struct Page {
   pub id: Id,

   /// The fully parsed metadata associated with the page.
   pub data: Metadata,

   /// The fully rendered contents of the page.
   pub content: Content,
}

/// A `Post` is simply a [`Page`] that has a date.
#[derive(Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Post {
   pub page: Page,
   pub date: Date,
}

impl<'s> Item<'s> {
   pub fn from_processed(
      rendered: Processed,
      source: &'s Source,
      in_dir: &Utf8Path,
   ) -> Result<Item<'s>, Error> {
      // TODO: use explicit ID if supplied.
      let id = Id(Uuid::new_v5(
         &Uuid::NAMESPACE_OID,
         source.path.as_os_str().as_bytes(),
      ));

      let (item_content, path) = match rendered {
         Processed::Item(item) => {
            let ProcessedItem {
               content,
               date,
               data,
            } = *item;
            let path = RootedPath::new(&data.slug, in_dir)?;
            let page = Page { id, content, data };

            let item_content = match date {
               Some(date) => ItemContent::Post(Post { date, page }),
               None => ItemContent::Page(page),
            };

            (item_content, path)
         }
         Processed::Raw { content } => {
            let slug = Slug::from_path(&source.path)?;
            let path = RootedPath::new(&slug, in_dir)?;
            let item_content = ItemContent::Raw(content);
            (item_content, path)
         }
      };

      Ok(Item {
         source,
         path,
         content: item_content,
      })
   }

   pub fn render(&self, env: &Environment, info: &RenderInfo) -> Result<String, Error> {
      match &self.content {
         ItemContent::Page(page) | ItemContent::Post(Post { page, .. }) => {
            match &page.data.layout {
               Some(layout) => Context::new(self, info).render(env, layout),
               None => Context::new(self, info).render_free(env, &page.content),
            }
            .map_err(|source| Error::RenderTemplate {
               path: self.path.as_ref().to_owned(),
               source,
            })
         }

         ItemContent::Raw(body) => Ok(body.clone()),
      }
   }

   pub fn path(&self) -> &RootedPath {
      &self.path
   }

   /// The path the item writes to, relative to the output root.
   pub fn output_path(&self) -> Utf8PathBuf {
      match &self.source.kind {
         // For Markdown source files, I emit `foo.md` at `foo/index.html` so that the
         // input is always in predictable nice location that does *not* include `.html`
         // as its suffix, by dint of how both the dev server and the production servers
         // I use work.
         Kind::Markdown => self.path.as_ref().with_extension("").join("index.html"),

         // For everything else, their (slugified) path is already ready to go.
         Kind::Templated(_) | Kind::Raw => self.path.as_ref().to_owned(),
      }
   }

   pub fn source(&self) -> &Source {
      self.source
   }

   pub fn title(&self) -> Option<&str> {
      self.data()?.title.as_deref()
   }

   pub fn data(&self) -> Option<&Metadata> {
      match &self.content {
         ItemContent::Page(page) => Some(&page.data),
         ItemContent::Post(post) => Some(&post.page.data),
         ItemContent::Raw(_) => None,
      }
   }
}

pub struct RenderInfo<'a> {
   pub config: &'a Config,
   pub built_at: String,
   pub archive: &'a Archive<'a>,
}

enum Context<'a, 's> {
   Page(PageContext<'a, 's>),
   Post(PostContext<'a, 's>),
   Raw(RawContext<'s>),
}

#[derive(Debug, Serialize)]
struct PageContext<'a, 's> {
   content: &'s str,
   data: &'s Metadata,
   path: &'s RootedPath,
   source: &'s Source,
   config: &'s Config,
   build_date: &'s str,
   archive: &'s Archive<'a>,
}

#[derive(Debug, Serialize)]
struct PostContext<'a, 's> {
   content: &'s str,
   date: &'s Date,
   data: &'s Metadata,
   path: &'s RootedPath,
   source: &'s Source,
   config: &'s Config,
   build_date: &'s str,
   archive: &'s Archive<'a>,
}

#[derive(Debug, Serialize)]
struct RawContext<'s> {
   content: &'s str,
   path: &'s RootedPath,
   source: &'s Source,
   config: &'s Config,
   build_date: &'s str,
}

impl<'a, 's> Context<'a, 's> {
   fn new(item: &'s Item<'s>, render_info: &'s RenderInfo<'a>) -> Context<'a, 's> {
      match &item.content {
         ItemContent::Page(page) => Context::Page(PageContext {
            content: page.content.as_ref(),
            data: &page.data,
            path: item.path(),
            source: item.source(),
            config: render_info.config,
            build_date: &render_info.built_at,
            archive: render_info.archive,
         }),
         ItemContent::Post(post) => Context::Post(PostContext {
            content: post.page.content.as_ref(),
            date: &post.date,
            data: &post.page.data,
            path: item.path(),
            source: item.source(),
            config: render_info.config,
            build_date: &render_info.built_at,
            archive: render_info.archive,
         }),
         ItemContent::Raw(content) => Context::Raw(RawContext {
            content: content.as_ref(),
            path: item.path(),
            source: item.source(),
            config: render_info.config,
            build_date: &render_info.built_at,
         }),
      }
   }

   fn render(&self, env: &Environment, layout: &str) -> Result<String, templates::Error> {
      match self {
         Context::Page(ctx) => templates::render(env, layout, ctx),
         Context::Post(ctx) => templates::render(env, layout, ctx),
         Context::Raw(ctx) => templates::render(env, layout, ctx),
      }
   }

   fn render_free(
      &self,
      env: &Environment<'_>,
      content: &Content,
   ) -> Result<String, templates::Error> {
      match self {
         Context::Page(ctx) => templates::render_free(content, env, ctx),
         Context::Post(ctx) => templates::render_free(content, env, ctx),
         Context::Raw(ctx) => templates::render_free(content, env, ctx),
      }
   }
}

// NOTE: the following all assume stable, unique identifiers for pages. The existing
//   implementation *does* handle this because it creates a v5 UUID from the path to the
//   file, which by definition must be unique (you cannot have two files with the same
//   path in any file system I know of!), but this only holds as long as that does.
impl PartialEq for Page {
   fn eq(&self, other: &Self) -> bool {
      self.id == other.id
   }
}

impl Eq for Page {}

impl Hash for Page {
   fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
      self.id.hash(state);
   }
}

impl PartialOrd for Post {
   fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
      Some(self.cmp(other))
   }
}

impl Ord for Post {
   fn cmp(&self, other: &Self) -> Ordering {
      self.date.cmp(&other.date)
   }
}

#[derive(Debug, Serialize, Hash, PartialEq, Eq)]
pub struct PostLink<'a> {
   anchor_title: String,
   slug: &'a Slug,
}

impl<'a> From<&'a Post> for PostLink<'a> {
   fn from(value: &'a Post) -> PostLink<'a> {
      PostLink {
         anchor_title: match &value.page.data.link {
            Some(url) => format!("link to {url}"),
            None => String::from("post permalink"),
         },
         slug: &value.page.data.slug,
      }
   }
}

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not prepare Markdown for parsing")]
   Preparation {
      #[from]
      source: lx_md::Error,
   },

   #[error("no metadata")]
   MissingMetadata,

   #[error(transparent)]
   MetadataParsing {
      #[from]
      source: serial::ItemParseError,
   },

   #[error("could not resolve metadata")]
   MetadataResolution {
      #[from]
      source: item::Error,
   },

   #[error(transparent)]
   RenderMarkdown {
      #[from]
      source: RenderError,
   },

   #[error("front matter fence is never closed")]
   UnclosedFrontMatter,

   #[error("could not render template for item at {path}: {source}")]
   RenderTemplate {
      path: Utf8PathBuf,
      source: templates::Error,
   },

   #[error("Invalid combination of root '{root}' and slug '{slug}'")]
   BadSlugRoot {
      source: std::path::StripPrefixError,
      root: Utf8PathBuf,
      slug: Utf8PathBuf,
   },

   #[error("error rendering Jinja template")]
   Postprocess {
      source: Box<dyn error::Error + Send + Sync>,
      original: String,
   },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RootedPath(Utf8PathBuf);

impl RootedPath {
   pub fn new(slug: &Slug, root_dir: &Utf8Path) -> Result<RootedPath, Error> {
      match slug {
         Slug::Permalink(str) => Ok(RootedPath(Utf8PathBuf::from(str))),
         Slug::FromPath(path_buf) => path_buf
            .strip_prefix(root_dir)
            .map(|path| RootedPath(path.to_owned()))
            .map_err(|source| Error::BadSlugRoot {
               source,
               root: root_dir.to_owned(),
               slug: path_buf.to_owned(),
            }),
      }
   }

   /// Given a config, generate the (canonicalized) URL for the rooted path
   pub fn url(&self, config: &Config) -> String {
      String::from(config.url.trim_end_matches('/')) + "/" + self.0.as_str()
   }
}

impl AsRef<Utf8Path> for RootedPath {
   fn as_ref(&self) -> &Utf8Path {
      &self.0
   }
}

/// Convenience to allow `From` for `FeedItem`.
pub struct PostAndConfig<'p, 'c>(pub &'p Post, pub PostInfo<'p>, pub &'c Config);

pub struct PostInfo<'a> {
   pub source: &'a Source,
   pub path: &'a RootedPath,
}

impl From<PostAndConfig<'_, '_>> for json_feed::FeedItem {
   fn from(PostAndConfig(post, info, config): PostAndConfig) -> Self {
      json_feed::FeedItem {
         id: post.page.id.to_string(),
         url: Some(info.path.url(config)),
         external_url: None, // TODO: support for page.link etc.
         title: post.page.data.title.clone(),
         content_text: None, // TODO: use this for microblogging?
         content_html: Some(post.page.content.as_ref().to_string()),
         summary: post
            .page
            .data
            .summary
            .as_ref()
            .map(|summary| summary.plain()),
         image: post
            .page
            .data
            .image
            .as_ref()
            .map(|image| image.url().to_string()),
         banner_image: None, // TODO: add support for these if I care?
         date_published: Some(post.date.to_rfc3339()),
         date_modified: post
            .page
            .data
            .updated
            .last()
            .map(|update| update.at.to_rfc3339()),
         author: Some(Author::All {
            avatar: "https://cdn.chriskrycho.com/images/avatars/2024%20600%C3%97600.jpeg"
               .to_string(),
            name: "Chris Krycho".to_string(),
            url: "https://www.chriskrycho.com".to_string(),
         }),
         tags: Some(post.page.data.tags.clone()),
         attachments: None,
      }
   }
}

#[allow(dead_code)] // till I use it!
#[derive(Debug, Serialize, Deserialize)]
pub struct Collections(HashMap<Id, crate::collection::Id>);

#[cfg(test)]
mod tests {
   use indoc::indoc;
   use minijinja::Environment;

   use super::*;

   fn source(path: &str, contents: &str) -> Source {
      Source {
         path: Utf8PathBuf::from(path),
         contents: String::from(contents),
         kind: Kind::from_path(path),
      }
   }

   fn cascade() -> Cascade {
      Cascade::new(&[]).unwrap()
   }

   fn config() -> Config {
      serde_yaml::from_str(indoc! {r#"
         url: https://example.com
         repo: https://example.com/repo
         title: Example
         description: Example
         author:
           name: Example
           email: hello@example.com
           links: {}
         output: public
         time_zone: America/Denver
         image:
           url: https://example.com/image.png
      "#})
      .unwrap()
   }

   fn post(name: &str, date: &str) -> Source {
      source(
         &format!("content/{name}.md"),
         &format!("---\ntitle: {name}\ndate: {date}\n---\nbody"),
      )
   }

   fn item<'e>(source: &'e Source, cascade: &Cascade) -> Item<'e> {
      prepare(source, cascade, &config())
         .unwrap()
         .process(&mut Highlighter::new(), |text, _| Ok(text.to_owned()))
         .and_then(|processed| {
            Item::from_processed(processed, source, Utf8Path::new("content"))
         })
         .unwrap()
   }

   fn body<'a>(item: &'a Item<'a>) -> &'a str {
      match &item.content {
         ItemContent::Page(page) => page.content.as_ref(),
         ItemContent::Post(post) => post.page.content.as_ref(),
         ItemContent::Raw(content) => content.as_ref(),
      }
   }

   fn assert_templated(source: &str, output: &str) {
      assert_eq!(
         Kind::from_path(source),
         Kind::Templated(Utf8PathBuf::from(output))
      );
   }

   fn output_path(source: &Source, cascade: &Cascade) -> String {
      item(source, cascade).output_path().to_string()
   }

   fn render_info<'a>(config: &'a Config, archive: &'a Archive<'a>) -> RenderInfo<'a> {
      RenderInfo {
         config,
         built_at: "2024-01-01T00:00:00+00:00".into(),
         archive,
      }
   }

   #[test]
   fn classifies_markdown_as_primary_content() {
      assert_eq!(Kind::from_path("content/index.md"), Kind::Markdown);
      assert_eq!(Kind::from_path("content/essays/jj init.md"), Kind::Markdown);
   }

   #[test]
   fn markdown_wins_over_explicit_templating() {
      assert_eq!(Kind::from_path("content/foo.lx.md"), Kind::Markdown);
   }

   #[test]
   fn classifies_terminal_lx_as_templated() {
      assert_templated("content/standalone.lx", "content/standalone");
   }

   #[test]
   fn classifies_lx_in_extension_chain_as_templated() {
      assert_templated("content/feed.lx.xml", "content/feed.xml");
      assert_templated("content/foo.bar.lx.xml", "content/foo.bar.xml");
      assert_templated(
         "content/foo.lx.fancy.extension.chain",
         "content/foo.fancy.extension.chain",
      );
   }

   #[test]
   fn does_not_mistake_a_named_stem_for_an_lx_extension() {
      assert_eq!(Kind::from_path("content/lx.xml"), Kind::Raw);
      assert_eq!(Kind::from_path("content/md"), Kind::Raw);
   }

   #[test]
   fn classifies_everything_else_as_raw() {
      assert_eq!(Kind::from_path("content/foo.xml"), Kind::Raw);
      assert_eq!(Kind::from_path("content/foo.html"), Kind::Raw);
      assert_eq!(Kind::from_path("content/data.json"), Kind::Raw);
      assert_eq!(Kind::from_path("content/extensionless"), Kind::Raw);
   }

   #[test]
   fn removes_every_intermediate_lx() {
      assert_templated("content/foo.lx.lx.xml", "content/foo.xml");
      assert_templated("content/nested/thing.lx.json", "content/nested/thing.json");
   }

   #[test]
   fn preserves_lx_outside_extension_positions() {
      assert_templated("content/foo.lxml.lx.xml", "content/foo.lxml.xml");
      assert_templated("content/a.lx.b/foo.lx.xml", "content/a.lx.b/foo.xml");
      assert_templated(
         "content/my-cool-thing.lx.lx.lx.so-lx-tastic.cool",
         "content/my-cool-thing.so-lx-tastic.cool",
      );
   }

   #[test]
   fn templated_metadata_slugs_from_its_output_path() {
      let source = source("content/feed.lx.xml", "body");
      let item = item(&source, &cascade());

      assert_eq!(
         item.data().unwrap().slug,
         Slug::FromPath(Utf8PathBuf::from("content/feed.xml"))
      );
   }

   #[test]
   fn permalink_overrides_every_other_slug_source() {
      let markdown = source(
         "content/journal/Hello World.md",
         "---\ntitle: Hello\npermalink: /hello/\n---\nbody",
      );
      assert_eq!(item(&markdown, &cascade()).path().as_ref(), "hello");

      let templated = source("content/feed.lx.xml", "---\npermalink: /feed\n---\nbody");
      assert_eq!(item(&templated, &cascade()).path().as_ref(), "feed");
   }

   #[test]
   fn markdown_outputs_index_html_inside_its_directory() {
      let source = source(
         "content/journal/Hello World.md",
         "---\ntitle: Hello\n---\nbody",
      );

      assert_eq!(
         output_path(&source, &cascade()),
         "journal/hello-world/index.html"
      );
   }

   #[test]
   fn templated_outputs_directly_at_its_derived_path() {
      let feed = source("content/feed.lx.xml", "---\nlayout: feed.jinja\n---\nbody");
      assert_eq!(output_path(&feed, &cascade()), "feed.xml");

      let standalone = source("content/standalone.lx", "body");
      assert_eq!(output_path(&standalone, &cascade()), "standalone");

      let nested = source("content/nested/thing.lx.json", "body");
      assert_eq!(output_path(&nested, &cascade()), "nested/thing.json");
   }

   #[test]
   fn raw_outputs_at_its_source_path() {
      let source = source("content/data.json", "{}");

      assert_eq!(output_path(&source, &cascade()), "data.json");
   }

   #[test]
   fn prepares_raw_sources_without_rendering() {
      let source = source("content/data.json", "{}");
      let raw = item(&source, &cascade());

      assert!(matches!(raw.content, ItemContent::Raw(_)));
      assert_eq!(body(&raw), "{}");
   }

   #[test]
   fn parses_fenced_front_matter() {
      let source = source(
         "content/feed.lx.xml",
         "---\nlayout: feed.jinja\ntitle: Feed\n---\nbody",
      );
      let item = item(&source, &cascade());
      let data = item.data().unwrap();

      assert_eq!(data.title.as_deref(), Some("Feed"));
      assert_eq!(data.layout.as_deref(), Some("feed.jinja"));
      assert_eq!(body(&item), "body");
   }

   #[test]
   fn absent_fence_treats_the_whole_source_as_body() {
      let source = source("content/body.lx", "just a body");
      let item = item(&source, &cascade());
      let data = item.data().unwrap();

      assert_eq!(data.title, None);
      assert_eq!(data.layout, None);
      assert_eq!(body(&item), "just a body");
   }

   #[test]
   fn empty_front_matter_is_an_empty_mapping() {
      let source = source("content/empty.lx", "---\n---\nbody");
      let item = item(&source, &cascade());

      assert_eq!(item.title(), None);
      assert_eq!(body(&item), "body");
   }

   #[test]
   fn preserves_multiline_bodies() {
      let source = source(
         "content/multiline.lx",
         "---\nlayout: base.jinja\n---\nhello\nworld\n",
      );
      let item = item(&source, &cascade());

      assert_eq!(body(&item), "hello\nworld\n");
   }

   #[test]
   fn renders_layouts_with_build_context() {
      let sources = vec![
         post("a", "2022-01-01T00:00:00-05:00"),
         post("b", "2023-01-01T00:00:00-05:00"),
      ];
      let mut cascade = cascade();
      cascade.add_at(
         "content",
         serial::Ambient {
            layout: Some("wrapper.jinja".into()),
            ..Default::default()
         },
      );

      let items: Vec<_> = sources.iter().map(|s| item(s, &cascade)).collect();
      let posts: Vec<_> = items
         .iter()
         .filter_map(|item| match &item.content {
            ItemContent::Post(post) => Some(post),
            ItemContent::Page(_) | ItemContent::Raw(_) => None,
         })
         .collect();

      let archive = Archive::new(posts).unwrap();
      let config = config();
      let info = render_info(&config, &archive);

      let mut env = Environment::new();
      env.add_template(
         "wrapper.jinja",
         "{{ content }}|{{ build_date }}|{{ archive | length }}",
      )
      .unwrap();

      assert_eq!(
         items[0].render(&env, &info).unwrap(),
         "<p>body</p>\n|2024-01-01T00:00:00+00:00|2"
      );
   }

   #[test]
   fn renders_a_standalone_body_template_without_a_layout() {
      let source = source(
         "content/standalone.lx",
         "---\ntitle: Standalone\n---\nHello, {{ data.title }} from {{ source.path }}!",
      );
      let item = item(&source, &cascade());
      let archive = Archive::new(Vec::new()).unwrap();
      let config = config();
      let info = render_info(&config, &archive);

      assert_eq!(
         item.render(&Environment::new(), &info).unwrap(),
         "Hello, Standalone from content/standalone.lx!"
      );
   }

   #[test]
   fn rejects_malformed_yaml() {
      let source = source("content/bad.lx", "---\nnot: valid: yaml\n---\nbody");

      assert!(matches!(
         prepare(&source, &cascade(), &config()),
         Err(Error::MetadataParsing { .. })
      ));
   }

   #[test]
   fn rejects_an_unclosed_fence() {
      let source = source("content/unclosed.lx", "---\nlayout: base.jinja\nbody");

      assert!(matches!(
         prepare(&source, &cascade(), &config()),
         Err(Error::UnclosedFrontMatter)
      ));
   }

   #[test]
   fn rejects_front_matter_that_is_not_a_mapping() {
      let source = source("content/sequence.lx", "---\n- one\n- two\n---\nbody");

      assert!(matches!(
         prepare(&source, &cascade(), &config()),
         Err(Error::MetadataParsing { .. })
      ));
   }

   #[test]
   fn resolves_a_local_date_in_the_configured_time_zone() {
      let source = source(
         "content/local.md",
         "---\ntitle: Local\ndate: 2025-06-23 18:55\n---\nbody",
      );
      let item = item(&source, &cascade());

      let ItemContent::Post(post) = &item.content else {
         panic!("expected a post, got a page or raw content");
      };

      assert_eq!(post.date.to_rfc3339(), "2025-06-23T18:55:00-06:00");
   }
}
