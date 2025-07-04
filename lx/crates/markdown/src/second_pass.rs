use std::error;

use log::error;
use pulldown_cmark::{CodeBlockKind, CowStr, Tag, TagEnd};
use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
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
   syntax_set: &'s SyntaxSet,
   code_block: Option<CodeBlock<'e, 's>>,
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
   BadSyntaxLine { source: syntect::Error },

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
   syntax_set: &SyntaxSet,
   events: Vec<first_pass::Event<'e>>,
   rewrite: impl Fn(&str) -> Result<String, Box<dyn error::Error + Send + Sync>>,
) -> Result<impl Iterator<Item = pulldown_cmark::Event<'e>>, Error> {
   let mut state = State {
      footnote_definitions,
      syntax_set,
      code_block: None,
      events: vec![],
      emitted_definitions: vec![],
   };

   for event in events {
      // If I ever extract/generalize this, I will want to use some kind of log level
      // handling instead of just always emitting the error.
      if let Some(warning) = state.handle(event, &rewrite)? {
         error!("{warning}");
      }
   }

   Ok(state.into_iter())
}

impl<'e> State<'e, '_> {
   /// Returns `Some(String)` when it could successfully emit an event, but there was
   /// something unexpected about it, e.g. a footnote with a missing definition.
   fn handle(
      &mut self,
      event: first_pass::Event<'e>,
      rewrite: &impl Fn(&str) -> Result<String, Box<dyn error::Error + Send + Sync>>,
   ) -> Result<Option<String>, Error> {
      use pulldown_cmark::Event::*;

      match event {
         first_pass::Event::Basic(basic) => match basic {
            Text(text) => {
               // We do *not* want to rewrite text in code blocks!
               match self.code_block {
                  Some(ref mut code_block) => {
                     code_block.highlight(&text)?;
                     Ok(None)
                  }
                  None => {
                     let rewritten =
                        rewrite(text.as_ref()).map_err(|source| Error::Rewrite {
                           source,
                           original: text.to_string(),
                        })?;
                     self.events.push(Html(rewritten.into()));
                     Ok(None)
                  }
               }
            }

            Start(Tag::CodeBlock(kind)) => {
               self.code_block = Some(CodeBlock::start(kind, self.syntax_set));
               Ok(None)
            }

            End(TagEnd::CodeBlock) => match self.code_block.take() {
               Some(code_block) => {
                  self.events.append(&mut code_block.end());
                  Ok(None)
               }
               None => Err(Error::FinishedNonStartedCodeBlock),
            },

            DisplayMath(content) => {
               let math = latex2mathml::latex_to_mathml(
                  content.as_ref(),
                  latex2mathml::DisplayStyle::Block,
               )?;
               self.events.push(Html(math.into()));
               Ok(None)
            }

            InlineMath(content) => {
               let math = latex2mathml::latex_to_mathml(
                  content.as_ref(),
                  latex2mathml::DisplayStyle::Inline,
               )?;
               self.events.push(Html(math.into()));
               Ok(None)
            }

            // If we find a footnote reference here, something has gone wrong: we should
            // have handled them all during `first_pass`.
            FootnoteReference(name) => {
               Err(Error::UnhandledFootnoteReference(name.to_string()))
            }

            // Everything else can just be emitted exactly as is.
            other => {
               self.events.push(other.clone());
               Ok(None)
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
               Ok(None)
            } else {
               let event = Text(format!("[^{name}]").into());
               self.events.push(event);
               Ok(Some(format!(
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

#[derive(Debug)]
struct CodeBlock<'e, 's> {
   highlighting: Highlighting<'s>,
   syntax_set: Option<&'s SyntaxSet>,
   events: Vec<pulldown_cmark::Event<'e>>,
}

impl<'c, 's> CodeBlock<'c, 's> {
   /// Start highlighting a code block.
   fn start(kind: CodeBlockKind, syntax_set: &'s SyntaxSet) -> Self {
      match kind {
         CodeBlockKind::Fenced(name) => {
            let found = syntax_set.find_syntax_by_token(name.as_ref());
            let (html, highlighting) = if let Some(syntax) = found {
               (
                  pulldown_cmark::Event::Html(
                     format!("<pre><code class='{}'>", syntax.name).into(),
                  ),
                  Highlighting::KnownSyntax(ClassedHTMLGenerator::new_with_class_style(
                     syntax,
                     syntax_set,
                     ClassStyle::Spaced,
                  )),
               )
            } else {
               (
                  pulldown_cmark::Event::Html("<pre><code>".into()),
                  Highlighting::UnknownSyntax,
               )
            };

            CodeBlock {
               highlighting,
               syntax_set: Some(syntax_set),
               events: vec![html],
            }
         }
         CodeBlockKind::Indented => CodeBlock {
            highlighting: Highlighting::RequiresFirstLineParse,
            syntax_set: Some(syntax_set),
            events: vec![],
         },
      }
   }

   /// Produces events when:
   ///
   /// - starting a new code block
   /// - ending a code block
   ///
   /// Note that it does *not* emit events while highlighting a line. Instead, it stores
   /// internal state which produces a single fully-rendered HTML event when complete.
   fn highlight(&mut self, text: &CowStr<'c>) -> Result<(), Error> {
      let mut handle_unknown = || {
         self
            .events
            .push(pulldown_cmark::Event::Text(text.to_owned()))
      };

      let Some(syntax_set) = self.syntax_set else {
         handle_unknown();
         return Ok(());
      };

      match self.highlighting {
         Highlighting::RequiresFirstLineParse => {
            match syntax_set.find_syntax_by_first_line(text) {
               // If Syntect has a definition, emit processed HTML for the wrapper
               // and for the first line.
               Some(definition) => {
                  let mut generator = ClassedHTMLGenerator::new_with_class_style(
                     definition,
                     syntax_set,
                     ClassStyle::Spaced,
                  );
                  let event = pulldown_cmark::Event::Html(
                     format!(
                        "<pre lang='{name}'><code class='{name}'>",
                        name = definition.name
                     )
                     .into(),
                  );
                  generator
                     .parse_html_for_line_which_includes_newline(text)
                     .map_err(|e| Error::BadSyntaxLine { source: e })?;
                  self.highlighting = Highlighting::KnownSyntax(generator);
                  self.events.push(event);
                  Ok(())
               }

               // Otherwise, we treat this as a code block, but with no syntax
               // highlighting applied.
               None => {
                  self.highlighting = Highlighting::UnknownSyntax;
                  let event = pulldown_cmark::Event::Html(
                     (String::from("<pre><code>") + text).into(),
                  );
                  self.events.push(event);
                  Ok(())
               }
            }
         }

         // This is a little quirky: it hands off the text to the highlighter and
         // relies on correctly calling `highlighter.finalize()` when we reach the
         // end of the code block.
         // TODO: consider type-state-ifying that, too!
         Highlighting::KnownSyntax(ref mut generator) => {
            generator
               .parse_html_for_line_which_includes_newline(text.as_ref())
               .map_err(|e| Error::BadSyntaxLine { source: e })?;

            // ...and therefore produces no events!
            Ok(())
         }

         Highlighting::UnknownSyntax => {
            handle_unknown();
            Ok(())
         }
      }
   }

   /// Finish a code block, consuming the state and producing a single `Event::Html`
   /// as its result.
   fn end(mut self) -> Vec<pulldown_cmark::Event<'c>> {
      let end_html = match self.highlighting {
         Highlighting::KnownSyntax(generator) => generator.finalize() + "</code></pre>",
         _ => "</code></pre>".to_string(),
      };
      let end_event = pulldown_cmark::Event::Html(end_html.into());
      self.events.push(end_event);
      self.events
   }
}

enum Highlighting<'s> {
   RequiresFirstLineParse,
   UnknownSyntax,
   KnownSyntax(ClassedHTMLGenerator<'s>),
}

impl std::fmt::Debug for Highlighting<'_> {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      match self {
         Self::RequiresFirstLineParse => write!(f, "RequiresFirstLineParse"),
         Self::UnknownSyntax => write!(f, "UnknownSyntax"),
         Self::KnownSyntax(_) => write!(f, "KnownSyntax"),
      }
   }
}
