use super::*;
use core::prelude::v1::test;

#[test]
fn activity_colors_follow_semantics_and_keep_status_words() {
    assert_eq!(
        tool_state(ToolStatus::Pending, ToolKind::Execute),
        ("waiting", 3)
    );
    assert_eq!(
        tool_state(ToolStatus::InProgress, ToolKind::Execute),
        ("running", 4)
    );
    assert_eq!(
        tool_state(ToolStatus::InProgress, ToolKind::Think),
        ("thinking", 5)
    );
    assert_eq!(
        tool_state(ToolStatus::Completed, ToolKind::Think),
        ("done", 2)
    );
    assert_eq!(
        tool_state(ToolStatus::Failed, ToolKind::Execute),
        ("failed", 1)
    );
}

#[test]
fn semantic_status_text_has_contrast_in_light_and_dark_themes() {
    for name in ["Default", "Catppuccin Latte"] {
        let Some(theme) = crate::config::Theme::builtin(name) else {
            panic!("missing {name}")
        };
        for palette in [1, 2, 3, 4, 5] {
            let ink = crate::chat::view::text_ink(theme.palette[palette], theme.background);
            assert!(
                crate::contrast::ratio(ink, theme.background) >= 4.5,
                "{name} {palette}"
            );
        }
    }
}
