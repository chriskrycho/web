mod filters;
mod functions;

use camino::{Utf8Path, Utf8PathBuf};
use log::trace;
use minijinja::Environment;
use serde::Serialize;
use thiserror::Error;

pub const PASS_THROUGH: &str = "PASS_THROUGH";

const PASS_THROUGH_TEMPLATE: &str = "{{ content | safe }}";

#[derive(Error, Debug)]
pub enum Error {
   #[error("could not load template in '{path}'")]
   Load {
      source: std::io::Error,
      path: Utf8PathBuf,
   },

   #[error("could not render template '{layout}'")]
   Render {
      source: minijinja::Error,
      layout: String,
   },

   #[error("could not render standalone (no-`layout`) template")]
   RenderFree { source: minijinja::Error },

   #[error("could not add template for '{path}'")]
   CouldNotAddTemplate {
      source: minijinja::Error,
      path: Utf8PathBuf,
   },

   #[error("could not load template '{name}'")]
   MissingTemplate {
      name: String,
      source: minijinja::Error,
   },

   #[error("could not resolve the template name for '{path}'")]
   ResolveName {
      path: Utf8PathBuf,
      source: Box<dyn std::error::Error + Send + Sync>,
   },
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
      let name = trim_root(path)
         .map_err(|source| Error::ResolveName {
            path: path.to_owned(),
            source,
         })?
         .to_string();

      let content = std::fs::read_to_string(path).map_err(|source| Error::Load {
         source,
         path: path.to_owned(),
      })?;

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

   env.add_template_owned(PASS_THROUGH, PASS_THROUGH_TEMPLATE)
      .expect("It is always possible to add the PASS_THROUGH template");

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

pub fn render_free(
   src: impl AsRef<str>,
   env: &Environment,
   ctx: impl Serialize,
) -> Result<String, Error> {
   env.render_str(src.as_ref(), ctx)
      .map_err(|source| Error::RenderFree { source })
}

#[cfg(test)]
mod tests {
   use super::*;

   fn no_name(
      _: &Utf8Path,
   ) -> Result<&Utf8Path, Box<dyn std::error::Error + Send + Sync>> {
      Err("not under the template root".into())
   }

   #[test]
   fn load_reports_which_name_could_not_be_resolved() {
      let err = match load(["ui/base.jinja"], no_name) {
         Err(err) => err,
         Ok(_) => panic!("expected loading to fail"),
      };

      assert_eq!(
         err.to_string(),
         "could not resolve the template name for 'ui/base.jinja'"
      );
   }
}
