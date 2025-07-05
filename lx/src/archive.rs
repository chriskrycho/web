use std::collections::HashMap;

use chrono::{Datelike, Month};
use thiserror::Error;

use crate::page::Page;

pub struct Archive<'p>(HashMap<Year, MonthMap<'p>>);

impl<'e> Archive<'e> {
   pub fn new(pages: &'e [Page<'e>], order: Order) -> Result<Archive<'e>, Error> {
      let mut pages = pages
         .iter()
         .filter(|page| page.data.date.is_some())
         .collect::<Vec<&Page>>();

      pages.sort_by(|a, b| {
         // I just filtered to items which have dates.
         let a_date = a.data.date.unwrap();
         let b_date = b.data.date.unwrap();
         match order {
            Order::OldFirst => a_date.partial_cmp(&b_date).unwrap(),
            Order::NewFirst => b_date.partial_cmp(&a_date).unwrap(),
         }
      });

      let mut year_map = HashMap::new();

      for page in pages {
         if let Some(date) = &page.data.date {
            let year = date.year_ce().1;

            let month = date.month();
            let month = Month::try_from(u8::try_from(month).unwrap())
               .map_err(|source| Error::BadMonth { raw: month, source })?;

            let day = Day::try_from(date.day()).map_err(Error::from)?;

            let month_map = year_map.entry(year).or_insert_with(HashMap::new);
            let day_map = month_map.entry(month).or_insert_with(HashMap::new);
            day_map.entry(day).or_insert_with(Vec::new).push(page);
         }
      }

      Ok(Archive(year_map))
   }
}

pub enum Order {
   #[allow(dead_code)]
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

type Year = u32;

type MonthMap<'p> = HashMap<Month, DayMap<'p>>;

type DayMap<'p> = HashMap<Day, Vec<&'p Page<'p>>>;

#[derive(Debug, PartialEq, Eq, Hash)]
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
