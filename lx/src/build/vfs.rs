use std::{collections::BTreeMap, fmt, ops::Bound};

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Default)]
pub struct Vfs {
   entries: BTreeMap<Utf8PathBuf, Provenance>,
}

impl Vfs {
   pub fn new() -> Vfs {
      Vfs {
         entries: BTreeMap::new(),
      }
   }

   pub fn insert(
      &mut self,
      path: &Utf8Path,
      provenance: Provenance,
   ) -> Result<(), Error> {
      let ancestor = path
         .ancestors()
         .skip(1) // first ancestor is self
         .find(|&ancestor| self.entries.contains_key(ancestor));

      if let Some(existing_path) = ancestor {
         return Err(Error::ConflictingOutputs {
            path: path.to_owned(),
            existing_path: existing_path.to_owned(),
            existing: self.entries[existing_path].clone(),
            incoming: provenance,
         });
      }

      let descendant = self
         .entries
         .range((Bound::Excluded(path.to_owned()), Bound::Unbounded)) // the map is sorted, so descendants *follow* `path`
         .find(|(candidate, _)| candidate.starts_with(path));

      if let Some((existing_path, existing)) = descendant {
         return Err(Error::ConflictingOutputs {
            path: path.to_owned(),
            existing_path: existing_path.to_owned(),
            existing: existing.to_owned(),
            incoming: provenance,
         });
      }

      match self.entries.insert(path.to_owned(), provenance.clone()) {
         Some(existing) => Err(Error::DuplicateOutput {
            path: path.to_owned(),
            existing,
            incoming: provenance,
         }),
         None => Ok(()),
      }
   }
}

#[derive(Debug, Clone)]
pub enum Provenance {
   Content(Utf8PathBuf),
   Static(Utf8PathBuf),
   Style(Utf8PathBuf),
}

impl fmt::Display for Provenance {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let (kind, path) = match self {
         Provenance::Content(path) => ("content", path),
         Provenance::Static(path) => ("static file", path),
         Provenance::Style(path) => ("stylesheet", path),
      };

      write!(f, "{kind} '{path}'")
   }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
   #[error("'{path}' is produced more than once: by {existing} and by {incoming}")]
   DuplicateOutput {
      path: Utf8PathBuf,
      existing: Provenance,
      incoming: Provenance,
   },

   #[error("'{path}' from {incoming} conflicts with '{existing_path}' from {existing}")]
   ConflictingOutputs {
      path: Utf8PathBuf,
      existing_path: Utf8PathBuf,
      existing: Provenance,
      incoming: Provenance,
   },
}

#[cfg(test)]
mod tests {
   use camino::Utf8Path;

   use super::*;

   fn content(path: &str) -> Provenance {
      Provenance::Content(Utf8PathBuf::from(path))
   }

   #[test]
   fn vfs_rejects_duplicate_outputs() {
      let mut vfs = Vfs::new();

      vfs.insert(
         Utf8Path::new("foo.json"),
         Provenance::Content(Utf8PathBuf::from("content/foo.json")),
      )
      .unwrap();

      let error = vfs
         .insert(
            Utf8Path::new("foo.json"),
            content("content/foo.markdown.json"),
         )
         .unwrap_err();

      assert!(matches!(
         error,
         Error::DuplicateOutput {
            existing: Provenance::Content(_),
            incoming: Provenance::Content(_),
            ..
         }
      ));
   }

   #[test]
   fn vfs_rejects_a_file_and_a_directory_at_the_same_path() {
      let mut vfs = Vfs::new();
      vfs.insert(Utf8Path::new("foo"), content("content/foo"))
         .unwrap();

      assert!(matches!(
         vfs.insert(Utf8Path::new("foo/index.html"), content("content/foo.md")),
         Err(Error::ConflictingOutputs { .. })
      ));

      let mut vfs = Vfs::new();
      vfs.insert(Utf8Path::new("foo/index.html"), content("content/foo.md"))
         .unwrap();

      assert!(matches!(
         vfs.insert(Utf8Path::new("foo"), content("content/foo")),
         Err(Error::ConflictingOutputs { .. })
      ));
   }
}
