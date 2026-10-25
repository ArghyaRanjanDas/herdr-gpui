"""Verify the real input divider through the native fixture's private pointer."""

import json
import time


def capture_resize(directory, app, record, drag, motion, double_click, capture):
    def wait_for(predicate):
        deadline = time.monotonic() + 4
        state = None
        while time.monotonic() < deadline:
            state = record()
            if state and predicate(state):
                return state
            if app.poll() is not None:
                raise RuntimeError("Input resize fixture exited during pointer checks")
            time.sleep(0.05)
        raise TimeoutError(f"Native input divider did not reach expected state: {state!r}")

    initial = wait_for(lambda state: state["mode"] == "Auto")
    start = initial["divider"]
    drag(start, [start[0], start[1] - 110])
    enlarged = wait_for(lambda state: state["mode"].startswith("Rows(")
                        and state["input_rows"] > initial["input_rows"]
                        and state["conversation_rows"] < initial["conversation_rows"])
    capture(directory / "input-enlarged.png", enlarged["viewport"])

    start = enlarged["divider"]
    drag(start, [start[0], start[1] + 55])
    reduced = wait_for(lambda state: state["mode"].startswith("Rows(")
                       and initial["input_rows"] <= state["input_rows"] < enlarged["input_rows"]
                       and state["conversation_rows"] > enlarged["conversation_rows"])
    capture(directory / "input-reduced.png", reduced["viewport"])

    # Moving after release must not keep resizing the pane.
    point = reduced["divider"]
    destination = [point[0] + 30, point[1] - 80]
    motion(destination)
    # Require the native window to observe this exact motion. Re-reading a
    # stale change-only geometry record could otherwise falsely pass.
    released = wait_for(lambda state: all(abs(state["pointer"][i] - destination[i]) <= 1
                                         for i in range(2)))
    if any(released[key] != reduced[key]
           for key in ("mode", "input_rows", "conversation_rows", "divider", "viewport")):
        raise AssertionError(f"Input divider continued dragging after release: {released!r}")

    double_click(reduced["divider"])
    restored = wait_for(lambda state: state["mode"] == "Auto"
                        and state["input_rows"] == initial["input_rows"])
    capture(directory / "input-auto.png", restored["viewport"])
    results = {
        "status": "passed",
        "checks": [
            {"case": "drag-up", "state": enlarged},
            {"case": "drag-down", "state": reduced},
            {"case": "released-pointer", "state": released},
            {"case": "double-click-auto", "state": restored},
        ],
    }
    (directory / "input-resize.json").write_text(json.dumps(results, indent=2))
