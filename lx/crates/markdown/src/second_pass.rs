use std::error;

use arborium::Highlighter;
use log::{debug, error};
use pulldown_cmark::{CodeBlockKind, CowStr, Tag, TagEnd};
use thiserror::Error;

use super::FootnoteDefinitions;
use super::first_pass;

/// The second pass through the events is responsible for three tasks:
///
/// 1. Applying syntax highlighting.
/// 2. Properly emitting footnotes.
/// 3. Performing any template-language-type rewriting of text nodes.
struct State<'e, 's> {
   footnote_definitions: FootnoteDefinitions<'e>,
   highlighter: &'s mut Highlighter,
   code_block: Option<CodeBlock<'e>>,
   events: Vec<pulldown_cmark::Event<'e>>,
   emitted_definitions: Vec<(CowStr<'e>, Vec<pulldown_cmark::Event<'e>>)>,
}

#[derive(Error, Debug)]
pub enum Error {
   #[error("cannot finish a code block we never started")]
   FinishedNonStartedCodeBlock,

   #[error(
      "all footnote references are handled in the first pass but {0} is provided to the second pass"
   )]
   UnhandledFootnoteReference(String),

   #[error("syntax highlighting failure")]
   SyntaxHighlighting {
      #[from]
      source: arborium::Error,
   },

   #[error("bad LaTeX input")]
   BadLatex {
      #[from]
      source: latex2mathml::LatexError,
   },

   #[error("Could not rewrite text")]
   Rewrite {
      source: Box<dyn error::Error + Send + Sync>,
      original: String,
   },
}

pub(super) fn second_pass<'e>(
   footnote_definitions: FootnoteDefinitions<'e>,
   highlighter: &mut Highlighter,
   events: Vec<first_pass::Event<'e>>,
   rewrite: impl Fn(&str) -> Result<String, Box<dyn error::Error + Send + Sync>>,
) -> Result<impl Iterator<Item = pulldown_cmark::Event<'e>>, Error> {
   let mut state = State {
      footnote_definitions,
      highlighter,
      code_block: None,
      events: vec![],
      emitted_definitions: vec![],
   };

   for event in events {
      // If I ever extract/generalize this, I will want to use some kind of log level
      // handling instead of just always emitting the error.
      if let HandleOutput::Weird(warning) = state.handle(event, &rewrite)? {
         error!("{warning}");
      }
   }

   Ok(state.into_iter())
}

enum HandleOutput {
   Normal,
   Weird(String),
}

impl<'e> State<'e, '_> {
   fn handle(
      &mut self,
      event: first_pass::Event<'e>,
      rewrite: &impl Fn(&str) -> Result<String, Box<dyn error::Error + Send + Sync>>,
   ) -> Result<HandleOutput, Error> {
      use pulldown_cmark::Event::*;

      match event {
         first_pass::Event::Basic(basic) => match basic {
            Text(text) => {
               // We do *not* want to rewrite text in code blocks!
               match self.code_block {
                  Some(ref mut code_block) => {
                     code_block.highlight(text, self.highlighter)?;
                     Ok(HandleOutput::Normal)
                  }
                  None => {
                     let rewritten =
                        rewrite(text.as_ref()).map_err(|source| Error::Rewrite {
                           source,
                           original: text.to_string(),
                        })?;
                     self.events.push(Html(rewritten.into()));
                     Ok(HandleOutput::Normal)
                  }
               }
            }

            Start(Tag::CodeBlock(kind)) => {
               self.code_block = CodeBlock::start(kind);
               Ok(HandleOutput::Normal)
            }

            End(TagEnd::CodeBlock) => match self.code_block.take() {
               Some(code_block) => {
                  self.events.append(&mut code_block.end());
                  Ok(HandleOutput::Normal)
               }
               None => Err(Error::FinishedNonStartedCodeBlock),
            },

            DisplayMath(content) => {
               let math = latex2mathml::latex_to_mathml(
                  content.as_ref(),
                  latex2mathml::DisplayStyle::Block,
               )?;
               self.events.push(Html(math.into()));
               Ok(HandleOutput::Normal)
            }

            InlineMath(content) => {
               let math = latex2mathml::latex_to_mathml(
                  content.as_ref(),
                  latex2mathml::DisplayStyle::Inline,
               )?;
               self.events.push(Html(math.into()));
               Ok(HandleOutput::Normal)
            }

            // If we find a footnote reference here, something has gone wrong: we should
            // have handled them all during `first_pass`.
            FootnoteReference(name) => {
               Err(Error::UnhandledFootnoteReference(name.to_string()))
            }

            // Everything else can just be emitted exactly as is.
            other => {
               self.events.push(other.clone());
               Ok(HandleOutput::Normal)
            }
         },

         first_pass::Event::FootnoteReference(name) => {
            if let Some(definition) = self.footnote_definitions.get(&name) {
               self.emitted_definitions.push((name, definition.clone()));
               let index = self.emitted_definitions.len();
               let link = format!(
                  r##"<sup><a href="#{name}" id="{backref}">{index}</a></sup>"##,
                  name = footnote_ref_name(index),
                  backref = footnote_backref_name(index),
               );

               self.events.push(Html(link.into()));
               Ok(HandleOutput::Normal)
            } else {
               let event = Text(format!("[^{name}]").into());
               self.events.push(event);
               Ok(HandleOutput::Weird(format!(
                  "Missing definition for footnote labeled '{name}'"
               )))
            }
         }
      }
   }
}

