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
   /// Definitions for which a corresponding reference has been found in the document.
   emitted_definitions: Vec<EmittedDefinition<'e>>,
}

struct EmittedDefinition<'e> {
   /// The name of the reference, like `foo` in `[^foo]`.
   ///
   /// Currently only used to keep track of whether I have seen the definition before, but
   /// it may also be useful for disambiguating references across documents if I find I
   /// need to do that at some point.
   ref_name: CowStr<'e>,

   events: Vec<pulldown_cmark::Event<'e>>,
   /// Can be combined with a backref index to produce a [`Backref`].
   ///
   /// This currently duplicates the item's position in the vector of `EmittedDefinition`s
   /// used to produce, but putting it here avoids relying implicitly on that
   /// implementation detail!
   footnote_index: usize,
   /// The number of back-references for a given footnote definition.
   backref_index_count: u8,
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

         // When I find footnote *references*, push the corresponding *definitions* into
         // the set of `emitted_definitions` so they will be rendered at the end of the
         // document. This approach guarantees two things:
         //
         // 1. I will always emit definitions in the order corresponding to the order the
         //    corresponding *reference* appears in the document.
         // 2. I will not emit definitions for which a reference never appears. (I do
         //    produce an error in that case, though, to be handled elsewhere!)
         first_pass::Event::FootnoteReference(name) => {
            match self.footnote_definitions.get(&name) {
               Some(definition) => {
                  // Only emit footnote definitions once. This means I need to track both
                  // whether I have emitted a given definition *and* how many times I have
                  // done so, so that I can target the correct footnote when generating a
                  // link and generate the correct `id` for the backref.
                  //
                  // Check whether I have previously emitted a definition, by finding it
                  // in the set of emitted definitions. A simple linear search *should* be
                  // fine here, as the number of emitted definitions should be pretty low
                  // in a large document.
                  //
                  // If I have previously emitted it, I will update its internal state;
                  // otherwise, I will emit it!

                  let previously_emitted = self
                     .emitted_definitions
                     .iter_mut() // so I can mutate `emitted_def` directly!
                     .find(|emitted| emitted.ref_name == name);

                  let (footnote_index, backref_index) = match previously_emitted {
                     Some(emitted_def) => {
                        emitted_def.backref_index_count += 1;
                        (emitted_def.footnote_index, emitted_def.backref_index_count)
                     }
                     None => {
                        let footnote_index = self.emitted_definitions.len() + 1;
                        let backref_index_count = 1;
                        self.emitted_definitions.push(EmittedDefinition {
                           ref_name: name.clone(),
                           events: definition.clone(),
                           footnote_index,
                           backref_index_count,
                        });
                        (footnote_index, backref_index_count)
                     }
                  };

                  let link = format!(
                     r##"<sup><a href="#{name}" id="{backref}">{footnote_index}</a></sup>"##,
                     name = footnote_ref_name(footnote_index),
                     backref = footnote_backref_name(Backref {
                        index: backref_index,
                        for_footnote_index: footnote_index,
                     }),
                  );

                  self.events.push(Html(link.into()));
                  Ok(HandleOutput::Normal)
               }
               None => {
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
}

#[inline]
fn footnote_ref_name(index: usize) -> String {
   format!("fn{index}")
}

#[inline]
fn footnote_backref_name(backref: Backref) -> String {
   format!(
      "fnref{index}{backref_index}",
      index = backref.for_footnote_index,
      backref_index = if backref.index == 0 {
         ""
      } else {
         &format!(":{}", backref.index)
      }
   )
}

/// A simple bit of structure for backrefs to use with [`footnote_backref_name`].
struct Backref {
   index: u8,
   for_footnote_index: usize,
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

         for (index, mut emitted) in self.emitted_definitions.into_iter().enumerate() {
            // Offset by 1 because I make sure the reference numbers and link numbers
            // match for the sake of URLs matching footnotes!
            events.push(Html(format!(r#"<li id="fn{}">"#, index + 1).into()));

            let backrefs = Html(
               // Back-refs start at 1, and I want to emit a backref for the total count.
               (1..=emitted.backref_index_count)
                  .map(|backref_index| {
                     format!(
                        r##"<a href="#{target}" class="fn-backref">↩{suffix}</a>"##,
                        target = footnote_backref_name(Backref {
                           index: backref_index,
                           for_footnote_index: emitted.footnote_index,
                        }),
                        suffix = if backref_index == 1 {
                           String::new()
                        } else {
                           format!("<sup>({})</sup>", backref_index)
                        }
                     )
                  })
                  .collect::<Vec<_>>()
                  .join(" ")
                  .into(),
            );

            if let Some(End(TagEnd::Paragraph)) = emitted.events.last() {
               let p = emitted.events.pop().unwrap();
               emitted.events.push(backrefs);
               emitted.events.push(p);
               events.append(&mut emitted.events);
            } else {
               events.append(&mut emitted.events);
               events.push(backrefs);
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
               "could not highlight code block tagged as `{}`",
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
