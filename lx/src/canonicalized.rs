use std::io;

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone)]
pub struct Canonicalized {
   path: Utf8PathBuf,
}

impl AsRef<Utf8Path> for Canonicalized {
   fn as_ref(&self) -> &Utf8Path {
      &self.path
   }
}

impl std::fmt::Display for Canonicalized {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      write!(f, "{}", self.path)
   }
}

impl TryFrom<Utf8PathBuf> for Canonicalized {
   type Error = InvalidDir;

   fn try_from(value: Utf8PathBuf) -> Result<Self, Self::Error> {
      value
         .canonicalize_utf8()
         .map_err(|source| InvalidDir::new(&value, source))
         .map(|path| Canonicalized { path })
   }
}

impl TryFrom<&Utf8Path> for Canonicalized {
   type Error = InvalidDir;

   fn try_from(value: &Utf8Path) -> Result<Self, Self::Error> {
      value
         .canonicalize_utf8()
         .map_err(|source| InvalidDir::new(value, source))
         .map(|path| Canonicalized { path })
   }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid directory '{path}: {source}")]
pub struct InvalidDir {
   path: Utf8PathBuf,
   source: io::Error,
}

impl InvalidDir {
   fn new(path: &Utf8Path, source: io::Error) -> InvalidDir {
      InvalidDir {
         path: path.to_path_buf(),
         source,
      }
   }
}
