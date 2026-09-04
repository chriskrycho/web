mod filters;
mod functions;

use std::io::Write;

use camino::{Utf8Path, Utf8PathBuf};
use log::{debug, trace};
use minijinja::{Environment, context};
use thiserror::Error;

use crate::{data::config::Config, page::Item};

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not load templates: {source}")]
   Load {
      #[from]
      source: std::io::Error,
   },

   #[error("could not render template for {path}")]
   Render {
      source: minijinja::Error,
      path: Utf8PathBuf,
   },

   #[error("could not add template for {path}")]
   CouldNotAddTemplate {
      source: minijinja::Error,
      path: Utf8PathBuf,
   },

   #[error("could not load template for {path}: {source}")]
   MissingTemplate {
      source: minijinja::Error,
      path: Utf8PathBuf,
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
   item: &Item,
   config: &Config,
   dest: impl Write,
) -> Result<(), Error> {
   debug!(
      "Rendering page '{}' ({:?}) with layout '{}'",
      item.title(),
      item.path(),
      item.layout()
   );

   env.get_template(item.layout())
      .map_err(|source| Error::MissingTemplate {
         source,
         path: item.source().path.clone(),
      })?
      .render_captured_to(
         context! {
            content => item.content().html(),
            data => item.data(),
            config,
            path => item.path(),
            source => item.source(),
         },
         dest,
      )
      .map(|_captured| ()) // throw it away for now; return it if we need it later
      .map_err(|source| Error::Render {
         source,
         path: item.source().path.clone(),
      })
}
