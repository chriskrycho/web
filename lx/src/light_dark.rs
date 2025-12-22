//! Transform Arborium theme CSS to use `light-dark()` for automatic theme switching.
//!
//! This module takes two `arborium::theme::Theme` instances (one light, one dark),
//! extracts their CSS, parses it with `lightningcss`, and emits tag rules using CSS
//! `light-dark()` for automatic theme switching based on user preference.

use std::collections::BTreeMap;

use lightningcss::{
   declaration::DeclarationBlock,
   printer::PrinterOptions,
   properties::Property,
   rules::CssRule,
   selector::{Component, SelectorList},
   stylesheet::{ParserOptions, StyleSheet},
   traits::ToCss,
   values::color::CssColor,
};

pub struct Config<'t, 's> {
   pub light: &'t arborium::theme::Theme,
   pub dark: &'t arborium::theme::Theme,
   pub selector_prefix: &'s str,
}

impl Config<'_, '_> {
   /// Transforms two Arborium themes into a single CSS output using `light-dark()`.
   ///
   /// # Returns
   ///
   /// A string containing CSS rules with `light-dark()` color values
   pub fn to_css(&self) -> Result<String, Error> {
      let mut tag_colors = BTreeMap::new();

      extract_colors(
         Theme::Light,
         &self.light.to_css(self.selector_prefix),
         &mut tag_colors,
      )?;

      extract_colors(
         Theme::Dark,
         &self.dark.to_css(self.selector_prefix),
         &mut tag_colors,
      )?;

      let colors = print(tag_colors);

      Ok(colors)
   }
}

fn print(tag_colors: BTreeMap<String, Color>) -> String {
   tag_colors
      .into_iter()
      .fold(String::new(), |mut colors, (tag, color)| {
         colors.push_str(&tag);
         colors.push_str(" { color: ");
         match color {
            Color::Both { light, dark } => {
               colors.push_str("light-dark(");
               colors.push_str(&css_color_for(&light));
               colors.push_str(", ");
               colors.push_str(&css_color_for(&dark));
               colors.push(')');
            }
            Color::Light(color) | Color::Dark(color) => {
               colors.push_str(&css_color_for(&color));
            }
         }
         colors.push_str("; }\n");

         colors
      })
}

/// Extracts color values for Arborium custom element tags from CSS.
///
/// Returns a HashMap mapping tag names (e.g., "a-k", "a-f") to their color values.
fn extract_colors(
   theme: Theme,
   css: &str,
   colors: &mut BTreeMap<String, Color>,
) -> Result<(), Error> {
   // Arborium's to_css("") generates CSS like " { ... }" which is invalid CSS. In that
   // case, wrap it in a dummy selector.
   let css = css.trim();
   let cleaned_css = if css.starts_with('{') && css.ends_with('}') {
      format!(":dummy {css}")
   } else {
      css.to_string()
   };

   let stylesheet = StyleSheet::parse(&cleaned_css, ParserOptions::default())
      .map_err(|e| Error::ParseCss(format!("{e:?}")))?;

   rules_to_colors(&stylesheet.rules.0, theme, colors);

   Ok(())
}

/// Recursively extracts colors from CSS rules, handling nested rules.
///
/// `colors` is passed by reference so it can be reused across the recursion.
fn rules_to_colors(
   rules: &[CssRule],
   theme: Theme,
   colors: &mut BTreeMap<String, Color>,
) {
   for rule in rules {
      match rule {
         CssRule::Style(style_rule) => {
            // Check if this is an arborium tag selector (a-k, a-f, etc.)
            if let Some(tag_name) = tag_in(&style_rule.selectors)
               && let Some(color) = color_from(&style_rule.declarations)
            {
               colors
                  .entry(tag_name)
                  .and_modify(|existing| match (&*existing, &theme) {
                     (Color::Light(light), Theme::Dark) => {
                        *existing = Color::Both {
                           light: light.clone(),
                           dark: color.clone(),
                        };
                     }
                     (Color::Dark(dark), Theme::Light) => {
                        *existing = Color::Both {
                           light: color.clone(),
                           dark: dark.clone(),
                        };
                     }
                     _ => unreachable!("each tag appears at most once per theme"),
                  })
                  .or_insert_with(|| match theme {
                     Theme::Light => Color::Light(color),
                     Theme::Dark => Color::Dark(color),
                  });
            }

            // Handle nested rules (CSS nesting)
            rules_to_colors(&style_rule.rules.0, theme, colors);
         }

         CssRule::Nesting(nesting_rule) => {
            rules_to_colors(&nesting_rule.style.rules.0, theme, colors)
         }

         // Ignore other rule types (media queries, keyframes, etc.)
         _ => {}
      }
   }
}

