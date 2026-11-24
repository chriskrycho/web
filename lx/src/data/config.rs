use camino::{Utf8Path, Utf8PathBuf};
use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::image::Image;

fn deserialize_time_zone<'de, D>(deserializer: D) -> Result<TimeZone, D::Error>
where
   D: Deserializer<'de>,
{
   let name = String::deserialize(deserializer)?;
   TimeZone::get(&name).map_err(serde::de::Error::custom)
}

fn serialize_time_zone<S>(time_zone: &TimeZone, serializer: S) -> Result<S::Ok, S::Error>
where
   S: Serializer,
{
   serializer.serialize_str(time_zone.iana_name().unwrap_or("UTC"))
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
   pub url: String,
   pub repo: String,
   pub title: String,
   pub subtitle: Option<String>,
   pub description: String,
   pub author: serial::Author,
   pub output: Utf8PathBuf,
   /// The IANA time zone in which local (offset-less) content dates are resolved.
   /// Required.
   #[serde(
      deserialize_with = "deserialize_time_zone",
      serialize_with = "serialize_time_zone",
      alias = "timezone"
   )]
   pub time_zone: TimeZone,
   pub image: Image,
   #[serde(default)]
   pub default_layout: Option<String>,
   #[serde(default)]
   pub nav: Vec<NavItem>,
}

impl Config {
   pub fn from_file(path: &Utf8Path) -> Result<Config, Error> {
      let serial_cfg = serial::Config::from_file(path)?;
      Ok(Config {
         url: serial_cfg.url,
         repo: serial_cfg.repo,
         title: serial_cfg.title.to_string(),
         subtitle: serial_cfg.subtitle,
         description: serial_cfg.description,
         author: serial_cfg.author,
         output: serial_cfg.output,
         time_zone: serial_cfg.time_zone,
         image: Image::from(serial_cfg.image),
         default_layout: serial_cfg.default_layout,
         nav: serial_cfg.nav,
      })
   }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error {
   #[from]
   source: serial::Error,
}

pub use serial::NavItem;

pub mod serial {
   use std::{collections::HashMap, fmt::Display};

   use camino::{Utf8Path, Utf8PathBuf};
   use normalize_path::NormalizePath as _;
   use serde::{Deserialize, Serialize};
   use thiserror::Error;

   use crate::data::email::Email;

   #[derive(Deserialize, Debug)]
   pub struct Config {
      pub url: String,
      pub repo: String,
      pub title: Title,
      pub subtitle: Option<String>,
      pub description: String,
      pub author: Author,
      pub output: Utf8PathBuf,
      /// The IANA time zone in which local (offset-less) content dates are resolved.
      /// Required.
      #[serde(deserialize_with = "super::deserialize_time_zone", alias = "timezone")]
      pub time_zone: jiff::tz::TimeZone,
      pub image: crate::data::image::serial::Image,
      #[serde(default)]
      pub default_layout: Option<String>,
      #[serde(default)]
      pub nav: Vec<NavItem>,
   }

   impl Config {
      pub fn from_file(path: &Utf8Path) -> Result<Config, Error> {
         let data = std::fs::read_to_string(path).map_err(|source| Error::BadFile {
            path: path.to_owned(),
            source,
         })?;

         let mut config: Config =
            serde_yaml::from_str(&data).map_err(|source| Error::YamlParsing {
               path: path.to_owned(),
               source,
            })?;

         config.output = path
            .parent()
            .unwrap_or_else(|| panic!("config file at {path} will have a parent dir",))
            .join(&config.output)
            .as_std_path()
            .normalize()
            .try_into()?;

         Ok(config)
      }
   }

   #[derive(Serialize, Deserialize, Debug)]
   pub struct Title {
      pub(crate) normal: String,
      pub(crate) stylized: Option<String>,
   }

   impl Display for Title {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
         f.write_str(self.stylized.as_ref().unwrap_or(&self.normal))
      }
   }

   #[derive(Serialize, Deserialize, Debug)]
   pub struct Author {
      pub name: String,
      pub email: Email,
      pub links: HashMap<String, String>,
   }

   #[derive(Serialize, Deserialize, Debug)]
   #[serde(tag = "type", rename_all = "snake_case")]
   pub enum NavItem {
      Separator,
      Page { title: String, path: String },
   }

   #[derive(Error, Debug)]
   pub enum Error {
      #[error("could not read file '{path}'")]
      BadFile {
         path: Utf8PathBuf,
         source: std::io::Error,
      },

      #[error("could not parse {path} as YAML")]
      YamlParsing {
         path: Utf8PathBuf,
         source: serde_yaml::Error,
      },

      #[error(transparent)]
      NonUtf8Path {
         #[from]
         source: camino::FromPathBufError,
      },
   }
}
