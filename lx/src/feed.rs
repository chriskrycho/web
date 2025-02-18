mod json;

use std::convert::TryFrom;

use json_feed::{AuthorOptions, JSONFeed};
use thiserror::Error;

use crate::{
   data::config::Config,
   page::{Page, PageAndConfig, Updated},
};

/// Required resources for a `Feed`.
pub struct Feed<'a> {
   /// Every feed has its own title.
   title: String,

   /// Feeds also need read access to the site config to be able to render the
   /// full set of data specified for Atom, JSON, or RSS.
   site_config: &'a Config,

   /// The set of items to render in the feed. A read-only slice because I will
   /// never actually need to *write* to these. I just need the parsed metadata
   /// and rendered HTML contents of the page, to render into the template.
   items: &'a [Page<'a>],
}

impl<'a> Feed<'a> {
   pub fn _new(title: String, site_config: &'a Config, items: &'a [Page]) -> Feed<'a> {
      Feed {
         title,
         site_config,
         items,
      }
   }
}

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not convert to JSON feed")]
   Json(String),
   #[error("could not convert to Atom feed")]
   Atom,
}

impl<'a> TryFrom<Feed<'a>> for JSONFeed {
   type Error = Error;

   fn try_from(feed: Feed<'a>) -> Result<Self, Self::Error> {
      let items = feed
         .items
         .iter()
         .map(|page| json_feed::FeedItem::from(PageAndConfig(page, feed.site_config)))
         .collect();

      // TODO: needs the info for the *feed* URL.
      let feed = JSONFeed::builder(&feed.title, items)
         .with_author(&AuthorOptions {
            name: Some(&feed.site_config.author.name),
            url: None,
            avatar: None,
         })
         .map_err(Error::Json)?
         .with_description(&feed.site_config.description)
         .build();

      Ok(feed)
   }
}
