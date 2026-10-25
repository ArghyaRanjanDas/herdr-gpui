# Rich chat review captures

These are real native fixture windows with synthetic conversations on an isolated
Linux Xvfb display. They are not mockups or captures of personal conversations.
Software rendering was resource-limited; these images are not performance evidence.

- [Earlier selection behavior](selection-before.png) is a development snapshot before the aligned-table selection fix, not an upstream release baseline.
- [Full-width selection](selection-dark-fullwidth.png) and [narrow light layout](selection-light-narrow.png) show paragraph, code and aligned table selection.
- [Automatic input height](input-auto.png), [dragged up](input-enlarged.png), [dragged down](input-reduced.png) and [narrow light input](input-light-narrow.png) show the original input divider.
- [Historical question/answer cards](questions.png) are read-only; live CLI questions still use the original editor.
- [Native widgets](widgets.png) show synthetic analysis controls.

Six native cases passed eighteen distinct real pointer checks. The renderer binary
was built at `fdb22e2c`; its Rust code, dependency manifests and bundled assets
are unchanged at `436dc1ae`. It still reports its actual `fdb22e2c` build revision.
Later commits only repaired the Python capture drivers. Per-image hashes and
actual driver/binary commits are recorded in [provenance.json](provenance.json).
Native Windows/macOS and full live-agent interaction checks remain separate QA.
