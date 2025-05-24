use std::io::{Read, Write};

pub fn convert(
   mut input: Box<dyn Read>,
   output: &mut Box<dyn Write>,
) -> Result<(), Error> {
   let mut src = String::new();
   input.read_to_string(&mut src)?;

   let css = grass::from_string(src, &Default::default()).map_err(|e| Error::from(*e))?;

   output.write_all(css.as_bytes())?;
   Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
   #[error(transparent)]
   Compile(#[from] grass::Error),

   #[error(transparent)]
   IO(#[from] std::io::Error),
}
