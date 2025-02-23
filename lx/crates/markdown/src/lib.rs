//! Implement Markdown transformation as a two-pass operation.
//!
//! 1. Handle two concerns:
//!     - metadata extraction (exposed to callers)
//!     - footnote extraction (managed wholly internally)
//! 2. Perform "transform" operations using the result of (1):
//!     - Rewrite the text of the document using a supplied templating language,
//!       if any (notably: applying this *only* to text nodes!).
//!     - Apply syntax highlighting.
//!     - Emit footnotes.

mod first_pass;
mod second_pass;

use std::collections::HashMap;
use std::fmt::Debug;

use lazy_static::lazy_static;
pub use pulldown_cmark::Options;
use pulldown_cmark::{CowStr, Event, MetadataBlockKind, Parser, Tag, TagEnd, html};
use serde::Deserialize;
use syntect::parsing::SyntaxSet;
use thiserror::Error;

use first_pass::FirstPass;
use second_pass::second_pass;

/// A footnote definition can have any arbitrary sequence of `pulldown_cmark::Event`s
/// in it, excepting other footnotes definitions. However, that scenario *should* be
/// forbidden by both `pulldown_cmark` itself *and* the event handling.
type FootnoteDefinitions<'e> = HashMap<CowStr<'e>, Vec<Event<'e>>>;

#[derive(Error, Debug)]
pub enum PrepareError {
   #[error("tried to use TOML for metadata")]
   UsedToml,

   #[error("failed to extract metadata section")]
   MetadataExtraction,

   #[error("could not prepare Markdown: {state} is invalid in {context}")]
   State { state: String, context: String },

   #[error("could not prepare Markdown content section")]
   Content {
      #[from]
      source: first_pass::Error,
   },
}

// The structure here lets the caller have access to the extracted metadata
// string (we do not need the parsed or rendered metadata) during the
// preparation pass, but only provides the `ToRender` type opaquely, so that it
// can only be used as the type-safe requirement for calling `render`.
pub struct Prepared<'e> {
   pub metadata_src: Option<String>,
   pub to_render: ToRender<'e>,
}

pub struct ToRender<'e> {
   first_pass_events: Vec<first_pass::Event<'e>>,
   footnote_definitions: FootnoteDefinitions<'e>,
}

#[derive(Error, Debug)]
pub enum Error {
   #[error(transparent)]
   Prepare {
      #[from]
      source: PrepareError,
   },
   #[error(transparent)]
   Render {
      #[from]
      source: RenderError,
   },
}

lazy_static! {
   static ref OPTIONS: Options = {
      let mut opts = Options::all();
      opts.set(Options::ENABLE_OLD_FOOTNOTES, false);
      opts.set(Options::ENABLE_FOOTNOTES, true);
      opts
   };
}

pub struct Markdown {
   syntax_set: SyntaxSet,
}

impl Markdown {
   pub fn new(syntax_set: Option<SyntaxSet>) -> Markdown {
      Markdown {
         syntax_set: syntax_set.unwrap_or_else(load_syntaxes), // TODO: pull from location?
      }
   }

