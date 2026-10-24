use super::*;

mod imessage;
mod typography;

#[core::prelude::v1::test]
fn code_tokens_and_comments_have_text_contrast_on_dark_and_light_surfaces() {
    for name in ["Default", "Catppuccin Latte"] {
        let Some(theme) = Theme::builtin(name) else {
            panic!("missing {name}")
        };
        for token in [
            Token::Comment,
            Token::String,
            Token::Number,
            Token::Constant,
            Token::Keyword,
            Token::Type,
            Token::Function,
        ] {
            assert!(
                crate::contrast::ratio(token_color(&theme, token), theme.surface) >= 4.5,
                "{name} {token:?}"
            );
        }
    }
}
