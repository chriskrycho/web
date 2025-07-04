use std::{collections::HashMap, fmt, hash::Hash, os::unix::prelude::OsStrExt};

use camino::{Utf8Path, Utf8PathBuf};
use lx_md::{self, Markdown, RenderError, ToRender};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::data::{
   config::Config,
   item::{self, Metadata, Slug, cascade::Cascade, serial},
};

pub fn prepare<'e>(
   md: &Markdown,
   source: &'e Source,
   cascade: &Cascade,
) -> Result<Prepared<'e>, Error> {
   let lx_md::Prepared {
      metadata_src,
      to_render,
   } = lx_md::prepare(&source.contents)?;

   let data = metadata_src
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

   Ok(Prepared { data, to_render })
}

pub struct Prepared<'e> {
   /// The fully-parsed metadata associated with the page.
   data: Metadata,

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
         data: self.data,
      })
   }
}

pub struct Rendered {
   content: lx_md::Rendered,
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

/// A fully-resolved representation of a page.
///
/// In this struct, the metadata has been parsed and resolved, and the content has been
/// converted from Markdown to HTML and preprocessed with both the templating engine and
/// my typography tooling. It is ready to render into the target layout template specified
/// by its `data: Metadata` and then to print to the file system.
#[derive(Debug)]
pub struct Page<'s> {
   pub id: Id,

   /// The fully-parsed metadata associated with the page.
   pub data: Metadata,

   /// The fully-rendered contents of the page.
   pub content: lx_md::Rendered,

   pub source: &'s Source,

   pub path: RootedPath,
}

impl<'s> Page<'s> {
   pub fn from_rendered(
      rendered: Rendered,
      source: &'s Source,
      in_dir: &Utf8Path,
   ) -> Result<Page<'s>, Error> {
      // TODO: This is the right idea for where I want to take this, but ultimately I
      // don't want to do it based on the source path (or if I do, *only* initially as
      // a way of generating it to start).
      let id = Id(Uuid::new_v5(
         &Uuid::NAMESPACE_OID,
         source.path.as_os_str().as_bytes(),
      ));

      let path = RootedPath::new(&rendered.data.slug, in_dir)?;

      Ok(Page {
         id,
         content: rendered.content,
         data: rendered.data,
         source,
         path,
      })
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
pub struct PageAndConfig<'p, 'c, 'e>(pub &'p Page<'e>, pub &'c Config);

// TODO: This will need to take `From` a different type, one that wraps `Page`
// and probably also `Config` (e.g. to build the full URL).
impl From<PageAndConfig<'_, '_, '_>> for json_feed::FeedItem {
   fn from(PageAndConfig(page, config): PageAndConfig) -> Self {
      json_feed::FeedItem {
         id: page.id.to_string(),
         url: Some(page.path.url(config)),
         external_url: None, // TODO: support for page.link etc.
         title: Some(page.data.title.clone()),
         content_text: None, // TODO: use this for microblogging?
         content_html: Some(page.content.html().to_string()),
         summary: page.data.summary.as_ref().map(|summary| summary.plain()),
         image: None,        // TODO: add support for images to metadata
         banner_image: None, // TODO: add support for these if I care?
         date_published: page.data.date.map(|date| date.to_rfc3339()),
         date_modified: None, // TODO: from `page.metadata.updated` in some way
         author: None,        // TODO: it me!
         tags: Some(page.data.tags.clone()),
         attachments: None,
      }
   }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Collections(HashMap<Id, crate::collection::Id>);
