use log::debug;
use minijinja::Environment;
use serde::Serialize;

pub(crate) trait View: Serialize + Sized {
   const VIEW_NAME: &'static str;

   fn view(&self, env: &Environment) -> Result<String, minijinja::Error> {
      env.get_template(&template_for(self))?.render(self)
   }
}

// The slightly-quirky approach here is one I am experimenting with, not sure about. It
// has the benefit/cost of not being able to do `template_for<SomeView>()` and requires
// me to pass an actual `impl View` type instead. It has *worse* monomorphization
// characteristics than just using an outer generic function! (This is sort of the
// inverse of where you do the non-generic inner function.) But it also has the upside
// of making it a bit more “normal” at the call side?
/// Get the template name for a given View given the conventional layout of my projects.
///
/// As of today, a template named `"foo"` will be resolved as `"view/foo.jinja"`.
pub(crate) fn template_for(view: &impl View) -> String {
   fn helper<V: View>(_: &V) -> String {
      format!("views/{}.jinja", V::VIEW_NAME)
   }

   helper(view)
}
