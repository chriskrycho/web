//! Common `Image` type to make it easy for me to use either a CDN path or a full URL.

use serde::{Deserialize, Serialize};

/// A resolved image URL.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Image {
   url: String,
}

impl Image {
   pub fn url(&self) -> &str {
      self.url.as_str()
   }
}

impl From<serial::Image> for Image {
   fn from(value: serial::Image) -> Self {
      let url = match value {
         serial::Image::Cdn(path) => format!("https://cdn.chriskrycho.com/images/{path}"),
         serial::Image::Url { url } => url,
      };
      Image { url }
   }
}

pub mod serial {
   use serde::{Deserialize, Serialize};

   #[derive(Serialize, Deserialize, Clone, Debug)]
   #[serde(untagged)]
   pub enum Image {
      Cdn(String),
      Url { url: String },
   }
}
