use super::*;

/// Symbols iOS draws as color emoji, or that read as nothing at a glance.
const SYMBOLS: [&str; 5] = ["⚙", "⊕", "⌄", "◉", "▭"];

#[test]
fn home_draws_its_controls_as_line_icons() {
    let layout = Layout {
        connected: true,
        boot: "b".into(),
        workspaces: vec![workspace(vec![row("p1")])],
    };
    let summaries = HashMap::new();
    let lists = Lists {
        computer: "MacBook Pro",
        layout: &layout,
        summaries: &summaries,
        now_ms: 0,
    };
    for page in [home(&lists).into_string(), board(&lists).into_string()] {
        for symbol in SYMBOLS {
            assert!(!page.contains(symbol), "{symbol} in {page}");
        }
        assert!(page.contains(r#"<svg class="icon""#), "{page}");
        assert!(page.contains(r#"aria-label="Settings""#), "{page}");
    }
}
