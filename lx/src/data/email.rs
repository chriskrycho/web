use std::{fmt, str::FromStr};

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, de};

lazy_static! {
    /// An incredibly stupid email-"parsing" regex.
    static ref EMAIL_RE: Regex = Regex::new(r"(?P<local>[^@]+)@(?P<host>[^@]+)").unwrap();
}

#[derive(Debug)]
pub struct Email {
   validated: String,
}

impl<'de> Deserialize<'de> for Email {
   fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
   where
      D: Deserializer<'de>,
   {
      let s = String::deserialize(deserializer)?;
      Email::from_str(&s).map_err(de::Error::custom)
   }
}

impl Serialize for Email {
   fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
   where
      S: serde::Serializer,
   {
      serializer.serialize_str(&self.to_string())
   }
}

impl fmt::Display for Email {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "{}", self.validated)
   }
}

impl std::str::FromStr for Email {
   type Err = String;
   fn from_str(s: &str) -> Result<Self, Self::Err> {
      EMAIL_RE
         .captures(s)
         .ok_or(format!("could not parse {}", s))
         .and_then(
            |captures| match (captures.name("local"), captures.name("host")) {
               (Some(..), Some(..)) => Ok(Email {
                  validated: s.to_string(),
               }),
               (Some(..), None) => Err(format!("missing host name in {}", s)),
               (None, Some(..)) => Err(format!("missing username in {}", s)),
               _ => Err(format!("could not parse {}", s)),
            },
         )
         .map_err(|e| format!("email validation error: {}", e))
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn deserializes_correctly() {
      let result = serde_json::from_str::<Email>(r#""user@example.com""#).unwrap();
      assert_eq!(result.validated, "user@example.com");
   }

   #[test]
   fn reports_error_with_invalid_email() {
      let result = serde_json::from_str::<Email>(r#""not-an-email""#);
      let err = result.unwrap_err();
      assert_eq!(
         err.to_string(),
         "email validation error: could not parse not-an-email"
      );
   }
}
