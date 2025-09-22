use chrono::{Datelike, Month};
use indexmap::{IndexMap, IndexSet};
use minijinja::Environment;
use serde::Serialize;
use thiserror::Error;

use crate::{
   page::{Item, Post, PostLink},
   templates::view::{self, View},
};

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
         let year = Year::from(post.date.year_ce().1);

         let month = post.date.month();
         let month = Month::try_from(u8::try_from(month).unwrap())
            .map_err(|source| Error::BadMonth { raw: month, source })?;

         let day = Day::try_from(post.date.day()).map_err(Error::from)?;

         let month_map = year_map.entry(year).or_default();
         let day_map = month_map.entry(month).or_default();
         day_map.entry(day).or_default().insert(PostLink::from(post));
      }

      Ok(Archive(year_map))
   }
}

impl<'e> View for Archive<'e> {
   const VIEW_NAME: &'static str = "archive";

   fn view(&self, env: &Environment) -> Result<String, minijinja::Error> {
      if self.0.is_empty() {
         Ok("".into())
      } else {
         env.get_template(&view::template_for(self))?.render(self)
      }
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
   BadMonth {
      raw: u32,
      source: chrono::OutOfRange,
   },

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
