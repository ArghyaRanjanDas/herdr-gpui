"""Native probes must never parse a log record still being written."""

import json
from pathlib import Path
import tempfile
import unittest

from rich_chat_native_input import last_record


class NativeRecordTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.log = Path(directory.name) / "native.log"
        self.prefix = "RICH CHAT INPUT SIZE "

    def test_partial_record_is_ignored_until_its_newline_arrives(self):
        complete = self.prefix + json.dumps({"rows": 8}) + "\n"
        pending = self.prefix + json.dumps({"rows": 13})
        self.log.write_text(complete + pending[:-3])
        self.assertEqual(last_record(self.log, self.prefix), {"rows": 8})
        self.log.write_text(complete + pending)
        self.assertEqual(last_record(self.log, self.prefix), {"rows": 8})
        with self.log.open("a") as output:
            output.write("\n")
        self.assertEqual(last_record(self.log, self.prefix), {"rows": 13})

    def test_no_completed_matching_record_returns_none(self):
        self.log.write_bytes(b"unrelated output\n" + self.prefix.encode() + b'{"text":"\xc3')
        self.assertIsNone(last_record(self.log, self.prefix))

    def test_malformed_complete_record_is_not_hidden(self):
        self.log.write_text(self.prefix + "{broken}\n")
        with self.assertRaises(json.JSONDecodeError):
            last_record(self.log, self.prefix)
