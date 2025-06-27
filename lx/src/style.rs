use std::path::Path;

use lightningcss::{
   bundler::{Bundler, FileProvider},
   printer::PrinterOptions,
   stylesheet::{MinifyOptions, ParserOptions},
};

pub fn convert(root: &Path, mode: Mode) -> Result<String, Error> {
   let fs = FileProvider::new();
   let mut bundler = Bundler::new(&fs, None, ParserOptions::default());
   let mut stylesheet = bundler
      .bundle(root)
      .map_err(|e| Error::Bundle(format!("{e:?}")))?;

   stylesheet
      .minify(MinifyOptions::default())
      .map_err(|e| Error::Minify(format!("{e:?}")))?;

   let mut print_options = PrinterOptions::default();
   print_options.minify = match mode {
      Mode::Dev => false,
      Mode::Prod => true,
   };

   let css = stylesheet
      .to_css(print_options)
      .map_err(|e| Error::EmitCss(format!("{e:?}")))?;

   Ok(css.code)
}

pub enum Mode {
   Dev,
   Prod,
}

// Because LightningCSS does not have good error handling!
#[derive(thiserror::Error, Debug)]
pub enum Error {
   #[error("Could not bundle CSS. Cause:\n{0}")]
   Bundle(String),

   #[error("Could not minify CSS. Cause:\n{0}")]
   Minify(String),

   #[error("Could not emit CSS. Cause:\n{0}")]
   EmitCss(String),

   #[error(transparent)]
   IO(#[from] std::io::Error),
}
