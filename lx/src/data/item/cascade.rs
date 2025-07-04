//! Manage the “data cascade”, i.e. data provided at various points in the
//! project hierarchy, which can then be merged with the metadata for a given
//! post.

use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use log::trace;
use thiserror::Error;

use crate::data::image::serial::Image;

use super::serial::*;

// NOTE: this is currently quite naïve and in fact *wrong* as a result: what I
// will actually need is a *tree*, where each point in the tree has two pieces
// of info: the path to that point, and the Metadata for that point. The path
// may want to be just the name of that point in the tree. (I *think* I need
// that, anyway!)
pub struct Cascade {
   inner: HashMap<Utf8PathBuf, Ambient>,
}

#[derive(Debug, Error)]
pub enum CascadeLoadError {
   #[error("failed to read file '{file}'")]
   OpenFile {
      source: std::io::Error,
      file: Utf8PathBuf,
   },

   #[error("could not parse metadata from '{file}'")]
   ParseMetadata {
      source: Box<dyn std::error::Error + Send + Sync>,
      file: Utf8PathBuf,
   },
}

impl Cascade {
   pub fn new(paths: &[Utf8PathBuf]) -> Result<Self, CascadeLoadError> {
      let mut cascade = Cascade {
         inner: HashMap::new(),
      };

      for path in paths {
         let fd = std::fs::File::open(path).map_err(|e| CascadeLoadError::OpenFile {
            source: e,
            file: path.clone(),
         })?;

         let metadata: Ambient = serde_yaml::from_reader(&fd).map_err(|e| {
            CascadeLoadError::ParseMetadata {
               source: Box::new(e),
               file: path.clone(),
            }
         })?;

         // Panic instead of returning a `Result` because this means there is
         // a real bug in the path construction (not something missing on disk).
         let context_dir = path
            .parent()
            .unwrap_or_else(|| panic!("missing parent of path {path}"));

         cascade.add_at(context_dir, metadata);
      }

      Ok(cascade)
   }

   pub fn add_at<P: AsRef<Utf8Path>>(&mut self, path: P, value: Ambient) -> &mut Self {
      trace!("Inserting {:?} at {}", value, path.as_ref());
      if let Some(existing) = self.inner.insert(path.as_ref().to_owned(), value) {
         panic!(
            "Bug: inserting data into `Cascade` for existing key: {key}.\nExisting data: {existing:?}",
            key = path.as_ref()
         );
      }
      self
   }

   pub fn layout<P: AsRef<Utf8Path>>(&self, p: P) -> Option<String> {
      self.find_map(p.as_ref(), &|m| m.layout.clone())
   }

   pub fn qualifiers<P: AsRef<Utf8Path>>(&self, p: P) -> Option<Qualifiers> {
      self.find_map(p.as_ref(), &|m| m.qualifiers.clone())
   }

   pub fn thanks<P: AsRef<Utf8Path>>(&self, p: P) -> Option<String> {
      self.find_map(p.as_ref(), &|m| m.thanks.clone())
   }

   pub fn tags<P: AsRef<Utf8Path>>(&self, p: P) -> Vec<String> {
      self
         .find_map(p.as_ref(), &|m| m.tags.clone())
         .unwrap_or_default()
   }

   pub fn subscribe<P: AsRef<Utf8Path>>(&self, p: P) -> Option<Subscribe> {
      self.find_map(p.as_ref(), &|m| m.subscribe.clone())
   }

   pub fn image<P: AsRef<Utf8Path>>(&self, p: P) -> Option<Image> {
      self.find_map(p.as_ref(), &|m| m.image.clone())
   }

   pub fn book<P: AsRef<Utf8Path>>(&self, p: P) -> Option<Book> {
      self.find_map(p.as_ref(), &|m| m.book.clone())
   }

   pub fn series<P: AsRef<Utf8Path>>(&self, p: P) -> Option<Series> {
      self.find_map(p.as_ref(), &|m| m.series.clone())
   }

   pub fn work<P: AsRef<Utf8Path>>(&self, path: P) -> Option<MusicalWork> {
      self.find_map(path.as_ref(), &|m| m.work.clone())
   }

   fn find_map<'a, T, F>(&'a self, path: &Utf8Path, f: &F) -> Option<T>
   where
      F: Fn(&'a Ambient) -> Option<T>,
   {
      let path = path.to_owned();
      self
         .inner
         .get(&path)
         .and_then(f)
         .or(path.parent().and_then(|parent| self.find_map(parent, f)))
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn direct_lookup_finds_entry() {
      let mut cascade = Cascade::new(&[]).unwrap();
      cascade.add_at(
         "basic-path",
         Ambient {
            layout: Some("index.hbs".into()),
            ..Default::default()
         },
      );

      assert_eq!(cascade.layout("basic-path"), Some("index.hbs".into()));
   }

   #[test]
   fn nested_lookup_finds_entry() {
      let mut cascade = Cascade::new(&[]).unwrap();
      cascade.add_at(
         "nested",
         Ambient {
            layout: Some("index.hbs".into()),
            ..Default::default()
         },
      );

      assert_eq!(cascade.layout("nested/path"), Some("index.hbs".into()));
   }

   #[test]
   fn direct_nesting_takes_last() {
      let mut cascade = Cascade::new(&[]).unwrap();
      cascade.add_at(
         "nested/path",
         Ambient {
            thanks: Some("To cool people".into()),
            ..Default::default()
         },
      );

      cascade.add_at(
         "nested",
         Ambient {
            thanks: Some("To lame people".into()),
            ..Default::default()
         },
      );

      assert_eq!(cascade.thanks("nested/path"), Some("To cool people".into()));
   }

   #[test]
   fn no_entry_is_none() {
      let cascade = Cascade::new(&[]).unwrap();
      assert_eq!(cascade.layout("path"), None);
   }

   #[test]
   fn no_matching_path_is_none() {
      let mut cascade = Cascade::new(&[]).unwrap();
      cascade.add_at(
         "some/path",
         Ambient {
            thanks: Some("to cool people".into()),
            ..Default::default()
         },
      );
      assert_eq!(cascade.thanks("other/path"), None);
   }

   #[test]
   fn no_matching_entry_is_none() {
      let mut cascade = Cascade::new(&[]).unwrap();
      cascade.add_at(
         "path",
         Ambient {
            thanks: Some("to cool people".into()),
            ..Default::default()
         },
      );
      assert_eq!(cascade.layout("path"), None);
   }
}
