use std::{
   io,
   path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Canonicalized {
   path: PathBuf,
}

impl AsRef<Path> for Canonicalized {
   fn as_ref(&self) -> &Path {
      &self.path
   }
}

impl std::fmt::Display for Canonicalized {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      write!(f, "{}", self.path.display())
   }
}

impl TryFrom<PathBuf> for Canonicalized {
   type Error = InvalidDir;

   fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
      let path = value.canonicalize().map_err(|source| InvalidDir {
         path: value.to_owned(),
         source,
      })?;
      Ok(Canonicalized { path })
   }
}

impl TryFrom<&Path> for Canonicalized {
   type Error = InvalidDir;

   fn try_from(value: &Path) -> Result<Self, Self::Error> {
      let path = value.canonicalize().map_err(|source| InvalidDir {
         path: value.to_owned(),
         source,
      })?;
      Ok(Canonicalized { path })
   }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid directory '{path}: {source}")]
pub struct InvalidDir {
   path: PathBuf,
   source: io::Error,
}
