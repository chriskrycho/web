//! Dev-only live-reload configuration.

use crate::build::Mode;

pub(crate) const CLIENT: &str = include_str!("../assets/live-reload.js");

const SCRIPT_TAG: &str = r#"<script src="/live-reload.js" defer></script>"#;
const BODY_CLOSE: &[u8] = b"</body>";

/// Inject live reload script *iff* in [`Mode::Serve`].
pub(crate) fn inject_for(mode: Mode, buf: &mut String) -> Result<(), Error> {
   match mode {
      Mode::Build => Ok(()),
      Mode::Serve => inject(buf),
   }
}

fn inject(buf: &mut String) -> Result<(), Error> {
   let insert_at = buf
      .as_bytes()
      .windows(BODY_CLOSE.len())
      .rposition(|window| window.eq_ignore_ascii_case(BODY_CLOSE))
      .ok_or(Error)?;

   buf.insert_str(insert_at, SCRIPT_TAG);
   Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("rendered page has no closing </body> tag, so live reload cannot be injected")]
pub(crate) struct Error;

#[cfg(test)]
mod tests {
   use super::*;

   const EXPECTED_SCRIPT: &str = "<script src=\"/live-reload.js\" defer></script>";

   #[test]
   fn inserts_before_final_body() {
      let mut input = String::from("<html><head></head><body></body></html>");
      let result = inject(&mut input);
      assert!(result.is_ok());

      let expected = String::from(
         "<html><head></head><body><script src=\"/live-reload.js\" defer></script></body></html>",
      );

      assert_eq!(input, expected);
   }

   #[test]
   fn serve_mode_injects() {
      let mut input = String::from("<body></body>");

      inject_for(Mode::Serve, &mut input).expect("serve-mode injection succeeds");

      assert_eq!(input, format!("<body>{EXPECTED_SCRIPT}</body>"));
   }

   #[test]
   fn build_mode_does_not_inject() {
      let original = String::from("<html><body></body></html>");
      let mut input = original.clone();

      inject_for(Mode::Build, &mut input).expect("build-mode injection is a no-op");

      assert_eq!(input, original);
   }

   #[test]
   fn inserts_before_the_last_body_close_tag() {
      let mut input = String::from(
         "<body><script>const closing = \"</body>\";</script></body></html>",
      );

      inject(&mut input).expect("injects into a document with multiple closing tags");

      assert_eq!(
         input,
         format!(
            "<body><script>const closing = \"</body>\";</script>{EXPECTED_SCRIPT}</body></html>"
         ),
      );
   }

   #[test]
   fn preserves_content_after_the_body() {
      let mut input = String::from("<body></body>\n<!-- trailing -->\n");

      inject(&mut input).expect("injects");

      assert_eq!(
         input,
         format!("<body>{EXPECTED_SCRIPT}</body>\n<!-- trailing -->\n"),
      );
   }

   #[test]
   fn handles_multibyte_content() {
      let mut input = String::from("<body>Coöperation FTW: 🎉 </body>");

      inject(&mut input).expect("injects at a character boundary");

      assert_eq!(
         input,
         format!("<body>Coöperation FTW: 🎉 {EXPECTED_SCRIPT}</body>")
      );
   }

   #[test]
   fn matches_closing_body_case_insensitively() {
      let mut input = String::from("<BODY></BODY>");

      inject(&mut input).expect("HTML tags are case-insensitive");

      assert_eq!(input, format!("<BODY>{EXPECTED_SCRIPT}</BODY>"));
   }

   #[test]
   fn leaves_buffer_unchanged_without_body() {
      let original = String::from("<html><head></head></html>");
      let mut input = original.clone();

      let error =
         inject(&mut input).expect_err("a page without </body> cannot be injected");

      assert!(matches!(error, Error));
      assert_eq!(input, original);
   }

   #[test]
   fn client_derives_the_endpoint_from_the_page() {
      assert!(CLIENT.contains("new URL('/live-reload', location.href)"));
      assert!(CLIENT.contains("location.protocol === 'https:' ? 'wss:' : 'ws:'"));
      assert!(!CLIENT.contains("24747"));
   }
}
