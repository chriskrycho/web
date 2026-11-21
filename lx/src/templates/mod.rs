mod filters;
mod functions;

use camino::{Utf8Path, Utf8PathBuf};
use log::{debug, trace};
use minijinja::Environment;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not load templates: {source}")]
   Load {
      #[from]
      source: std::io::Error,
   },

   #[error("could not render template {layout}: {source}")]
   Render {
      source: minijinja::Error,
      layout: String,
   },

   #[error("could not add template for {path}")]
   CouldNotAddTemplate {
      source: minijinja::Error,
      path: Utf8PathBuf,
   },

   #[error("could not load template {name} for: {source}")]
   MissingTemplate {
      name: String,
      source: minijinja::Error,
   },

   #[error(transparent)]
   Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

pub fn load<I, F>(templates: I, trim_root: F) -> Result<Environment<'static>, Error>
where
   I: IntoIterator,
   I::Item: AsRef<Utf8Path>,
   for<'a> F:
      Fn(&'a Utf8Path) -> Result<&'a Utf8Path, Box<dyn std::error::Error + Send + Sync>>,
{
   let mut env = Environment::new();
   env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
   for path in templates {
      let path = path.as_ref();
      let name = trim_root(path)?.to_string();
      let content = std::fs::read_to_string(path)?;
      trace!("Adding template at {name}");
      env.add_template_owned(name, content).map_err(|source| {
         Error::CouldNotAddTemplate {
            source,
            path: path.to_owned(),
         }
      })?;
   }

   filters::add_all(&mut env);
   functions::add_all(&mut env);

   Ok(env)
}

pub fn render(
   env: &Environment,
   layout: &str,
   context: impl Serialize,
) -> Result<String, Error> {
   env.get_template(layout)
      .map_err(|source| Error::MissingTemplate {
         source,
         name: layout.to_string(),
      })?
      .render(context)
      .map_err(|source| Error::Render {
         source,
         layout: layout.to_string(),
      })
}