/// Checks if the selector list contains a simple arborium tag selector.
///
/// Returns the tag name if it's a single element selector matching the pattern `a-*`.
/// Handles both plain selectors (e.g., "a-k") and nested selectors (e.g., "& a-k").
fn tag_in(selectors: &SelectorList) -> Option<String> {
   // We're looking for simple selectors like "a-k", "a-f", "& a-k", etc.
   if selectors.0.len() != 1 {
      return None;
   }

   // Anything past the first
   let components: Vec<_> = selectors.0[0].iter_raw_match_order().take(3).collect();

   let local_name = match components.as_slice() {
      // "a-<tag>" pattern
      [Component::LocalName(local_name)] => Some(local_name),
      // "& a-<tag>" pattern (tag + combinator + nesting)
      [
         Component::LocalName(local_name),
         Component::Combinator(_),
         Component::Nesting,
      ] => Some(local_name),
      _ => None,
   }?;

   let name = local_name.name.as_ref();
   if name.starts_with("a-") {
      Some(name.to_string())
   } else {
      None
   }
}

/// Extracts the color value from a declaration block.
fn color_from(declarations: &DeclarationBlock) -> Option<CssColor> {
   for decl in &declarations.declarations {
      if let Property::Color(color) = decl {
         return Some(color.clone());
      }
   }
   None
}

/// Represents the color(s) available for a tag across light and dark themes.
enum Color {
   Both { light: CssColor, dark: CssColor },
   Light(CssColor),
   Dark(CssColor),
}

#[derive(Copy, Clone)]
enum Theme {
   Light,
   Dark,
}

/// Converts a CssColor to its CSS string representation.
fn css_color_for(color: &CssColor) -> String {
   color
      .to_css_string(PrinterOptions::default())
      .expect("All colors from Arborium themes should be actual colors")
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
   #[error("Could not parse CSS: {0}")]
   ParseCss(String),
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn test_to_light_dark_css_with_builtin_themes() {
      let result = Config {
         light: &arborium::theme::builtin::alabaster(),
         dark: &arborium::theme::builtin::ayu_dark(),
         selector_prefix: "",
      }
      .to_css()
      .unwrap();

      // Verify the output contains expected structure
      assert!(
         result.contains("light-dark("),
         "Output should contain light-dark() function"
      );

      // Verify it contains some common arborium tags
      assert!(result.contains("a-k"), "Should contain a-k tag");
      assert!(result.contains("a-f"), "Should contain a-f tag");
      assert!(result.contains("a-s"), "Should contain a-s tag");

      // Verify the format is correct (tag { color: ... })
      assert!(
         result.contains("{ color: "),
         "Should have color property declarations"
      );
   }

   #[test]
   fn test_extract_colors() {
      let css = r#"
         a-k { color: #000080; }
         a-f { color: #795e26; }
         & a-em { font-style: italic; color: hsl(150deg 30% 60%); }
      "#;

      let mut colors = BTreeMap::new();
      extract_colors(Theme::Light, css, &mut colors).unwrap();

      assert!(colors.contains_key("a-k"));
      assert!(colors.contains_key("a-f"));
      assert!(colors.contains_key("a-em"));
   }

   #[test]
   fn test_print_both_present() {
      let mut colors = BTreeMap::new();
      colors.insert(
         "a-k".to_string(),
         Color::Both {
            light: CssColor::RGBA(lightningcss::values::color::RGBA {
               red: 0,
               green: 0,
               blue: 129,
               alpha: 255,
            }),
            dark: CssColor::RGBA(lightningcss::values::color::RGBA {
               red: 122,
               green: 162,
               blue: 247,
               alpha: 255,
            }),
         },
      );

      let result = print(colors);

      assert_eq!(result, "a-k { color: light-dark(#000081, #7aa2f7); }\n");
   }

   #[test]
   fn test_print_only_light() {
      let mut colors = BTreeMap::new();
      colors.insert(
         "a-k".to_string(),
         Color::Light(CssColor::RGBA(lightningcss::values::color::RGBA {
            red: 0,
            green: 0,
            blue: 129,
            alpha: 255,
         })),
      );

      let result = print(colors);

      assert_eq!(result, "a-k { color: #000081; }\n");
   }
}
