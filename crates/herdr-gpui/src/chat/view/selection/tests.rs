use super::*;
use core::prelude::v1::test;

#[test]
fn selecting_even_one_byte_blocks_link_activation_until_the_next_plain_click() {
    let mut transcript = herdr_chat_protocol::Transcript::new("test");
    transcript.entries.push(herdr_chat_protocol::Entry {
        id: EntryId(1),
        kind: herdr_chat_protocol::EntryKind::Agent {
            text: "[label](https://example.org)".into(),
        },
    });
    let mut prepared = crate::chat::prepared::Prepared::default();
    prepared.update(&transcript, &[(EntryId(1), 1)]);
    let geometry = Geometry::default();
    let selection = Selection::default();
    let scope = Scope::new(
        "test",
        EntryId(1),
        0,
        prepared.text(),
        &selection,
        &geometry,
    );
    let Some(fragment) = scope.fragment(0, 0, 0) else {
        panic!("link text missing")
    };
    let caret = Caret {
        key: fragment.key,
        offset: 0,
    };
    let origin = point(px(10.), px(10.));
    geometry.press(caret, origin, 1, false);
    geometry.moved(origin + point(px(2.), px(0.)));
    assert!(
        fragment.allows_link(),
        "two pixels alone are below the drag threshold"
    );
    geometry.dragged(Caret { offset: 1, ..caret });
    geometry.release();
    assert!(
        !fragment.allows_link(),
        "a selected character must never open the link"
    );
    geometry.press(caret, origin, 2, false);
    assert!(!fragment.allows_link(), "word selection must not navigate");
    geometry.press(caret, origin, 3, false);
    assert!(
        !fragment.allows_link(),
        "paragraph selection must not navigate"
    );
    geometry.press(caret, origin, 1, true);
    assert!(
        !fragment.allows_link(),
        "extending selection must not navigate"
    );
    geometry.press(caret, origin, 1, false);
    assert!(
        fragment.allows_link(),
        "a fresh plain click still opens a link"
    );
    geometry.clear();
    assert!(
        !fragment.allows_link(),
        "replaced geometry cannot activate a stale link"
    );
}

#[test]
fn selection_splits_styles_without_losing_emphasis_or_link_decoration() {
    let theme = Theme::default();
    let bold = HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    let underline = HighlightStyle {
        underline: Some(UnderlineStyle::default()),
        ..Default::default()
    };
    let marks = highlights(
        "éabcdef",
        vec![(0..4, bold), (4..8, underline)],
        Some(2..6),
        &theme,
    );
    assert_eq!(
        marks
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>(),
        vec![0..2, 2..4, 4..6, 6..8]
    );
    assert_eq!(marks[1].1.font_weight, bold.font_weight);
    assert_eq!(marks[2].1.underline, underline.underline);
    assert!(marks[0].1.background_color.is_none());
    assert!(marks[1].1.background_color.is_some());
    assert!(marks[2].1.background_color.is_some());
    assert!(marks[3].1.background_color.is_none());
}
