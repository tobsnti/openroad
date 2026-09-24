#!/usr/bin/env python3
"""Fixture tests for check_no_private.py — stdlib unittest, no cargo, no deps.

Run: `python3 scripts/test_check_no_private.py` (also run by `make ci`).

Two rules are pinned here because both were changed after the gate produced a
wrong answer in each direction:

* the address rule fired on four-level item TIDs (`3.3.10.1`), a false alarm
  that made the gate unusable; the exemption is now two conditions wide, and
  the tests that matter are the ones proving a real address still trips it,
* the account rule missed a name glued to a prefix (`gf_<name>`), a silent
  hole, which is the worse failure of the two.

Address fixtures use 203.0.113.7 (TEST-NET-3, RFC 5737 -- reserved for
documentation, routable nowhere), so proving the rule fires does not require
writing down a real endpoint.
"""

import contextlib
import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "check_no_private", Path(__file__).with_name("check_no_private.py")
)
gate = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gate)


def scan(line):
    """Names of the patterns that fire on one added line of a diff."""
    return [name for name, _ in gate.scan_line(line)]


class ItemTidExemption(unittest.TestCase):
    def test_a_tid_named_as_such_in_a_code_span_is_not_an_address(self):
        self.assertEqual(scan("/// A Lucky Powder (TID `3.3.10.2`)."), [])

    def test_a_real_address_still_trips_the_gate(self):
        self.assertIn("gateway address", scan('    Some("203.0.113.7:15779".into())'))

    def test_backticks_alone_do_not_exempt_an_address(self):
        # Half the exemption is not the exemption: without the word TID a code
        # span is still an address, which is how a pasted endpoint gets caught.
        self.assertIn("gateway address", scan("/// the host `203.0.113.7`"))

    def test_the_word_tid_alone_does_not_exempt_an_address(self):
        self.assertIn("gateway address", scan("/// TID table lives on 203.0.113.7"))

    def test_an_address_later_on_an_exempted_line_is_still_found(self):
        # The reason the scan uses finditer: an exempted TID at the start of the
        # line must not hide what follows it.
        line = "/// TID `3.3.10.1` was measured on 203.0.113.7"
        self.assertIn("gateway address", scan(line))

    def test_loopback_stays_allowed(self):
        self.assertEqual(scan('Some("127.0.0.1:15779".into())'), [])


class AccountRule(unittest.TestCase):
    def test_a_bare_account_name_trips_the_gate(self):
        self.assertIn("test account", scan("/// `packet_dump/admin1/0x3013.log`"))

    def test_a_prefixed_account_name_also_trips_the_gate(self):
        self.assertIn("test account", scan("Dumps: `packet_dump/xx_admin1/`"))

    def test_the_anonymised_citation_passes(self):
        self.assertEqual(scan("/// `packet_dump/<account 1>/0x3013.log` 2026-08-20T14:32:55.583Z"), [])


class HostnameRule(unittest.TestCase):
    """A named endpoint is the same leak as a numeric one.

    Fixtures use the RFC 2606 `.invalid` TLD, which resolves nowhere, so
    proving the rule fires does not require writing a live host down.
    """

    def test_a_service_prefixed_host_trips_the_gate(self):
        self.assertIn("gateway host", scan('gateways: vec!["filter.some-shard.invalid".into()],'))

    def test_a_host_with_a_port_trips_the_gate(self):
        self.assertIn("host:port endpoint", scan('Some("some-shard.invalid:4001")'))

    def test_the_documentation_placeholder_passes(self):
        self.assertEqual(scan('gateways: vec!["filter.example.com".into()],'), [])

    def test_the_documentation_placeholder_with_a_port_passes(self):
        self.assertEqual(scan("`filter.example.com:4001` for this build"), [])

    def test_localhost_with_a_port_stays_allowed(self):
        self.assertEqual(scan('Some("localhost:15779")'), [])

    def test_an_ordinary_domain_in_prose_is_not_an_endpoint(self):
        # The gate has to stay usable: the licence table and RESOURCES.md are
        # full of plain domains, and none of them is a server endpoint.
        self.assertEqual(scan("see https://github.com/Veykril/pk2 and crates.io"), [])

    def test_a_rust_method_chain_is_not_a_host(self):
        # `shard.name.clone()` reads exactly like a service-prefixed FQDN.
        # A match followed by `(` is code, not an endpoint.
        self.assertEqual(scan(".map(|shard| shard.name.clone())"), [])
        # ...and the real thing still trips, even next to a call.
        self.assertTrue(scan("gateway.sro.invalid resolved"))


