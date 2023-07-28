mod email;

use normalize_path::NormalizePath;
use serde::Serialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

use serde_derive::Deserialize;

use email::Email;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
   pub url: String,
   pub repo: String,
   pub title: Title,
   pub subtitle: String,
   pub description: String,
   pub author: Author,
   pub output: PathBuf,
}

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not read file '{path}'")]
   BadFile {
      path: PathBuf,
      source: std::io::Error,
   },

   #[error("could not parse {path} as JSON5")]
   JSON5ParseError { path: PathBuf, source: json5::Error },
}

impl Config {
   pub fn from_file(path: &Path) -> Result<Config, Error> {
      let data = std::fs::read_to_string(path).map_err(|e| Error::BadFile {
         path: path.to_owned(),
         source: e,
      })?;

      let mut config: Config =
         json5::from_str(&data).map_err(|e| Error::JSON5ParseError {
            path: path.to_owned(),
            source: e,
         })?;

      config.output = path
         .parent()
         .unwrap_or_else(|| {
            panic!(
               "config file at {path} will have a parent dir",
               path = path.display()
            )
         })
         .join(&config.output)
         .normalize();

      Ok(config)
   }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Title {
   normal: String,
   stylized: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Author {
   pub name: String,
   #[serde(deserialize_with = "Email::de_from_str")]
   pub email: Email,
   pub links: Vec<String>,
}
