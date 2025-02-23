pub mod cascade;
pub mod serial;

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::path::StripPrefixError;

use chrono::DateTime;
use chrono::FixedOffset;
use lx_md::Markdown;
use serde::Deserialize;
use serde::Serialize;
use slug::slugify;
use thiserror::Error;

use super::image::Image;
use crate::page;

use self::cascade::Cascade;

/// Fully resolved metadata for an item, after merging the data from the item's
/// own header with all items in its data cascade.
///
/// **NOTE:** Although `title` and `date` are optional here, this is a function
/// of the fact that minijinja has no notion of pattern-matching, and therefore
/// no easy way to deal with a nested sum type. One or the other *is* required,
/// but this is handled by way of runtime validation. (Nothing makes me want so
/// badly to implement my own type-safe template language…)
#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
   /// The title of the item.
   pub title: String,

   /// The date the item was published.
   pub date: Option<DateTime<FixedOffset>>,

   /// The path to this piece of content.
   pub slug: Slug,

   /// Which layout should be used to render this?
   pub layout: String,

   pub book: Option<Book>,
   pub featured: bool,
   pub image: Option<Image>, // TODO: make it `Image`, not `Option`, and generate it .
   pub qualifiers: Qualifiers,
   pub series: Option<serial::Series>,
   pub subscribe: Option<serial::Subscribe>,
   pub subtitle: Option<Rendered>,
   pub summary: Option<Rendered>,
   pub tags: Vec<String>,
   pub thanks: Option<Rendered>,
   pub updated: Vec<Update>,
   pub work: Option<MusicalWork>,
}

impl Metadata {
   pub fn resolved(
      item: serial::Item,
      source: &page::Source,
      cascade: &Cascade,
      default_template_name: String,
      md: &Markdown,
   ) -> Result<Self, Error> {
      let permalink = item.permalink.map(|permalink| {
         permalink
            .trim_start_matches('/')
            .trim_end_matches('/')
            .to_string()
      });

      let dir = source.path.parent().ok_or_else(|| Error::BadPermalink {
         reason: format!("Missing parent for file at {}", source.path.display()),
         source: None,
      })?;

      let render = |s: String| Rendered::as_markdown(&s, md);

      let work = MusicalWork::resolved(item.work, cascade.work(dir))?;

      let title = work
         .as_ref()
         .map(|work| work.title.clone())
         .or(item.title)
         .ok_or_else(|| Error::MissingRequiredField {
            name: "title".to_string(),
         })?;

      let metadata = Metadata {
         title,
         date: item.date,
         slug: Slug::new(permalink.as_deref(), &source.path)?,
         subtitle: item.subtitle.map(render).transpose()?,
         layout: item
            .layout
            .or(cascade.layout(dir))
            .unwrap_or(default_template_name),
         summary: item.summary.map(render).transpose()?,
         qualifiers: {
            let from_item = item.qualifiers.unwrap_or_default();
            let from_cascade = cascade.qualifiers(dir).unwrap_or_default();

            Qualifiers {
               audience: from_item.audience.or(from_cascade.audience),
               epistemic: from_item.epistemic.or(from_cascade.epistemic),
               context: from_item.context.or(from_cascade.context),
               discusses: {
                  let discusses = from_item
                     .discusses
                     .iter()
                     .map(String::as_str)
                     .chain(from_cascade.discusses.iter().map(String::as_str))
                     .collect::<Vec<_>>();

                  nice_list(&discusses)
                     .map(|formatted| format!("{DISCUSSES} {formatted}"))
               },
               disclosure: from_item.disclosure.or(from_cascade.disclosure),
               retraction: from_item
                  .retraction
                  .or(from_cascade.retraction)
                  .map(Into::into),
            }
         },
         updated: item.updated.into_iter().try_fold(
            Vec::new(),
            |mut acc, serial::Update { at, changes }| match at {
               Some(at) => {
                  acc.push(Update { at, changes });
                  Ok(acc)
               }
               None => Err(FieldError::Update),
            },
         )?,
         thanks: item
            .thanks
            .or(cascade.thanks(dir))
            .map(render)
            .transpose()?,
         tags: {
            let mut tags = item.tags.unwrap_or_default();
            tags.extend(cascade.tags(dir));
            tags
         },
         featured: item.featured,
         image: item.image.or(cascade.image(dir)).map(Image::from),
         book: item.book.or(cascade.book(dir)).map(Book::from),
         series: item.series.or(cascade.series(dir)),
         subscribe: cascade.subscribe(dir),
         work,
      };

      Ok(metadata)
   }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Rendered {
   source: String,
   html: String,
}

impl Rendered {
   fn as_markdown(src: &str, md: &Markdown) -> Result<Rendered, Error> {
      md.render(src, |s| Ok(s.to_string()))
         .map(|(_, rendered)| Rendered {
            source: src.to_owned(),
            html: rendered.html().to_string(),
         })
         .map_err(Error::from)
   }

