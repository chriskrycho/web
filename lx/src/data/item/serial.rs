//! The serialization inputs for metadata. Covers both YAML metadata in headers
//! and associated data from JSON/TOML/YAML/JSON5/whatever else I decide to
//! support in data files.

use std::{
   collections::HashMap,
   fmt::{self},
};

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::data::{image::serial::Image, item::nice_list};

#[derive(Deserialize, Debug, Default)]
pub struct Item {
   pub title: Option<String>,
   pub subtitle: Option<String>,
   pub summary: Option<String>,
   pub date: Option<DateTime<FixedOffset>>,
   pub permalink: Option<String>,
   pub qualifiers: Option<Qualifiers>,
   /// When was the item first created? Useful for distinguishing between item creation
   /// and item publication, when letting something bake in public for a while.
   pub started: Option<DateTime<FixedOffset>>,
   #[serde(default)]
   pub updated: Vec<Update>,
   // --- Begin section of fields also available in AmbientMetadata --- //
   pub book: Option<Book>,
   #[serde(default)]
   pub featured: bool,
   pub image: Option<Image>,
   pub layout: Option<String>,
   pub series: Option<Series>,
   pub thanks: Option<String>,
   pub tags: Option<Vec<String>>,
   pub work: Option<MusicalWork>,
}

#[derive(Error, Debug)]
#[error("could not parse YAML metadata")]
pub struct ItemParseError {
   unparseable: String,
   source: serde_yaml::Error,
}

impl Item {
   pub fn try_parse(src: &str) -> Result<Item, ItemParseError> {
      serde_yaml::from_str(src).map_err(|e| ItemParseError {
         unparseable: src.to_string(),
         source: e,
      })
   }
}

#[derive(Debug, Deserialize)]
pub struct Update {
   pub(super) at: Option<DateTime<FixedOffset>>,
   pub(super) changes: Option<String>,
}

/// Fields which are allowed to be present "ambiently" for a given item, i.e.
/// from a `my-dir.lx.yaml` or some such colocated next to a file.
#[derive(Deserialize, Debug, Default)]
pub struct Ambient {
   pub book: Option<Book>,
   #[serde(default)]
   pub featured: bool,
   pub image: Option<Image>,
   pub layout: Option<String>,
   pub qualifiers: Option<Qualifiers>,
   pub series: Option<Series>,
   pub subscribe: Option<Subscribe>,
   pub tags: Option<Vec<String>>,
   pub thanks: Option<String>,
   pub work: Option<MusicalWork>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Qualifiers {
   pub audience: Option<String>,
   pub epistemic: Option<String>,
   pub context: Option<String>,
   #[serde(default)]
   pub discusses: Vec<String>,
   pub disclosure: Option<String>,
   pub retraction: Option<Retraction>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Video {
   YouTube { yt: String },
   YouTubePlaylist { pl: String },
   Url { url: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Retraction {
   pub url: String,
   pub title: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Subscribe {
   atom: Option<String>,
   json: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Book {
   pub title: Option<String>,
   pub author: Option<Authorship>,
   /// Year is a `String`, rather than something like a `u16`, because years
   /// are a lot more complicated than a number represents. If I write "400
   /// B.C.", for example, the system should still work.
   pub year: Option<String>,
   pub editors: Option<Authorship>,
   pub translators: Option<Authorship>,
   pub cover: Option<Image>,
   pub link: Option<String>, // URL or Bookshop - much like `Image`
   pub review: Option<Review>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Authorship {
   Single(String),
   Multi(Vec<String>),
}

impl fmt::Display for Authorship {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let s = match self {
         Authorship::Single(name) => name.to_owned(),
         Authorship::Multi(names) => nice_list(names).unwrap_or_default(),
      };
      write!(f, "{s}")
   }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BibleRef {
   translation: String,
   book: String,
   /// This needs to handle the following:
   ///
   /// - Individual verse references: "John 1:1"
   /// - Books without chapters "Jude 1"
   /// - Within-chapter ranges: "John 1:1–3"
   /// - Multi-chapter ranges: "John 4–5"
   /// - Verse ranges crossing chapter breaks: "John 1:35–2:12"
   ///
   /// Given all of these, "just use a string" makes far more sense than trying
   /// to build a complex set of alternative types for it. (This is, after all,
   /// not a Bible application, where that would be a thing to parse!)
   passage: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Review {
   rating: Rating,
   summary: String,
}

impl fmt::Display for Review {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "{}: {}", self.rating, self.summary)
   }
}

// TODO: right now this assumes it can be deserialized from the associated text,
// but in fact it should be derived from the same text as its `Display`
// implementation below. (A later enhancement: converting "****" etc. to it or
// something cool like that.)
#[derive(Serialize, Deserialize, Clone, Debug)]
enum Rating {
   #[serde(rename = "Not recommended")]
   NotRecommended,
   #[serde(rename = "Recommended with qualifications")]
   WithQualifications,
   #[serde(rename = "Recommended")]
   Recommended,
   #[serde(rename = "Required")]
   Required,
}

impl fmt::Display for Rating {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(
         f,
         "{}",
         match self {
            Rating::NotRecommended => "Not recommended",
            Rating::WithQualifications => "Recommended with qualifications",
            Rating::Recommended => "Recommended",
            Rating::Required => "Required",
         }
      )
   }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Series {
   // The name is optional: it could be supplied via the data file somewhere up
   // the tree.
   pub name: Option<String>,
   // The *part* has to be supplied, though.
   pub part: u8,
}

// All fields here are optional because they may be supplied either ambiently or via the
// item itself.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MusicalWork {
   /// The title of the work.
   pub title: Option<String>,
   /// An intentionally unformatted string describing the instrumentation.
   pub instrumentation: Option<String>,
   /// A subtitle for the work.
   pub subtitle: Option<String>,
   /// When the work was published.
   pub date: Option<String>,
   /// Where to listen to the work.
   pub listen: Option<Listen>,
   /// A video of the work to embed.
   pub video: Option<Video>,
   // TODO: require a format: `(\d{1,2}:)?\d{2}:\d{2}
   /// How long is the piece?
   pub duration: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Listen {
   /// Where to buy the cue or album.
   #[serde(default)]
   pub buy: HashMap<String, String>,

   /// Where to stream the cue or album.
   #[serde(default)]
   pub stream: HashMap<String, String>,
}
