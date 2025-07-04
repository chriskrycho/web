use minijinja::Environment;
use nanohtml2text::html2text;

pub(crate) fn add_all(env: &mut Environment) {
   env.add_filter("strip_tags", strip_tags);
}

fn strip_tags(value: String) -> String {
   html2text(value.as_str())
}
