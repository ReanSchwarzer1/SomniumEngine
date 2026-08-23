"""Opt-in red gates established by CONTROL-A and closed by CONTROL-B."""

from __future__ import annotations

import os
import unittest

from .generate import missing_editable_fields, missing_property_editors, schema_blocks


class AuditParserTests(unittest.TestCase):
    def test_explicit_non_editable_flags_are_not_credited_to_the_inspector(self) -> None:
        schemas = {schema.stable_id: schema for schema in schema_blocks()}
        rigid_body = schemas["somnium.RigidBody"]
        self.assertNotIn("body", rigid_body.editable_fields)
        self.assertNotIn("grounded", rigid_body.editable_fields)
        self.assertIn("velocity", rigid_body.editable_fields)
        self.assertIn("script_driven", rigid_body.editable_fields)

    def test_runtime_mesh_offsets_are_not_credited_to_the_inspector(self) -> None:
        schemas = {schema.stable_id: schema for schema in schema_blocks()}
        self.assertEqual((), schemas["somnium.Mesh"].editable_fields)


@unittest.skipUnless(
    os.environ.get("SOMNIUM_CONTROL_B_GATES") == "1",
    "CONTROL-B completeness gates are intentionally red during CONTROL-A",
)
class ControlBCompletenessGates(unittest.TestCase):
    def test_every_editable_schema_field_has_a_generated_inspector_row(self) -> None:
        self.assertEqual([], missing_editable_fields())

    def test_every_field_type_has_a_registered_property_editor(self) -> None:
        self.assertEqual([], missing_property_editors())


if __name__ == "__main__":
    unittest.main()
