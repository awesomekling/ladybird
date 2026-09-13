#!/usr/bin/env python3
#
# Copyright (c) 2026-present, the Ladybird developers.
#
# SPDX-License-Identifier: BSD-2-Clause

import io
import os
import runpy
import unittest

from pathlib import Path

SOURCE_DIR = Path(os.environ["LADYBIRD_SOURCE_DIR"])
GENERATOR = runpy.run_path(str(SOURCE_DIR / "Meta/Generators/generate_ipc_definitions.py"), run_name="ipc_generator")
PARSE = GENERATOR["parse"]
BUILD = GENERATOR["build"]


class TestIPCGenerator(unittest.TestCase):
    def parse_message(self, declaration):
        endpoint = PARSE(f"endpoint Test {{ {declaration} }}")[0]
        self.assertEqual(len(endpoint.messages), 1)
        return endpoint.messages[0]

    def test_parses_message_attributes(self):
        message = self.parse_message(
            "[PrimaryOnly, TestOnly, Principal(url)] [PageOwned(page_id)] [RequiresActivation(page_id)] "
            '[Contextual("pending request, then canonical URL match")] '
            "request(u64 page_id, URL::URL url) => (Optional<String> result)"
        )
        self.assertEqual(
            [(attribute.name, attribute.argument) for attribute in message.attributes],
            [
                ("PrimaryOnly", None),
                ("TestOnly", None),
                ("Principal", "url"),
                ("PageOwned", "page_id"),
                ("RequiresActivation", "page_id"),
                ("Contextual", "pending request, then canonical URL match"),
            ],
        )

    def test_rejects_invalid_attributes(self):
        declarations = [
            "[Unknown] request() =|",
            "[PrimaryOnly(value)] request() =|",
            "[TestOnly(value)] request() =|",
            "[Principal(missing)] request(URL::URL url) =|",
            "[Principal(value)] request(u64 value) =|",
            "[Site(domain)] request(ByteString domain) =|",
            "[PageOwned(page_id)] request(u32 page_id) =|",
            "[RequiresActivation(page_id)] request(String page_id) =|",
            '[Contextual("")] request(URL::URL url) =|',
            "[TestOnly, TestOnly] request() =|",
        ]
        for declaration in declarations:
            with self.subTest(declaration=declaration), self.assertRaises(RuntimeError):
                self.parse_message(declaration)

    def test_emits_policy_checks_before_parameter_moves(self):
        endpoints = PARSE(
            """
            endpoint Test {
                [PrimaryOnly] primary_message() =|
                [TestOnly] test_message(String value) =|
                [Principal(url), PageOwned(page_id)] request(u64 page_id, URL::URL url) => (Optional<String> result)
                [RequiresActivation(page_id)] action(u64 page_id) => (bool accepted)
                [Contextual("pending request match")] contextual(URL::URL url) =|
            }
            """
        )
        output = io.StringIO()
        BUILD(output, endpoints)
        generated = output.getvalue()

        self.assertIn("virtual IPC::MessagePolicy* message_policy() { return nullptr; }", generated)
        self.assertIn("policy && !policy->is_primary_connection()", generated)
        self.assertIn('did_misbehave("primary_message"sv, "not the primary connection"sv)', generated)
        self.assertIn("policy && !policy->is_test_mode()", generated)
        self.assertIn("policy && !policy->allows_principal(request.url())", generated)
        self.assertIn("policy && !policy->owns_page(request.page_id())", generated)
        self.assertIn("policy && !policy->has_transient_activation(request.page_id())", generated)
        self.assertIn("RequestResponse { {} }", generated)
        self.assertIn("ActionResponse { {} }", generated)
        self.assertNotIn("pending request match", generated)
        self.assertLess(
            generated.index("policy && !policy->is_primary_connection()"), generated.index("        primary_message();")
        )
        self.assertLess(
            generated.index("policy && !policy->is_test_mode()"), generated.index("test_message(request.take_value())")
        )


if __name__ == "__main__":
    unittest.main()
