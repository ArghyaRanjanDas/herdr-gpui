# Rich chat feature gallery

These are direct captures of real native Herdr UI windows using synthetic fixture
conversations on an isolated Linux display. No private conversations, browser
sessions, or user desktop windows were captured. No UI content was retouched.

The feature commit is `436dc1ae`. The renderer binary reports `fdb22e2c`; tracked
Rust code, dependency manifests and bundled assets are identical at the feature
commit. Later commits changed Python test drivers. See [provenance.json](provenance.json)
for actual hashes, capture settings and environment.

## Gallery

- [hero.png](hero.png) — Native Document layout: readable prose, inline and display equations, a table containing math, and syntax-highlighted Python with copy controls.
- [widgets-results.png](widgets-results.png) — Native interactive card, scrolled to its input, comparison table, chart, and review/send controls; all values are illustrative.
- [diagram.png](diagram.png) — Native Mermaid flow diagram embedded in the same transcript.
- [user-light.png](user-light.png) — Light theme in a 650-pixel-wide native window: a rich user message with math, inline code, highlighted Python, and a table in iMessage layout.
- [questions.png](questions.png) — Recorded questions and answers rendered as a structured native card, including inline math, highlighted code, copy controls, and collapsed original source.
- [notices.png](notices.png) — Background notifications and reminders appear as compact, expandable native activity rows.
- [attachments.png](attachments.png) — A synthetic image attachment and a labelled local-document link, with environment metadata kept in compact notices.
- [original-input.png](original-input.png) — The rich answer remains above the original agent input; its divider was physically dragged to increase the input area in this isolated fixture.
- [widgets.png](widgets.png) — Complete native interactive card with choice, toggle, slider, input, metric, table, chart, and actions; no action was submitted.
- [selection.png](selection.png) — One deliberate native text selection spanning Unicode prose, code, and right-aligned table cells; Ctrl+C was checked against the exact expected text.

Historical question cards are read-only. Native card values are illustrative;
no prompt or permission response was submitted. These screenshots demonstrate
appearance, not live-agent behavior or hardware GPU performance. The existing
Windows file-link test failure and review findings remain separate open work.
