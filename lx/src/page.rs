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
   match source.kind {
      Kind::Markdown => prepare_markdown(source, cascade, config).map(Prepared::page),
      Kind::Templated => prepare_template(source, cascade, config).map(Prepared::page),
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
      kind: Kind::Markdown,
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
      kind: Kind::Templated,
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

pub struct PreparedPage<'e> {
   /// The fully-parsed metadata associated with the item.
   data: Metadata,

   /// The date and time of the item, if any.
   date: Option<Date>,

   body: Body<'e>,
}

impl PreparedPage<'_> {
   pub fn render(
      self,
      highlighter: &mut Highlighter,
      postprocess: impl Fn(
         &str,
         &Metadata,
      ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
   ) -> Result<Rendered, Error> {
      let PreparedPage { data, date, body } = self;

      let content = match body {
         Body::Markdown(to_render) => lx_md::emit(to_render, Some(highlighter), |text| {
            postprocess(text, &data)
         })
         .map(Content::FromMd)?,

         Body::Template(template) => postprocess(template, &data)
            .map_err(|source| Error::Postprocess {
               source,
               original: template.to_string(),
            })
            .map(Content::FromTemplate)?,
      };

      Ok(Rendered {
         content,
         date,
         data,
      })
   }
}

pub struct Rendered {
   content: Content,
   date: Option<Date>,
   data: Metadata,
}

#[derive(Debug)]
pub enum Content {
   FromMd(lx_md::Rendered),
   FromTemplate(String),
}

impl Content {
   pub fn html(&self) -> &str {
      match self {
         Content::FromMd(rendered) => rendered.html(),
         Content::FromTemplate(body) => body,
      }
   }
}

impl serde::Serialize for Content {
   fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
   where
      S: serde::Serializer,
   {
      serializer.serialize_str(self.html())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
   Markdown,
   Templated,
   Raw,
}

impl<P> From<P> for Kind
where
   P: AsRef<Utf8Path>,
{
   fn from(value: P) -> Self {
      let extensions: Vec<&str> = value
         .as_ref()
         .file_name()
         .map(|file_name| file_name.split('.').skip(1).collect())
         .unwrap_or_default();

      if extensions.last() == Some(&"md") {
         Kind::Markdown
      } else if extensions.contains(&"lx") {
         Kind::Templated
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

/// A fully resolved representation of a page.
///
/// In this struct, the metadata has been parsed and resolved, and the content has been
/// converted from Markdown to HTML and preprocessed with both the templating engine and
/// my typography tooling. It is ready to render into the target layout template specified
/// by its [`Metadata`] and then to print to the file system.
#[derive(Debug, Serialize)]
pub struct Page<'s> {
   pub id: Id,

   /// The fully parsed metadata associated with the page.
   pub data: Metadata,

   /// The fully rendered contents of the page.
   pub content: Content,

   pub source: &'s Source,

   pub path: RootedPath,
}

pub enum Item<'s> {
   Page(Page<'s>),
   Post(Post<'s>),
}

impl<'s> Item<'s> {
   pub fn from_rendered(
      rendered: Rendered,
      source: &'s Source,
      in_dir: &Utf8Path,
   ) -> Result<Self, Error> {
      let id = Id(Uuid::new_v5(
         &Uuid::NAMESPACE_OID,
         source.path.as_os_str().as_bytes(),
      ));

      let path = RootedPath::new(&rendered.data.slug, in_dir)?;
      let page = Page {
         id,
         content: rendered.content,
         data: rendered.data,
         source,
         path,
      };

      let item = match rendered.date {
         Some(date) => Item::Post(Post { date, page }),
         None => Item::Page(page),
      };

      Ok(item)
   }

   pub fn render(&self, env: &Environment, info: &RenderInfo) -> Result<String, Error> {
      match self.layout() {
         Some(layout) => templates::render(env, layout, Context::new(info, self))
            .map_err(|source| Error::RenderTemplate {
               path: self.path().as_ref().to_owned(),
               source,
            }),
         None => Ok(self.content().html().into()),
      }
   }

   pub fn content(&self) -> &Content {
      match self {
         Item::Page(page) => &page.content,
         Item::Post(post) => &post.page.content,
      }
   }

   pub fn layout(&self) -> Option<&str> {
      self.data().layout.as_deref()
   }

   pub fn path(&self) -> &RootedPath {
      match self {
         Item::Page(page) => &page.path,
         Item::Post(post) => &post.page.path,
      }
   }

   pub fn source(&self) -> &Source {
      match self {
         Item::Page(page) => page.source,
         Item::Post(post) => post.page.source,
      }
   }

   pub fn title(&self) -> Option<&str> {
      self.data().title.as_deref()
   }

   pub fn data(&self) -> &Metadata {
      match self {
         Item::Page(page) => &page.data,
         Item::Post(post) => &post.page.data,
      }
   }
}

pub struct RenderInfo<'a> {
   pub config: &'a Config,
   pub build_date: String,
   pub archive: &'a Archive<'a>,
}

#[derive(Serialize)]
struct Context<'i, 'a> {
   content: &'i str,
   data: &'i Metadata,
   path: &'i RootedPath,
   source: &'i Source,
   config: &'i Config,
   build_date: &'i str,
   archive: &'i Archive<'a>,
}

impl<'i, 'a> Context<'i, 'a> {
   fn new(render_info: &'i RenderInfo<'a>, item: &'i Item) -> Context<'i, 'a> {
      Context {
         content: item.content().html(),
         data: item.data(),
         path: item.path(),
         source: item.source(),
         config: render_info.config,
         build_date: &render_info.build_date,
         archive: render_info.archive,
      }
   }
}

// NOTE: the following all assume stable, unique identifiers for pages. The existing
//   implementation *does* handle this because it creates a v5 UUID from the path to the
//   file, which by definition must be unique (you cannot have two files with the same
//   path in any file system I know of!), but this only holds as long as that does.
impl PartialEq for Page<'_> {
   fn eq(&self, other: &Self) -> bool {
      self.id == other.id
   }
}

impl Eq for Page<'_> {}

impl Hash for Page<'_> {
   fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
      self.id.hash(state);
   }
}

