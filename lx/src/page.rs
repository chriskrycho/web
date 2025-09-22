use crate::{
   data::{
      config::Config,
      item::{self, Metadata, Slug, cascade::Cascade, serial},
   },
   templates::view::View,
};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, FixedOffset};
use json_feed::Author;
use lx_md::{self, Markdown, RenderError, ToRender};
use minijinja::{Environment, State, Value, context, value::Object};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, sync::Arc};
use std::{collections::HashMap, fmt, hash::Hash, os::unix::prelude::OsStrExt};
use thiserror::Error;
use uuid::Uuid;

pub fn prepare<'e>(
   md: &Markdown,
   source: &'e Source,
   cascade: &Cascade,
) -> Result<Prepared<'e>, Error> {
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
            String::from("base.jinja"), // TODO: not this
            md,
         )
         .map_err(Error::from)
      })?;

   Ok(Prepared {
      data,
      date,
      to_render,
   })
}

pub struct Prepared<'e> {
   /// The fully-parsed metadata associated with the item.
   data: Metadata,

   /// The date and time of the item, if any.
   date: Option<DateTime<FixedOffset>>,

   to_render: ToRender<'e>,
}

impl Prepared<'_> {
   pub fn render(
      self,
      md: &Markdown,
      rewrite: impl Fn(
         &str,
         &Metadata,
      ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
   ) -> Result<Rendered, Error> {
      Ok(Rendered {
         content: md.emit(self.to_render, |text| rewrite(text, &self.data))?,
         date: self.date,
         data: self.data,
      })
   }
}

pub struct Rendered {
   content: lx_md::Rendered,
   date: Option<DateTime<FixedOffset>>,
   data: Metadata,
}

/// Source data for a file: where it came from, and its original contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
   /// Original source location for the file.
   pub path: Utf8PathBuf,
   /// Original contents of the file.
   pub contents: String,
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
   pub content: lx_md::Rendered,

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

   pub fn content(&self) -> &lx_md::Rendered {
      match self {
         Item::Page(page) => &page.content,
         Item::Post(post) => &post.page.content,
      }
   }

   pub fn layout(&self) -> &str {
      self.data().layout.as_str()
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

   pub fn title(&self) -> &str {
      self.data().title.as_ref()
   }

   pub fn data(&self) -> &Metadata {
      match self {
         Item::Page(page) => &page.data,
         Item::Post(post) => &post.page.data,
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

#[derive(Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Post<'e> {
   pub page: Page<'e>,
   pub date: DateTime<FixedOffset>,
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

impl Object for PostLink<'_> {
   fn call(
      self: &Arc<Self>,
      state: &State<'_, '_>,
      _args: &[Value],
   ) -> Result<Value, minijinja::Error> {
      self.view(state.env()).map(Value::from)
   }
}

impl<'e> View for PostLink<'e> {
   const VIEW_NAME: &'static str = "post-link";
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
   Render {
      #[from]
      source: RenderError,
   },

   #[error("Invalid combination of root '{root}' and slug '{slug}'")]
   BadSlugRoot {
      source: std::path::StripPrefixError,
      root: Utf8PathBuf,
      slug: Utf8PathBuf,
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
         title: Some(post.page.data.title.clone()),
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