   pub fn render(
      &self,
      src: &str,
      rewrite: impl Fn(&str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
   ) -> Result<(Option<String>, Rendered), Error> {
      let prepared = prepare(src)?;
      let rendered = self.emit(prepared.to_render, rewrite)?;

      // TODO: return named types instead of anonymous tuple values. Maybe just attach the
      // metadata to the `Rendered` type?
      Ok((prepared.metadata_src, rendered))
   }

   pub fn emit(
      &self,
      to_render: ToRender,
      rewrite: impl Fn(&str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
   ) -> Result<Rendered, RenderError> {
      let ToRender {
         first_pass_events,
         footnote_definitions,
      } = to_render;

      let events = second_pass(
         footnote_definitions,
         &self.syntax_set,
         first_pass_events,
         rewrite,
      )
      .map_err(RenderError::from)?;

      let mut content = String::new();
      html::push_html(&mut content, events);

      Ok(Rendered(content))
   }
}

pub fn prepare(src: &str) -> Result<Prepared<'_>, Error> {
   let parser = Parser::new_ext(src, *OPTIONS);

   let mut state = first_pass::FirstPass::new();

   // TODO: rewrite all these `bad_prepare_state` calls into actual specific errors from
   // the enum above!
   for event in parser {
      match event {
         Event::Start(Tag::MetadataBlock(kind)) => match state {
            FirstPass::Initial(initial) => {
               state = FirstPass::ExtractingMetadata(initial.parsing_metadata(kind))
            }
            _ => return bad_prepare_state(&event, &state),
         },

         Event::End(TagEnd::MetadataBlock(_)) => match state {
            FirstPass::ExtractedMetadata(metadata) => {
               state = FirstPass::Content(metadata.start_content())
            }
            _ => return bad_prepare_state(&event, &state),
         },

         Event::Text(ref text) => match state {
            FirstPass::Initial(initial) => {
               state = FirstPass::Content(initial.start_content());
            }

            FirstPass::ExtractingMetadata(parsing) => match parsing.kind() {
               MetadataBlockKind::YamlStyle => {
                  state = FirstPass::ExtractedMetadata(parsing.parsed(text.clone()));
               }

               MetadataBlockKind::PlusesStyle => {
                  return Err(Error::from(PrepareError::UsedToml));
               }
            },

            FirstPass::Content(ref mut content) => {
               content.handle(event).map_err(PrepareError::from)?
            }

            _ => return bad_prepare_state(&event, &state),
         },

         other => match state {
            FirstPass::Initial(initial) => {
               let mut content = initial.start_content();
               content.handle(other).map_err(PrepareError::from)?;
               state = FirstPass::Content(content);
            }

            FirstPass::Content(ref mut content) => {
               content.handle(other).map_err(PrepareError::from)?
            }

            _ => return bad_prepare_state(&other, &state),
         },
      }
   }

   let (metadata, first_pass_events, footnote_definitions) =
      state.finalize().map_err(PrepareError::from)?;

   Ok(Prepared {
      metadata_src: metadata.map(|m| m.to_string()),
      to_render: ToRender {
         first_pass_events,
         footnote_definitions,
      },
   })
}

#[derive(Error, Debug)]
#[error("could not render Markdown content")]
pub struct RenderError {
   #[from]
   source: second_pass::Error,
}

/// The result of successfully rendering content: HTML. It can be extracted via
/// the `.html()` method.
#[derive(Debug, Deserialize)]
#[repr(transparent)]
pub struct Rendered(String);

impl Rendered {
   #[inline(always)]
   pub fn html(&self) -> &str {
      self.0.as_str()
   }
}

fn bad_prepare_state<T>(state: &impl Debug, context: &impl Debug) -> Result<T, Error> {
   Err(Error::from(PrepareError::State {
      state: format!("{state:?}"),
      context: format!("{context:?}"),
   }))
}

// TODO: I think what I would *like* to do is have a slow path for dev and a
// fast path for prod, where the slow path just loads the `.sublime-syntax`
// from disk and compiles them, and the fast path uses a `build.rs` or similar
// to build a binary which can then be compiled straight into the target binary
// and loaded *extremely* fast as a result.
//
// The basic structure for a prod build would be something like:
//
// - `build.rs`:
//    - `syntect::SyntaxSet::load_from_folder(<path to templates>)`
//    - `syntect::dumps::dump_to_uncompressed_file(<well-known-path>)`
// - here (or, better, in a dedicated `syntax` module?):
//    - `include_bytes!(<well-known-path>)`
//    - `syntect::dumps::from_uncompressed_data()`
fn load_syntaxes() -> SyntaxSet {
   // let mut extra_syntaxes_dir = std::env::current_dir().map_err(|e| format!("{}", e))?;
   // extra_syntaxes_dir.push("syntaxes");

   // let syntax_builder = SyntaxSet::load_defaults_newlines().into_builder();
   // let mut syntax_builder = SyntaxSet::load_defaults_newlines().into_builder();
   // syntax_builder
   //     .add_from_folder(&extra_syntaxes_dir, false)
   //     .map_err(|e| format!("could not load {}: {}", &extra_syntaxes_dir.display(), e))?;

   // syntax_builder.build()
   SyntaxSet::load_defaults_newlines()
}