#[inline]
fn footnote_ref_name(index: usize) -> String {
   format!("fn{index}")
}

#[inline]
fn footnote_backref_name(index: usize) -> String {
   format!("fnref{index}")
}

impl<'e> IntoIterator for State<'e, '_> {
   type Item = pulldown_cmark::Event<'e>;
   type IntoIter = std::vec::IntoIter<pulldown_cmark::Event<'e>>;

   fn into_iter(self) -> Self::IntoIter {
      use pulldown_cmark::Event::*;

      let mut events = self.events;

      if !self.emitted_definitions.is_empty() {
         events.push(Rule);
         events.push(Html(
            r#"<section class="footnotes"><ol class="footnotes-list">"#.into(),
         ));

         for (index, _, mut definition_events) in self
            .emitted_definitions
            .into_iter()
            .enumerate()
            .map(|(index, (name, evts))| (index + 1, name, evts))
         {
            events.push(Html(format!(r#"<li id="fn{index}">"#).into()));

            let backref = Html(
               format!(
                  r##"<a href="#{backref}" class="fn-backref">↩</a>"##,
                  backref = footnote_backref_name(index)
               )
               .into(),
            );

            if let Some(End(TagEnd::Paragraph)) = definition_events.last() {
               let p = definition_events.pop().unwrap();
               definition_events.push(backref);
               definition_events.push(p);
               events.append(&mut definition_events);
            } else {
               events.append(&mut definition_events);
               events.push(backref);
            }

            events.push(End(TagEnd::Item));
         }

         events.push(Html("</ol></section>".into()));
      }

      events.into_iter()
   }
}

struct CodeBlock<'e> {
   name: CowStr<'e>,
   events: Vec<pulldown_cmark::Event<'e>>,
}

impl<'e> CodeBlock<'e> {
   /// Start highlighting a code block.
   fn start(kind: CodeBlockKind<'e>) -> Option<Self> {
      match kind {
         CodeBlockKind::Fenced(name) => {
            let lang = match name.as_ref() {
               "sh" => "bash",
               _ => &name,
            };
            let leading_html = pulldown_cmark::Event::Html(
               format!(r#"<pre lang="{lang}"><code class="{lang}">"#).into(),
            );
            Some(CodeBlock {
               name,
               events: vec![leading_html],
            })
         }
         // `arborium` does not support parsing from the text, and I always specify the
         // type on the “fence” anyway so in this case I *expect* no parsing to happen.
         CodeBlockKind::Indented => None,
      }
   }

   fn highlight(
      &mut self,
      text: CowStr<'_>,
      highlighter: &mut Highlighter,
   ) -> Result<(), Error> {
      let highlighted_if_possible = match highlighter.highlight(&self.name, &text) {
         Ok(s) => {
            debug!("highlighted some {} code", self.name);
            s
         }
         Err(arborium::Error::UnsupportedLanguage { language }) => {
            debug!(
               "could not highlight {} code",
               if language.is_empty() {
                  "(unknown)"
               } else {
                  &language
               }
            );
            text.to_string()
         }
         Err(highlight_err) => return Err(highlight_err.into()),
      };

      self
         .events
         .push(pulldown_cmark::Event::Html(highlighted_if_possible.into()));

      Ok(())
   }

   fn end(mut self) -> Vec<pulldown_cmark::Event<'e>> {
      self
         .events
         .push(pulldown_cmark::Event::Html("</code></pre>".into()));
      self.events
   }
}
