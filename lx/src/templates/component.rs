use minijinja::Environment;
use serde::Serialize;

pub(crate) trait Component: Serialize + Sized {
   const VIEW_NAME: &'static str;

   fn view(&self, env: &Environment) -> Result<String, minijinja::Error> {
      env.get_template(&Self::template())?.render(self)
   }

   fn template() -> String {
      format!("components/{}.jinja", Self::VIEW_NAME)
   }
}