   pub fn plain(&self) -> String {
      // TODO: at construction above, create a plain text version as well as an HTML
      // version of the text.
      self.source.clone()
   }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Update {
   pub at: DateTime<FixedOffset>,
   pub changes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum Slug {
   Permalink(String),
   FromPath(PathBuf),
}

impl Slug {
   /// Attempt to build a slug given:
   ///
   /// - the item permalink, if any
   /// - the path to the item
   ///
   /// # Errors
   ///
   /// This function will return an error if .
   fn new(permalink: Option<&str>, source: &Path) -> Result<Slug, Error> {
      match permalink {
         Some(s) => Ok(Slug::Permalink(s.to_owned())),

         None => {
            let start = source.parent().ok_or_else(|| Error::BadPermalink {
               reason: format!("missing parent on '{}'?!?", source.display()),
               source: None,
            })?;

            let end = source
               .file_stem()
               .ok_or_else(|| Error::BadPermalink {
                  reason: format!("missing file stem on '{}'?!?", source.display()),
                  source: None,
               })?
               .to_str()
               .ok_or_else(|| Error::bad_permalink(source, None))
               .map(slugify)?;

            Ok(Slug::FromPath(start.join(end)))
         }
      }
   }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Qualifiers {
   pub audience: Option<String>,
   pub epistemic: Option<String>,
   pub context: Option<String>,
   pub discusses: Option<String>,
   pub disclosure: Option<String>,
   pub retraction: Option<Retraction>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Retraction(String);

impl From<serial::Retraction> for Retraction {
   fn from(serial::Retraction { url, title }: serial::Retraction) -> Self {
      let content = format!(
         r#"<p><strong>Caveat lector:</strong> I have since retracted this (see <a href="{url}">{title}</a>), but as a matter of policy I leave even work I have retracted publicly available as a matter of record.<p>"#
      );
      Retraction(content)
   }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Book {
   title: Option<String>,
   author: Option<String>,
   /// Year is a `String`, rather than something like a `u16`, because years
   /// are a lot more complicated than a number represents. If I write "400
   /// B.C.", for example, the system should still work.
   year: Option<String>,
   editors: Option<String>,
   translators: Option<String>,
   cover: Option<Image>,
   link: Option<String>,
   pub review: Option<serial::Review>,
}

impl From<serial::Book> for Book {
   fn from(
      serial::Book {
         title,
         author,
         year,
         editors,
         translators,
         cover,
         link,
         review,
      }: serial::Book,
   ) -> Self {
      Book {
         title,
         author: author.map(|a| a.to_string()),
         year,
         editors: editors.map(|e| e.to_string()),
         translators: translators.map(|t| t.to_string()),
         cover: cover.map(Image::from),
         link,
         review,
      }
   }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MusicalWork {
   /// The title of the work.
   pub title: String,

   /// An intentionally unformatted string describing the instrumentation.
   pub instrumentation: String,

   /// How long is the piece?
   pub duration: String,

   /// A subtitle for the work, if any.
   pub subtitle: Option<String>,

   // TODO: parse this, at minimum into a known-valid form (`\d{4}`).
   /// When the work was published.
   pub date: String,

   /// Where to listen to the work. Optional because it may not be included (at
   /// least: so I presently suppose!).
   pub listen: Option<Listen>,

   /// A video of the work to embed.
   pub video: Option<Video>,
}

impl MusicalWork {
   fn resolved(
      from_item: Option<serial::MusicalWork>,
      from_cascade: Option<serial::MusicalWork>,
   ) -> Result<Option<MusicalWork>, Error> {
      Ok(match (from_item, from_cascade) {
         (Some(from_item), Some(from_cascade)) => {
            let title = from_item
               .title
               .or(from_cascade.title)
               .ok_or(FieldError::Work(WorkError::Title, WorkMissingFrom::Both))?;

            let subtitle = from_item.subtitle.or(from_cascade.subtitle);

            let date = from_item
               .date
               .or(from_cascade.date)
               .ok_or(FieldError::Work(WorkError::Date, WorkMissingFrom::Both))?;

            let duration = from_item
               .duration
               .or(from_cascade.duration)
               .ok_or(FieldError::Work(WorkError::Duration, WorkMissingFrom::Both))?;

            let instrumentation = from_item
               .instrumentation
               .or(from_cascade.instrumentation)
               .ok_or(FieldError::Work(
                  WorkError::Instrumentation,
                  WorkMissingFrom::Both,
               ))?;

            let listen = from_item.listen.or(from_cascade.listen).map(Listen::from);
            let video = from_item.video.or(from_cascade.video).map(Video::from);

            Some(MusicalWork {
               title,
               date,
               duration,
               instrumentation,
               subtitle,
               listen,
               video,
            })
         }

         (Some(from_item), None) => {
            let title = from_item
               .title
               .ok_or(FieldError::Work(WorkError::Title, WorkMissingFrom::Item))?;

            let date = from_item
               .date
               .ok_or(FieldError::Work(WorkError::Date, WorkMissingFrom::Item))?;

            let duration = from_item
               .duration
               .ok_or(FieldError::Work(WorkError::Duration, WorkMissingFrom::Item))?;

            let instrumentation = from_item.instrumentation.ok_or(FieldError::Work(
               WorkError::Instrumentation,
               WorkMissingFrom::Item,
            ))?;

            Some(MusicalWork {
               title,
               subtitle: from_item.subtitle,
               date,
               duration,
               instrumentation,
               listen: from_item.listen.map(Listen::from),
               video: from_item.video.map(Video::from),
            })
         }

         (None, Some(from_cascade)) => {
            let title = from_cascade.title.ok_or(Error::bad_field(FieldError::Work(
               WorkError::Title,
               WorkMissingFrom::Cascade,
            )))?;

            let date = from_cascade.date.ok_or(Error::bad_field(FieldError::Work(
               WorkError::Date,
               WorkMissingFrom::Cascade,
            )))?;

            let duration =
               from_cascade
                  .duration
                  .ok_or(Error::bad_field(FieldError::Work(
                     WorkError::Duration,
                     WorkMissingFrom::Cascade,
                  )))?;

            let instrumentation =
               from_cascade
                  .instrumentation
                  .ok_or(Error::bad_field(FieldError::Work(
                     WorkError::Instrumentation,
                     WorkMissingFrom::Cascade,
                  )))?;

            Some(MusicalWork {
               title,
               subtitle: from_cascade.subtitle,
               date,
               duration,
               instrumentation,
               listen: from_cascade.listen.map(Listen::from),
               video: from_cascade.video.map(Video::from),
            })
         }
         (None, None) => None,
      })
   }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Listen {
   #[serde(default)]
   buy: HashMap<String, String>,

   #[serde(default)]
   stream: HashMap<String, String>,
}

impl std::convert::From<serial::Listen> for Listen {
   fn from(serial::Listen { buy, stream }: serial::Listen) -> Self {
      Listen { buy, stream }
   }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Video {
   YouTube { id: String },
   Url { url: String },
}

impl From<serial::Video> for Video {
   fn from(value: serial::Video) -> Self {
      match value {
         serial::Video::YouTube { yt } => Video::YouTube { id: yt },
      }
   }
}

#[derive(Error, Debug)]
pub enum Error {
   #[error("missing required field '{name}'")]
   MissingRequiredField { name: String },

   #[error("bad field data")]
   BadField {
      #[from]
      source: FieldError,
   },

   #[error("bad permalink: '{reason}'")]
   BadPermalink {
      reason: String,
      source: Option<StripPrefixError>,
   },

   #[error(transparent)]
   Markdown {
      #[from]
      source: lx_md::Error,
   },
}

impl Error {
   fn bad_permalink(p: &Path, source: Option<StripPrefixError>) -> Error {
      Error::BadPermalink {
         reason: format!("could not get `str` for '{}'", p.display()),
         source,
      }
   }

   fn bad_field(source: FieldError) -> Error {
      Error::BadField { source }
   }
}

#[derive(Error, Debug)]
pub enum FieldError {
   #[error("missing `updated.at` field")]
   Update,

   #[error("missing `{0}` in {1}")]
   Work(WorkError, WorkMissingFrom),
}

#[derive(Debug)]
pub enum WorkError {
   Title,
   Instrumentation,
   Date,
   Duration,
}

impl std::fmt::Display for WorkError {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      match self {
         WorkError::Title => write!(f, "title"),
         WorkError::Instrumentation => write!(f, "instrumentation"),
         WorkError::Date => write!(f, "date"),
         WorkError::Duration => write!(f, "duration"),
      }
   }
}

#[derive(Debug)]
pub enum WorkMissingFrom {
   Item,
   Cascade,
   Both,
}

impl std::fmt::Display for WorkMissingFrom {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      match self {
         WorkMissingFrom::Item => write!(f, "item (not present in cascade)"),
         WorkMissingFrom::Cascade => write!(f, "cascade (not present on item)"),
         WorkMissingFrom::Both => write!(f, "both item and cascade"),
      }
   }
}

fn nice_list<I, S>(strings: I) -> Option<String>
where
   I: IntoIterator<Item = S>,
   S: fmt::Display,
{
   // Might be a thing to think about cleaning up later to reduce allocations,
   // but honestly I don’t think it will matter very often!
   let strings = strings.into_iter().collect::<Vec<_>>();
   match strings.len() {
      0 => None,
      1 => Some(strings[0].to_string()),
      2 => Some(format!("{} and {}", strings[0], strings[1])),
      _ => {
         let (last, init) = strings.split_last().unwrap();
         Some(format!(
            "{}, and {last}",
            init.iter().fold(String::new(), |acc, s| {
               if acc.is_empty() {
                  s.to_string()
               } else {
                  acc + ", " + &s.to_string()
               }
            })
         ))
      }
   }
}

const DISCUSSES: &str = "<b>Heads up:</b> this post directly discusses";

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn slug_from_explicit_permalink() {
      let permalink = "Hello There";
      let source = PathBuf::default();

      assert_eq!(
         Slug::new(Some(permalink), &source).unwrap(),
         Slug::Permalink(String::from(permalink)),
         "The provided permalink is always respected"
      );
   }

   #[test]
   fn slug_from_simple_relative_path_with_simple_title() {
      let source = PathBuf::from("a/b/c/q.rs");
      let expected = PathBuf::from("a/b/c/q");

      assert_eq!(Slug::new(None, &source).unwrap(), Slug::FromPath(expected));
   }

   #[test]
   fn slug_from_simple_relative_path_with_complicated_title() {
      let source = PathBuf::from("a/b/c/Q R S.rs");
      let expected = PathBuf::from("a/b/c/q-r-s");

      assert_eq!(Slug::new(None, &source).unwrap(), Slug::FromPath(expected));
   }

   #[test]
   fn slug_from_complex_relative_path_with_simple_title() {
      let source = PathBuf::from("a/B C/d/q.rs");
      let expected = PathBuf::from("a/B C/d/q");

      assert_eq!(Slug::new(None, &source).unwrap(), Slug::FromPath(expected));
   }

   #[test]
   fn slug_from_complex_relative_path_with_complex_title() {
      let source = PathBuf::from("a/B C/d/Q R S.rs");
      let expected = PathBuf::from("a/B C/d/q-r-s");

      assert_eq!(Slug::new(None, &source).unwrap(), Slug::FromPath(expected));
   }

   #[test]
   fn nice_list_formatting() {
      assert_eq!(
         nice_list(&["a", "b", "c"]),
         Some(String::from("a, b, and c"))
      );
      assert_eq!(nice_list(&["a", "b"]), Some(String::from("a and b")));
      assert_eq!(nice_list(&["a"]), Some(String::from("a")));
      assert_eq!(nice_list(&Vec::<String>::new()), None);
   }
}
