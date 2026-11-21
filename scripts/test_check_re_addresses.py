#!/usr/bin/env python3
"""Tests for check_re_addresses.py - stdlib unittest, no cargo, no deps.

Run: `python3 scripts/test_check_re_addresses.py` (also run by `make ci`).

The address-shaped tokens are built from parts, so this file does not count
itself, and the source-file name is made up.
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
BARE = "00" + "412345"
SOURCE = "CExampleWindow" + ".cpp"


class TestFind(unittest.TestCase):
    def test_names_and_both_address_widths_are_found(self):
        line = f"/// {NAME} reads it (`{LONG}`, sro_client.exe@{SHORT})"
        self.assertEqual(cra.find(line), [(0, NAME), (1, LONG), (1, SHORT)])

    def test_an_address_without_its_0x_is_found_but_a_decimal_fraction_is_not(self):
        line = f"/// builder `{BARE}`, `{BARE}..{BARE}`; vec3(0.{BARE}, 0x{BARE})"
        self.assertEqual(cra.find(line), [(1, BARE)] * 3 + [(1, "0x" + BARE)])

    def test_masks_opcodes_offsets_and_separated_constants_are_not_addresses(self):
        self.assertEqual(cra.find("0x00ff0000 0x400000 0x3015 +0x2e5c 0x80_0000"), [])

    def test_a_listed_value_is_not_an_address_in_either_case(self):
        listed = next(iter(cra.NOT_ADDRESSES))
        self.assertEqual(cra.find(f"{listed}, {listed[:2]}{listed[2:].upper()}"), [])

    def test_a_source_file_name_is_found_unless_it_is_a_listed_foreign_one(self):
        foreign = next(iter(cra.FOREIGN_SOURCES))
        line = f"// {SOURCE}:42, {foreign}, tpl.h and self.h are field reads"
        self.assertEqual(cra.find(line), [(2, SOURCE)])


class TestRatchet(unittest.TestCase):
    def test_a_count_must_equal_its_ceiling(self):
        ceiling = {"client": (1, 2, 1)}
        exact = Counter({("client", 0): 1, ("client", 1): 2, ("client", 2): 1})
        self.assertEqual(cra.problems(exact, ceiling), [])
        self.assertEqual(
            cra.problems(exact + Counter({("client", 0): 1}), ceiling),
            [("client", 0, 2, 1)],
        )
        self.assertEqual(
            cra.problems(exact - Counter({("client", 1): 1}), ceiling),
            [("client", 1, 1, 2)],
        )
        self.assertEqual(
            cra.problems(exact + Counter({("client", 2): 1}), ceiling),
            [("client", 2, 2, 1)],
        )

    def test_an_unlisted_directory_has_no_room(self):
        self.assertEqual(cra.problems(Counter({("server", 2): 1}), {}), [("server", 2, 1, 0)])


if __name__ == "__main__":
    unittest.main()
