use indexmap::{IndexMap, IndexSet};
use serde::Serialize;
use thiserror::Error;

use crate::page::{Item, PostLink};

/// A data structure that maps each post to Y -> M -> D -> posts, preserving the order of
/// the posts.
#[derive(Debug, Serialize)]
pub struct Archive<'p>(IndexMap<Year, MonthMap<'p>>);

impl<'e> Archive<'e> {
   /// Reference all pages in an unordered fashion.
   pub fn new(
      items: impl IntoIterator<Item = &'e Item<'e>>,
   ) -> Result<Archive<'e>, Error> {
      let mut year_map = IndexMap::<Year, MonthMap<'e>>::new();

      let posts = items.into_iter().filter_map(|item| match item {
         Item::Page(_) => None,
         Item::Post(post) => Some(post),
      });

      for post in posts {
         // Archive years are positive CE year numbers: ISO year 0 (1 BCE) is archived
         // as 1, and BCE year `y` is archived as `1 - y`.
         let year = match post.date.year() {
            0 => 1,
            y if y > 0 => u32::try_from(y).expect("positive jiff year fits in u32"),
            y => u32::try_from(1 - i32::from(y)).expect("BCE year fits in u32"),
         };
         let year = Year::from(year);

         let month =
            u8::try_from(post.date.month()).expect("jiff months are in the range 1..=12");
         let month = Month::try_from(month).map_err(|source| Error::BadMonth {
            raw: u32::from(month),
            source,
         })?;

         let day =
            u32::try_from(post.date.day()).expect("jiff days are in the range 1..=31");
         let day = Day::try_from(day).map_err(Error::from)?;

         let month_map = year_map.entry(year).or_default();
         let day_map = month_map.entry(month).or_default();
         day_map.entry(day).or_default().insert(PostLink::from(post));
      }

      Ok(Archive(year_map))
   }
}

#[allow(dead_code)]
pub enum Order {
   OldFirst,
   NewFirst,
}

#[derive(Debug, Error)]
pub enum Error {
   #[error("nonsense month value: '{raw}")]
   BadMonth { raw: u32, source: MonthOutOfRange },

   #[error(transparent)]
   BadDay {
      #[from]
      source: BadDay,
   },
}

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone, Serialize)]
pub struct Year {
   raw: u32,
}

impl From<u32> for Year {
   fn from(value: u32) -> Self {
      Self { raw: value }
   }
}

/// The month of a year.
///
/// Jiff has no month type, so this preserves the ordering (`January` through
/// `December`) and the serialization (the English name of the month) used by the
/// archive’s map keys.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone)]
pub struct Month {
   raw: u8,
}

impl Month {
   /// The English name of the month, e.g. `January`.
   pub fn name(&self) -> &'static str {
      const NAMES: [&str; 12] = [
         "January",
         "February",
         "March",
         "April",
         "May",
         "June",
         "July",
         "August",
         "September",
         "October",
         "November",
         "December",
      ];

      // SAFETY: `Month` is only constructed from `1..=12`, so this cannot be out of
      // bounds.
      NAMES[usize::from(self.raw - 1)]
   }
}

impl TryFrom<u8> for Month {
   type Error = MonthOutOfRange;

   fn try_from(value: u8) -> Result<Self, Self::Error> {
      match value {
         1..=12 => Ok(Month { raw: value }),
         wat => Err(MonthOutOfRange { raw: wat }),
      }
   }
}

impl Serialize for Month {
   fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
      serializer.collect_str(self.name())
   }
}

#[derive(Debug, Error)]
#[error("nonsense month value: '{raw}'")]
pub struct MonthOutOfRange {
   raw: u8,
}

type MonthMap<'p> = IndexMap<Month, DayMap<'p>>;

type DayMap<'p> = IndexMap<Day, IndexSet<PostLink<'p>>>;

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone, Serialize)]
pub struct Day {
   raw: u8,
}

impl TryFrom<u32> for Day {
   type Error = BadDay;

   fn try_from(value: u32) -> Result<Self, Self::Error> {
      match value {
         // SAFETY: this cast will never truncate because 1..=31 < 256.
         legit @ 1..=31 => Ok(Day { raw: legit as u8 }),
         wat => Err(BadDay { raw: wat }),
      }
   }
}

#[derive(Debug, Error)]
#[error("nonsense day value: '{raw}'")]
pub struct BadDay {
   raw: u32,
}
