use std::{fmt, sync::Arc};

use minijinja::value::Object;

use crate::data::{config::NavItem, item::Qualifiers};

impl Object for NavItem {
   fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result
   where
      Self: Sized + 'static,
   {
      match self.as_ref() {
         NavItem::Separator => write!(f, r#"<hr>"#),
         NavItem::Page { title, path } => write!(f, r#"<a href="{path}">{title}</a>"#),
      }
   }
}

impl Object for Qualifiers {
   fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result
   where
      Self: Sized + 'static,
   {
      if !self.needs_to_render() {
         return fmt::Result::Ok(());
      }

      const OPEN: &str = r#"<p class="qualifier">"#;
      const CLOSE: &str = "</p>";

      let audience = self.audience.as_ref().map(|audience| format!(r#"{OPEN}<b><a href="https://v4.chriskrycho.com/2018/assumed-audiences.html">Assumed Audience</a>:</b> {audience}{CLOSE}"#)).unwrap_or_default();
      let epistemic = self.epistemic.as_ref().map(|epistemic| format!(r#"{OPEN}<b><a href='https://v5.chriskrycho.com/journal/epistemic-status/'>Epistemic status</a>:</b> {epistemic}{CLOSE}"#)).unwrap_or_default();
      let context = self
         .context
         .as_ref()
         .map(|context| format!(r#"{OPEN}<b>A bit of context:</b> {context}{CLOSE}"#))
         .unwrap_or_default();
      let discusses = self
         .discusses
         .as_ref()
         .map(|discusses| {
            format!(
               r#"{OPEN}<b>Heads up:</b> this post directly discusses {discusses}{CLOSE}"#
            )
         })
         .unwrap_or_default();
      let disclosure = self
         .disclosure
         .as_ref()
         .map(|disclosure| {
            format!(r#"{OPEN}<b>Full disclosure:</b> {disclosure}{CLOSE}"#)
         })
         .unwrap_or_default();
      let retraction = self
         .retraction.as_ref()
         .map(|retraction| {
            format!(
               r#"<p><strong>Caveat lector:</strong> I have since retracted this (see <a href="{url}">{title}</a>), but as a matter of policy I leave even work I have retracted publicly available as a matter of record.<p>"#,
               url=retraction.url,
               title=retraction.title
            )
         })
         .unwrap_or_default();

      write!(
         f,
         "{audience}{epistemic}{context}{discusses}{disclosure}{retraction}"
      )
   }

   fn call_method(
      self: &Arc<Self>,
      _state: &minijinja::State<'_, '_>,
      method: &str,
      _args: &[minijinja::Value],
   ) -> Result<minijinja::Value, minijinja::Error> {
      match method {
         "needs_to_render" => Ok(self.needs_to_render().into()),
         _ => Err(minijinja::Error::new(
            minijinja::ErrorKind::UnknownMethod,
            method.to_string(),
         )),
      }
   }
}