/// A `Post` is simply a [`Page`] that has a date.
#[derive(Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Post<'e> {
   pub page: Page<'e>,
   pub date: Date,
}

impl<'e> PartialOrd for Post<'e> {
   fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
      Some(self.cmp(other))
   }
}

impl Ord for Post<'_> {
   fn cmp(&self, other: &Self) -> Ordering {
      self.date.cmp(&other.date)
   }
}

#[derive(Debug, Serialize, Hash, PartialEq, Eq)]
pub struct PostLink<'e> {
   anchor_title: String,
   slug: &'e Slug,
}

impl<'e> From<&'e Post<'e>> for PostLink<'e> {
   fn from(value: &'e Post<'e>) -> Self {
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
pub struct PostAndConfig<'p, 'c, 'e>(pub &'p Post<'e>, pub &'c Config);

// TODO: This will need to take `From` a different type, one that wraps `Post`
// and probably also `Config` (e.g. to build the full URL).
impl From<PostAndConfig<'_, '_, '_>> for json_feed::FeedItem {
   fn from(PostAndConfig(post, config): PostAndConfig) -> Self {
      json_feed::FeedItem {
         id: post.page.id.to_string(),
         url: Some(post.page.path.url(config)),
         external_url: None, // TODO: support for page.link etc.
         title: post.page.data.title.clone(),
         content_text: None, // TODO: use this for microblogging?
         content_html: Some(post.page.content.html().to_string()),
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Collections(HashMap<Id, crate::collection::Id>);

#[cfg(test)]
mod tests {
   use minijinja::{Environment, context};

   use super::*;

   fn source(path: &str, contents: &str) -> Source {
      Source {
         path: Utf8PathBuf::from(path),
         contents: String::from(contents),
         kind: Kind::from(path),
      }
   }

   fn cascade() -> Cascade {
      Cascade::new(&[]).unwrap()
   }

   fn prepared_page<'e>(source: &'e Source, cascade: &Cascade) -> PreparedPage<'e> {
      match prepare(source, cascade, &config()).unwrap() {
         Prepared::Page(page) => *page,
         Prepared::Raw(_) => panic!("expected a page"),
      }
   }

   fn item<'e>(source: &'e Source, cascade: &Cascade) -> Item<'e> {
      let prepared = prepared_page(source, cascade);
      let rendered = prepared
         .render(&mut Highlighter::new(), |text, _| Ok(text.to_owned()))
         .unwrap();
      Item::from_rendered(rendered, source, Utf8Path::new("content")).unwrap()
   }

   fn post(name: &str, date: &str) -> Source {
      source(
         &format!("content/{name}.md"),
         &format!("---\ntitle: {name}\ndate: {date}\n---\nbody"),
      )
   }

   fn config() -> Config {
      serde_yaml::from_str(
         "url: https://example.com\nrepo: https://example.com/repo\ntitle: Example\ndescription: Example\nauthor:\n  name: Example\n  email: hello@example.com\n  links: {}\noutput: public\ntime_zone: America/Denver\nimage:\n  url: https://example.com/image.png\n",
      )
      .unwrap()
   }

   #[test]
   fn classifies_markdown_as_primary_content() {
      assert_eq!(Kind::from("content/index.md"), Kind::Markdown);
      assert_eq!(Kind::from("content/essays/jj init.md"), Kind::Markdown);
   }

   #[test]
   fn markdown_wins_over_explicit_templating() {
      assert_eq!(Kind::from("content/foo.lx.md"), Kind::Markdown);
   }

   #[test]
   fn classifies_terminal_lx_as_templated() {
      assert_eq!(Kind::from("content/foo.lx"), Kind::Templated);
   }

   #[test]
   fn classifies_lx_in_extension_chain_as_templated() {
      assert_eq!(Kind::from("content/foo.lx.xml"), Kind::Templated);
      assert_eq!(
         Kind::from("content/foo.lx.fancy.extension.chain"),
         Kind::Templated
      );
      assert_eq!(Kind::from("content/foo.bar.lx.xml"), Kind::Templated);
   }

   #[test]
   fn does_not_mistake_a_named_stem_for_an_lx_extension() {
      assert_eq!(Kind::from("content/lx.xml"), Kind::Raw);
      assert_eq!(Kind::from("content/md"), Kind::Raw);
   }

   #[test]
   fn classifies_everything_else_as_raw() {
      assert_eq!(Kind::from("content/foo.xml"), Kind::Raw);
      assert_eq!(Kind::from("content/foo.html"), Kind::Raw);
      assert_eq!(Kind::from("content/data.json"), Kind::Raw);
      assert_eq!(Kind::from("content/extensionless"), Kind::Raw);
   }

   #[test]
   fn splits_front_matter_from_body() {
      let parsed = FrontMatter::parse("---\nlayout: base.jinja\n---\nhello\n").unwrap();

      assert_eq!(parsed.metadata, Some("layout: base.jinja\n"));
      assert_eq!(parsed.body, "hello\n");
   }

   #[test]
   fn front_matter_is_none_when_the_fence_is_absent() {
      let parsed = FrontMatter::parse("hello").unwrap();

      assert_eq!(parsed.metadata, None);
      assert_eq!(parsed.body, "hello");
   }

   #[test]
   fn prepares_raw_sources_without_rendering() {
      let source = source("content/data.json", "{}");

      assert!(matches!(
         prepare(&source, &cascade(), &config()),
         Ok(Prepared::Raw(_))
      ));
   }

   #[test]
   fn parses_fenced_front_matter() {
      let source = source(
         "content/feed.lx.xml",
         "---\nlayout: feed.jinja\ntitle: Feed\n---\nbody",
      );
      let prepared = prepared_page(&source, &cascade());

      assert_eq!(prepared.data.title.as_deref(), Some("Feed"));
      assert_eq!(prepared.data.layout.as_deref(), Some("feed.jinja"));
      assert!(matches!(&prepared.body, Body::Template(body) if *body == "body"));
   }

   #[test]
   fn absent_fence_treats_the_whole_source_as_body() {
      let source = source("content/body.lx", "just a body");
      let prepared = prepared_page(&source, &cascade());

      assert_eq!(prepared.data.title, None);
      assert_eq!(prepared.data.layout, None);
      assert!(matches!(&prepared.body, Body::Template(body) if *body == "just a body"));
   }

   #[test]
   fn empty_front_matter_is_an_empty_mapping() {
      let source = source("content/empty.lx", "---\n---\nbody");
      let prepared = prepared_page(&source, &cascade());

      assert_eq!(prepared.data.title, None);
      assert!(matches!(&prepared.body, Body::Template(body) if *body == "body"));
   }

   #[test]
   fn preserves_multiline_bodies() {
      let source = source(
         "content/multiline.lx",
         "---\nlayout: base.jinja\n---\nhello\nworld\n",
      );
      let prepared = prepared_page(&source, &cascade());

      assert!(
         matches!(&prepared.body, Body::Template(body) if *body == "hello\nworld\n")
      );
   }

   #[test]
   fn renders_layouts_with_build_context() {
      let mut cascade = cascade();
      cascade.add_at(
         "content",
         serial::Ambient {
            layout: Some("wrapper.jinja".into()),
            ..Default::default()
         },
      );
      let sources = vec![
         post("a", "2022-01-01T00:00:00-05:00"),
         post("b", "2023-01-01T00:00:00-05:00"),
      ];
      let items: Vec<_> = sources.iter().map(|s| item(s, &cascade)).collect();
      let archive = crate::archive::Archive::new(&items).unwrap();
      let config = config();
      let info = RenderInfo {
         config: &config,
         build_date: "2024-01-01T00:00:00+00:00".into(),
         archive: &archive,
      };

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
      let prepared = prepared_page(&source, &cascade());

      let rendered = prepared
         .render(&mut Highlighter::new(), |text, _| {
            let env = Environment::new();
            env.render_str(
               text,
               context! {
                  data => context! {
                     title => "Standalone",
                  },
                  source => source,
               },
            )
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
         })
         .unwrap();

      assert_eq!(
         rendered.content.html(),
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
      let prepared = prepared_page(&source, &cascade());

      assert_eq!(
         prepared.date.unwrap().to_rfc3339(),
         "2025-06-23T18:55:00-06:00"
      );
   }
}
