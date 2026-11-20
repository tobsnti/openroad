#!/usr/bin/env python3
"""Tests for check_re_addresses.py - stdlib unittest, no cargo, no deps.

Run: `python3 scripts/test_check_re_addresses.py` (also run by `make ci`).

The address-shaped tokens are built from parts, so this file does not count
itself.
"""

import importlib.util
import unittest
from collections import Counter
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "check_re_addresses", Path(__file__).with_name("check_re_addresses.py")
)
cra = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cra)

NAME = "FUN" + "_00412345"
SHORT = "0x" + "412345"
LONG = "0x00" + "412345"


class TestFind(unittest.TestCase):
    def test_names_and_both_address_widths_are_found(self):
        line = f"/// {NAME} reads it (`{LONG}`, sro_client.exe@{SHORT})"
        self.assertEqual(cra.find(line), [(0, NAME), (1, LONG), (1, SHORT)])

    def test_masks_opcodes_offsets_and_separated_constants_are_not_addresses(self):
        self.assertEqual(cra.find("0x00ff0000 0x400000 0x3015 +0x2e5c 0x80_0000"), [])


class TestRatchet(unittest.TestCase):
    def test_a_count_must_equal_its_ceiling(self):
        ceiling = {"client": (1, 2)}
        self.assertEqual(cra.problems(Counter({("client", 0): 1, ("client", 1): 2}), ceiling), [])
        self.assertEqual(
            cra.problems(Counter({("client", 0): 2, ("client", 1): 2}), ceiling),
            [("client", 0, 2, 1)],
        )
        self.assertEqual(
            cra.problems(Counter({("client", 0): 1, ("client", 1): 1}), ceiling),
            [("client", 1, 1, 2)],
        )

    def test_an_unlisted_directory_has_no_room(self):
        self.assertEqual(cra.problems(Counter({("server", 1): 1}), {}), [("server", 1, 1, 0)])


if __name__ == "__main__":
    unittest.main()
