use std::io::{Read, Write};

use serde_yaml::Value;

pub struct Include {
   pub metadata: bool,
   pub wrapping_html: bool,
}

pub fn convert(
   mut input: Box<dyn Read>,
   output: &mut Box<dyn Write>,
   include: Include,
) -> Result<(), Error> {
   let mut src = String::new();
   input
      .read_to_string(&mut src)
      .map_err(|source| Error::ReadBuffer { source })?;

   let (meta, rendered) = lx_md::Markdown::new(None)
      .render(&src, |s| Ok(s.to_string()))
      .map_err(Error::from)?;

   if include.wrapping_html {
      write(
         r#"<html>
          <head>
              <link rel="stylesheet" href="/light.css" media="(prefers-color-scheme: light)" />
              <link rel="stylesheet" href="/dark.css" media="(prefers-color-scheme: dark)" />
          </head>
          <body>"#,
         output,
      )?;
   }

   if include.metadata {
      if let Some(metadata) = meta {
         let metadata_table = match serde_yaml::from_str::<Value>(&metadata)? {
            // Allowed, carry on. Uses `value` so that `yaml_to_value` below can simply be
            // a recursive function, with no special casing for `value`; I handle that
            // here.
            value @ Value::Mapping(_) => Ok(value),

            // Not allowed!
            Value::Null => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::Null,
               src: "null".to_string(),
            }),
            Value::Bool(src) => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::Bool,
               src: src.to_string(),
            }),
            Value::Number(src) => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::Number,
               src: src.to_string(),
            }),
            Value::String(src) => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::String,
               src,
            }),
            Value::Sequence(src) => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::Sequence,
               src: format!("{src:?}"),
            }),
            Value::Tagged(src) => Err(Error::CouldNotRenderYamlMetadata {
               kind: InvalidKind::Tagged,
               src: format!("{src:?}"),
            }),
         }?;

         yaml_to_html(&metadata_table, output)?;
      }
   }

   write(rendered.html(), output)?;

   if include.wrapping_html {
      write("</body></html>", output)?;
   }

   Ok(())
}

fn write(src: &str, dest: &mut Box<dyn Write>) -> Result<(), Error> {
   dest
      .write_all(src.as_bytes())
      .map_err(|source| Error::WriteBuffer { source })
}

fn yaml_to_html(source: &Value, output: &mut Box<dyn Write>) -> Result<(), Error> {
   match source {
      Value::Null => write("(null)", output),
      Value::Bool(bool) => write(&bool.to_string(), output),
      Value::Number(number) => write(&number.to_string(), output),
      Value::String(string) => write(string, output),
      Value::Sequence(values) => {
         write("<ul>", output)?;
         for value in values {
            yaml_to_html(value, output)?;
         }
         write("</ul>", output)?;
         Ok(())
      }
      Value::Mapping(mapping) => {
         write("<table>", output)?;
         let (keys, values) = mapping.into_iter().collect::<(Vec<_>, Vec<_>)>();
         if !keys.is_empty() {
            write("<thead><tr>", output)?;
            for key in keys {
               write("<th>", output)?;
               yaml_to_html(key, output)?;
               write("</th>", output)?;
            }
            write("</tr></thead>", output)?;

            write("<tbody><tr>", output)?;
            for value in values {
               write("<td>", output)?;
               yaml_to_html(value, output)?;
               write("</td>", output)?;
            }
            write("</tr></tbody>", output)?;
         }

         write("</table>", output)?;
         Ok(())
      }
      Value::Tagged(tagged_value) => write(&format!("{tagged_value:?}"), output),
   }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
   #[error("could not read buffer")]
   ReadBuffer { source: std::io::Error },

   #[error("could not write to buffer")]
   WriteBuffer { source: std::io::Error },

   #[error(transparent)]
   CouldNotParseYaml {
      #[from]
      source: serde_yaml::Error,
   },

   #[error(transparent)]
   Render {
      #[from]
      source: lx_md::Error,
   },

   #[error(
      "Could not render YAML metadata as an HTML table. Instead of a table it was: {src}"
   )]
   CouldNotRenderYamlMetadata { kind: InvalidKind, src: String },
}

#[derive(Debug)]
pub enum InvalidKind {
   Null,
   Bool,
   Number,
   String,
   Sequence,
   Tagged,
}
