use std::{
   collections::{BTreeMap, BTreeSet},
   sync::Arc,
};

use chrono::{Datelike, Month};
use minijinja::{Environment, value::Object};
use thiserror::Error;

use crate::{
   page::{Item, Post},
   templates::view::{self, View},
};

/// A data structure that maps each post to Y -> M -> D -> posts, preserving the order of
/// the posts.
#[derive(Debug)]
pub struct Archive<'p>(BTreeMap<Year, MonthMap<'p>>);

impl<'e> Archive<'e> {
   /// Reference all pages in an unordered fashion.
   pub fn new(
      items: impl IntoIterator<Item = &'e Item<'e>>,
   ) -> Result<Archive<'e>, Error> {
      let mut year_map = BTreeMap::<Year, MonthMap<'e>>::new();

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
         day_map.entry(day).or_default().insert(post);
      }

      Ok(Archive(year_map))
   }

   /// Iterate over all pages in the archive, returning a tuple of (Y, M, D, Page) so that
   /// I can then filter on that by topic, iterate
   pub fn iter(&self) -> impl IntoIterator<Item = (Year, Month, Day, &'e Post<'e>)> {
      self
         .0
         .iter()
         .flat_map(|(&year, month_map)| {
            month_map
               .iter()
               .map(move |(&month, day_map)| (year, month, day_map))
         })
         .flat_map(|(year, month, day_map)| {
            day_map
               .iter()
               .map(move |(&day, pages)| (year, month, day, pages))
         })
         .flat_map(|(year, month, day, pages)| {
            pages.iter().map(move |&page| (year, month, day, page))
         })
   }

   pub fn has_entries(&self) -> bool {
      !self.0.is_empty()
   }
}

impl<'e> View for Archive<'e> {
   const VIEW_NAME: &'static str = "archive";

   fn view<'a, I: IntoIterator<Item = &'a Item<'a>>>(
      &self,
      items: I,
      env: &Environment,
   ) -> String {
      let template = env
         .get_template(&view::template_for(self))
         .expect("Add error handling!");

      todo!()
   }
}

impl<'e> Object for Archive<'e> {
   fn call_method(
      self: &Arc<Self>,
      state: &minijinja::State<'_, '_>,
      method: &str,
      args: &[minijinja::Value],
   ) -> Result<minijinja::Value, minijinja::Error> {
      match method {
         "has_entries" => Ok(self.has_entries().into()),

         _ => Err(minijinja::Error::new(
            minijinja::ErrorKind::UnknownMethod,
            method.to_owned(),
         )),
      }
   }

   fn render(self: &Arc<Self>, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
   where
      Self: Sized + 'static,
   {
      todo!()
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
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone)]
pub struct Year {
   raw: u32,
}

impl From<u32> for Year {
   fn from(value: u32) -> Self {
      Self { raw: value }
   }
}

type MonthMap<'p> = BTreeMap<Month, DayMap<'p>>;

type DayMap<'p> = BTreeMap<Day, BTreeSet<&'p Post<'p>>>;

#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Copy, Clone)]
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
