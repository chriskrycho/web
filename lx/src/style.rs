use camino::Utf8Path;
use lightningcss::{
   bundler::{Bundler, FileProvider},
   printer::PrinterOptions,
   stylesheet::{MinifyOptions, ParserOptions},
};

pub fn convert(root: &Utf8Path, mode: OutputMode) -> Result<String, Error> {
   let fs = FileProvider::new();
   let mut bundler = Bundler::new(&fs, None, ParserOptions::default());
   let mut stylesheet = bundler
      .bundle(root.as_std_path())
      .map_err(|e| Error::Bundle(format!("{e:?}")))?;

   stylesheet
      .minify(MinifyOptions::default())
      .map_err(|e| Error::Minify(format!("{e:?}")))?;

   let print_options = PrinterOptions {
      minify: match mode {
         OutputMode::Dev => false,
         OutputMode::Prod => true,
      },
      ..Default::default()
   };

   let css = stylesheet
      .to_css(print_options)
      .map_err(|e| Error::EmitCss(format!("{e:?}")))?;

   Ok(css.code)
}

pub enum OutputMode {
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