class NewlyClosedHoles(unittest.TestCase):
    """Three rules were provably blind, plus one false alarm (2026-09-11)."""

    def test_a_two_digit_account_number_trips_the_gate(self):
        # `admin\d\b` reads `admin10` as `admin1` + no boundary and says nothing.
        self.assertIn("test account", scan("/// `packet_dump/admin10/0x3013.log`"))

    def test_a_linux_home_path_trips_the_gate(self):
        self.assertIn("machine path", scan("  path = /home/tobias/Projects/openroad"))

    def test_the_machine_path_rule_is_case_insensitive(self):
        self.assertIn("machine path", scan("  see /USERS/Tobias/Projects/openroad"))

    def test_a_host_on_a_tld_outside_the_old_allowlist_trips_the_gate(self):
        # A finite TLD allowlist would not carry `.pw`.
        self.assertIn("gateway host", scan('gateways: vec!["gw.some-shard.pw".into()],'))

    def test_the_client_version_is_not_an_address(self):
        self.assertEqual(scan("//! Matches the 1.188.0.0 client."), [])

    def test_an_address_next_to_the_client_version_is_still_found(self):
        self.assertIn("gateway address",
                      scan("//! 1.188.0.0 measured against 203.0.113.7"))

    def test_a_source_path_with_a_line_number_is_not_an_endpoint(self):
        # What a TLD allowlist is there to prevent has to keep working.
        self.assertEqual(scan("see client/src/plugins/net/agent.rs:120"), [])

    def test_a_decompile_citation_is_not_an_endpoint(self):
        # The format docs cite C#/C++ sources by file and line all over, and
        # `.cs`/`.hpp` are TLD-shaped once the TLD list is open.
        self.assertEqual(scan("JMX-File-Editor `PrimAniTypeData.cs:29-46` round-trip"), [])


class GateFailsLoud(unittest.TestCase):
    """`git diff` failing must not read as an empty, therefore clean, patch."""

    def test_an_unreadable_range_exits_non_zero(self):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = gate.main("no-such-ref-9f3c2a...HEAD")
        self.assertNotEqual(rc, 0)
        self.assertIn("nothing was scanned", buf.getvalue())


class FileNameScan(unittest.TestCase):
    """A path is content: a dump dir named after an account leaks it alone."""

    def test_a_leaking_file_name_is_a_hit(self):
        with tempfile.TemporaryDirectory() as d:
            run = lambda *a: subprocess.run(a, cwd=d, check=True,
                                            capture_output=True, text=True)
            run("git", "init", "-q", "-b", "main")
            run("git", "config", "user.email", "t@example.com")
            run("git", "config", "user.name", "t")
            Path(d, "seed.txt").write_text("seed\n")
            run("git", "add", "seed.txt")
            run("git", "commit", "-qm", "seed")
            leak = Path(d, "packet_dump", "admin1")
            leak.mkdir(parents=True)
            (leak / "0x3013.log").write_text("00 11 22\n")
            run("git", "add", "packet_dump/admin1/0x3013.log")
            run("git", "commit", "-qm", "dump")
            cwd = os.getcwd()
            buf = io.StringIO()
            try:
                os.chdir(d)
                with contextlib.redirect_stdout(buf):
                    rc = gate.main("HEAD~1...HEAD")
            finally:
                os.chdir(cwd)
        self.assertEqual(rc, 1)
        self.assertIn("in the file name", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
