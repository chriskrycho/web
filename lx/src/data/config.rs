use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use super::image::Image;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
   pub url: String,
   pub repo: String,
   pub title: String,
   pub subtitle: Option<String>,
   pub description: String,
   pub author: serial::Author,
   pub output: Utf8PathBuf,
   pub image: Image,
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
         image: Image::from(serial_cfg.image),
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
   use minijinja::Environment;
   use normalize_path::NormalizePath as _;
   use serde::{Deserialize, Serialize};
   use thiserror::Error;

   use crate::{
      data::email::Email,
      templates::view::{self, View},
   };

   #[derive(Serialize, Deserialize, Debug)]
   pub struct Config {
      pub url: String,
      pub repo: String,
      pub title: Title,
      pub subtitle: Option<String>,
      pub description: String,
      pub author: Author,
      pub output: Utf8PathBuf,
      pub image: crate::data::image::serial::Image,
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

   // TODO: maybe move this elsewhere?
   impl View for NavItem {
      const VIEW_NAME: &'static str = "nav-item";

      fn view(&self, env: &Environment) -> Result<String, minijinja::Error> {
         env.get_template(&Self::template())?.render(self)
      }
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
