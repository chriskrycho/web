//! Date and time helpers.
//!
//! Content dates may be written as local dates and times resolved in the site time zone
//! zone *or* (for legacy reasons) as RFC 3339 (same as ISO 8601 for these purposes) with
//! an explicit offset (`2025-06-23T18:55:00-0600`). [`jiff::ZonedDateTime`] already
//! stores the instant alongside either the fixed offset or the IANA time zone, so this
//! module only supplies the config-aware parsing, the RFC 3339 printing, and the wrapper
//! type that templates see as an object.

use std::{fmt, ops::Deref, sync::Arc};

use jiff::{
   civil,
   fmt::temporal::{DateTimeParser, DateTimePrinter},
   tz::TimeZone,
   ZonedDateTime,
};
use minijinja::{
   value::{Object, ObjectRepr},
   State, Value,
};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

const PARSER: DateTimeParser = DateTimeParser::new();
const PRINTER: DateTimePrinter = DateTimePrinter::new();

/// A date and time as written in content.
///
/// This derefs to [`ZonedDateTime`], so the full jiff API is available, but it renders
/// as RFC 3339 both in templates and when serialized.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date(ZonedDateTime);

impl Date {
   /// The RFC 3339 representation of this date, e.g. `2025-06-23T18:55:00-06:00`.
   ///
   /// This uses jiff's [`DateTimePrinter`], which writes `+00:00` (not `Z`) for UTC.
   pub fn to_rfc3339(&self) -> String {
      PRINTER.timestamp_with_offset_to_string(&self.0.timestamp(), self.0.offset())
   }
}

impl fmt::Debug for Date {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      f.write_str(&self.to_rfc3339())
   }
}

impl Deref for Date {
   type Target = ZonedDateTime;

   fn deref(&self) -> &Self::Target {
      &self.0
   }
}

impl Object for Date {
   fn repr(self: &Arc<Self>) -> ObjectRepr {
      ObjectRepr::Plain
   }

   fn call_method(
      self: &Arc<Self>,
      _state: &State,
      name: &str,
      _args: &[Value],
   ) -> Result<Value, minijinja::Error> {
      match name {
         "to_rfc3339" => Ok(Value::from(self.to_rfc3339())),
         _ => Err(minijinja::Error::new(
            minijinja::ErrorKind::UnknownMethod,
            name.to_owned(),
         )),
      }
   }

   fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      f.write_str(&self.to_rfc3339())
   }
}

impl From<Date> for Value {
   fn from(value: Date) -> Self {
      Value::from_object(value)
   }
}

impl Serialize for Date {
   /// Round-tripping through [`Value`] makes the date an object in template context,
   /// while serializers outside MiniJinja see the rendered RFC 3339 string.
   fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
   where
      S: Serializer,
   {
      Value::from_object(self.clone()).serialize(serializer)
   }
}

impl<'de> Deserialize<'de> for Date {
   /// Deserialize a date written as RFC 3339 or as a local date and time in UTC. Callers
   /// that know the site time zone should prefer [`parse`].
   fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
   where
      D: Deserializer<'de>,
   {
      struct DateTimeVisitor;

      impl de::Visitor<'_> for DateTimeVisitor {
         type Value = Date;

         fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an RFC 3339 formatted date and time string")
         }

         fn visit_str<E>(self, value: &str) -> Result<Date, E>
         where
            E: de::Error,
         {
            parse(value, &TimeZone::UTC).map_err(E::custom)
         }
      }

      deserializer.deserialize_str(DateTimeVisitor)
   }
}

/// Parse a date and time written as either RFC 3339 or a local date and time.
///
/// An explicit UTC offset in `source` always wins and is preserved as written.
/// Otherwise, `source` is interpreted as a civil date and time in `time_zone`, and a
/// date without a time means midnight.
///
/// # Errors
///
/// This returns an error if `source` is not a parseable date and time or cannot be
/// represented as a timestamp.
pub fn parse(source: &str, time_zone: &TimeZone) -> Result<Date, jiff::Error> {
   let pieces = PARSER.parse_pieces(source.as_bytes())?;
   let time = pieces.time().unwrap_or_else(civil::Time::midnight);
   let civil = pieces.date().to_datetime(time);
   let zoned = match pieces.to_numeric_offset() {
      Some(offset) => civil.to_zoned_datetime(TimeZone::fixed(offset)),
      None => civil.to_zoned_datetime(time_zone.clone()),
   }?;

   Ok(Date(zoned))
}

#[cfg(test)]
mod tests {
   use minijinja::{context, Environment};

   use super::*;

   fn denver() -> TimeZone {
      TimeZone::get("America/Denver").unwrap()
   }

   #[test]
   fn explicit_offset_is_preserved() {
      let parsed = parse("2025-06-23T18:55:00-04:00", &denver()).unwrap();
      assert_eq!(parsed.to_rfc3339(), "2025-06-23T18:55:00-04:00");
   }

   #[test]
   fn local_datetime_uses_the_supplied_time_zone() {
      let parsed = parse("2025-06-23 18:55", &denver()).unwrap();
      assert_eq!(parsed.to_rfc3339(), "2025-06-23T18:55:00-06:00");
   }

   #[test]
   fn local_datetime_uses_the_supplied_time_zone_in_winter() {
      let parsed = parse("2025-01-15 12:00", &denver()).unwrap();
      assert_eq!(parsed.to_rfc3339(), "2025-01-15T12:00:00-07:00");
   }

   #[test]
   fn date_only_means_midnight_in_the_supplied_time_zone() {
      let parsed = parse("2025-06-23", &denver()).unwrap();
      assert_eq!(parsed.to_rfc3339(), "2025-06-23T00:00:00-06:00");
   }

   #[test]
   fn serializes_as_rfc3339_outside_templates() {
      let date = parse("2025-06-23 18:55", &denver()).unwrap();

      assert_eq!(
         serde_yaml::to_string(&date).unwrap(),
         "2025-06-23T18:55:00-06:00\n"
      );
      assert_eq!(
         serde_yaml::from_str::<Date>("2025-06-23T18:55:00-06:00")
            .unwrap()
            .to_rfc3339(),
         "2025-06-23T18:55:00-06:00"
      );
   }

   #[test]
   fn renders_in_templates_as_an_object_and_as_rfc3339() {
      let date = parse("2025-06-23 18:55", &denver()).unwrap();
      let env = Environment::new();

      let rendered = env
         .render_str(
            "{{ date.to_rfc3339() }}|{{ date }}",
            context! { date => date },
         )
         .unwrap();

      assert_eq!(
         rendered,
         "2025-06-23T18:55:00-06:00|2025-06-23T18:55:00-06:00"
      );
   }

   #[test]
   fn nested_dates_are_objects_in_templates() {
      #[derive(serde::Serialize)]
      struct Wrapper {
         date: Date,
      }

      let date = parse("2025-06-23 18:55", &denver()).unwrap();
      let wrapper = Value::from_serialize(Wrapper { date });
      let env = Environment::new();

      let rendered = env
         .render_str(
            "{{ wrapper.date.to_rfc3339() }}|{{ wrapper.date }}",
            context! { wrapper => wrapper },
         )
         .unwrap();

      assert_eq!(
         rendered,
         "2025-06-23T18:55:00-06:00|2025-06-23T18:55:00-06:00"
      );
   }
}
