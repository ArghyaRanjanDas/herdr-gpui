# Native follow-up review captures

All images are direct native-window captures of isolated synthetic fixtures. No
real agent transcript or connected-provider end-to-end session is shown.

Baseline: `b9796152fc998906ec61edf979d42bccf44ed7a6`.
After: `0feeb4a581ddffbd83078e82929e6b97dfbfa72d`.

The activity pairs use the same fixture in dark/light narrow windows. The tool
showcase deliberately combines different tool states while the session is Ready;
the separate reported-activity image shows the Running strip. Loading uses a wide
window so the synthetic terminal grid fits.

[Provenance](provenance.json) records exact binary/driver/source hashes and image
dimensions. Images have not been cropped, generated, or retouched.

- [before-activity-dark.png](before-activity-dark.png): Baseline synthetic notice/prose fixture, dark narrow.
- [before-activity-light.png](before-activity-light.png): Baseline synthetic notice/prose fixture, light narrow.
- [after-activity-dark.png](after-activity-dark.png): The same synthetic notice/prose fixture with terminal accents and reader status, dark narrow.
- [after-activity-light.png](after-activity-light.png): The same synthetic notice/prose fixture with terminal accents and reader status, light narrow.
- [diff-expanded-dark.png](diff-expanded-dark.png): Synthetic tool-card showcase: expanded unified diff with syntax, red/green rows and Unicode. Tool statuses are deliberately mixed; this is not a connected provider run.
- [diff-expanded-light.png](diff-expanded-light.png): The same expanded synthetic tool-card diff in the light theme.
- [disclosure-collapsed-dark.png](disclosure-collapsed-dark.png): The synthetic tool cards after a real disclosure click hides source panels and copy controls.
- [jump-to-latest-dark.png](jump-to-latest-dark.png): The measured Jump to latest button in a scrolled synthetic transcript; native checks also click it and verify return to the tail.
- [reported-activity-dark.png](reported-activity-dark.png): Synthetic reported Running activity with elapsed time and explicitly supplied token metrics. Static image; streaming behavior is covered separately by protocol tests.
- [loading-input-light-wide.png](loading-input-light-wide.png): Loading shell with the original input available below. Synthetic fixed terminal grid, wide viewport.
