use std::{fmt, sync::Arc};

use minijinja::{
   value::{Object, Rest, ViaDeserialize},
   State, Value,
};

use crate::{
   data::{config::Config, image::Image, item::Metadata},
   page::RootedPath,
};

pub(crate) fn add_all(env: &mut minijinja::Environment<'_>) {
   env.add_function("label_for", label_for);
   env.add_function("resolved_title", resolved_title);
   env.add_function("resolved_image", resolved_image);
   env.add_function("description", description);
   env.add_function("url_for", url_for);
   env.add_function("fdbg", fancy_debug);
}

fn resolved_title(page_title: Option<String>, site_title: String) -> String {
   match page_title {
      Some(page_title) => {
         if page_title != site_title {
            page_title + " | " + &site_title
         } else {
            page_title
         }
      }
      None => site_title.clone(),
   }
}

fn url_for(
   ViaDeserialize(path): ViaDeserialize<RootedPath>,
   ViaDeserialize(config): ViaDeserialize<Config>,
) -> String {
   path.url(&config)
}

// TODO: generate image when it is not present and don’t fall back to config
// value; that will make it so there is no need to set it.
fn resolved_image(
   from_page: ViaDeserialize<Option<Image>>,
   from_config: ViaDeserialize<Image>,
) -> String {
   from_page
      .0
      .map(|image| image.url().to_string())
      .unwrap_or(from_config.0.url().to_string())
}

fn description(
   ViaDeserialize(page_data): ViaDeserialize<Metadata>,
   content: &str,
) -> String {
   page_data
      .summary
      .map(|summary| summary.plain())
      .or(
         page_data
            .book
            .and_then(|book| book.review.map(|review| review.to_string())),
      )
      .or(page_data.subtitle.map(|subtitle| subtitle.plain()))
      .unwrap_or_else(|| truncate(content))
}

fn truncate(content: &str) -> String {
   // TODO: strip the tags!
   if content.len() > 155 {
      let mut truncated = String::from(content);
      truncated.truncate(155);
      truncated += "…";
      truncated
   } else {
      content.to_string()
   }
}

fn fancy_debug(name: Option<&str>, args: Rest<Value>) -> String {
   let title = name.map(|n| format!("<p>{n}:</p>")).unwrap_or_default();
   let args = if args.is_empty() {
      "{no args!}".to_string()
   } else if args.len() == 1 {
      format!("{:#?}", args.0[0])
   } else {
      format!("{:#?}", &args.0[..])
   };

   format!("{title}<pre><code>{args}</code></pre>")
}

fn label_for(
   ViaDeserialize(page_data): ViaDeserialize<Metadata>,
   content: &str,
) -> Label {
   Label::new(page_data, content)
}

/// Data for the `twitter:(label|data)(1|2)` meta tags.
#[derive(Debug, serde::Serialize)]
enum Label {
   Post {
      length: ApproximateLength,
   },
   Work {
      duration: String,
      instrumentation: String,
   },
}

impl Label {
   pub fn new(page_data: Metadata, content: &str) -> Label {
      if let Some(work) = page_data.work {
         Label::Work {
            duration: work.duration,
            instrumentation: work.instrumentation,
         }
      } else {
         Label::Post {
            length: content.into(),
         }
      }
   }

   // Might later decide to do something more meaningful than Author/Composer
   // here given it’s pretty obviously just me on my own site?
   pub fn label1(&self) -> &'static str {
      match self {
         Label::Post { .. } => "Author",
         Label::Work { .. } => "Instrumentation",
      }
   }

   pub fn data1(&self) -> String {
      match self {
         Label::Post { .. } => "Chris Krycho".into(),
         Label::Work {
            instrumentation, ..
         } => instrumentation.to_owned(),
      }
   }

   pub fn label2(&self) -> &'static str {
      match self {
         Label::Post { .. } => "Length",
         Label::Work { .. } => "Duration",
      }
   }

   pub fn data2(&self) -> String {
      match self {
         Label::Post { length } => length.to_string(),
         Label::Work { duration, .. } => duration.clone(),
      }
   }
}

impl Into<Value> for Label {
   fn into(self) -> Value {
      Value::from_object(self)
   }
}

impl Object for Label {
   fn call_method(
      self: &Arc<Label>,
      _state: &State,
      name: &str,
      _args: &[Value],
   ) -> Result<Value, minijinja::Error> {
      match name {
         "label1" => Ok(self.label1().into()),
         "data1" => Ok(self.data1().into()),
         "label2" => Ok(self.label2().into()),
         "data2" => Ok(self.data2().into()),
         _ => Err(minijinja::Error::new(
            minijinja::ErrorKind::UnknownMethod,
            name.to_owned(),
         )),
      }
   }
}

#[derive(Debug, serde::Serialize)]
struct ApproximateLength {
   rounded: u64,
}

impl From<&str> for ApproximateLength {
   fn from(source: &str) -> Self {
      let actual = count_md::count(source);

      let rounded = if actual < 100 {
         (actual / 10) * 10
      } else if actual < 1_000 {
         (actual / 50) * 50
      } else {
         (actual / 100) * 100
      };

      ApproximateLength { rounded }
   }
}

impl fmt::Display for ApproximateLength {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      let formatted = {
         let formatted_number = self.rounded.to_string();
         if formatted_number.len() <= 3 {
            formatted_number
         } else {
            formatted_number
               .chars()
               .rev()
               .enumerate()
               .fold(String::new(), |mut s, (idx, c)| {
                  if idx > 0 && idx % 3 == 0 {
                     s.push(',');
                  }
                  s.push(c);
                  s
               })
               .chars()
               .rev()
               .collect()
         }
      };

      write!(f, "About {formatted} words")
   }
}
