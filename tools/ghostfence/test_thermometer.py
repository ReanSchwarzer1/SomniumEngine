import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools.ghostfence.thermometer import (
    FIXED_SEED,
    evidence_paths,
    fixed_environment,
)


class ThermometerFixtureTests(unittest.TestCase):
    def test_names_are_the_phase_evidence_contract(self) -> None:
        paths = evidence_paths(Path("evidence"), "DREAMS-B", "island-ground", "on")
        self.assertEqual(paths.timing, Path("evidence/DREAMS-B_island-ground_on.somtime"))
        self.assertEqual(paths.picture, Path("evidence/DREAMS-B_island-ground_on.png"))

    def test_only_the_requested_switch_changes_between_off_and_on(self) -> None:
        paths = evidence_paths(Path("evidence"), "DREAMS-B", "coastal-ground", "off")
        with mock.patch.dict(os.environ, {}, clear=True):
            off = fixed_environment(paths, "coastal-ground", "SOMNIUM_DREAMS_GRAIN", "off", 180, 300)
            on = fixed_environment(paths, "coastal-ground", "SOMNIUM_DREAMS_GRAIN", "on", 180, 300)
        changed = {
            key
            for key in off | on
            if key != "SOMNIUM_TIME_LABEL" and off.get(key) != on.get(key)
        }
        self.assertEqual(changed, {"SOMNIUM_DREAMS_GRAIN"})
        self.assertEqual(off["SOMNIUM_DREAMS_SEED"], FIXED_SEED)
        self.assertEqual(off["SOMNIUM_CAPTURE_FRAME"], "240")

    def test_default_really_unsets_the_switch(self) -> None:
        paths = evidence_paths(Path("evidence"), "DREAMS-B", "coastal-ground", "default")
        with mock.patch.dict(os.environ, {"SOMNIUM_DREAMS_GRAIN": "1"}, clear=True):
            env = fixed_environment(
                paths,
                "coastal-ground",
                "SOMNIUM_DREAMS_GRAIN",
                "default",
                0,
                8,
            )
        self.assertNotIn("SOMNIUM_DREAMS_GRAIN", env)

    def test_ambient_dreams_switches_cannot_confound_a_run(self) -> None:
        paths = evidence_paths(Path("evidence"), "DREAMS-B", "island-ground", "on")
        with mock.patch.dict(
            os.environ,
            {"SOMNIUM_DREAMS_STF": "1", "SOMNIUM_DREAMS_FUTURE": "1"},
            clear=True,
        ):
            env = fixed_environment(
                paths,
                "island-ground",
                "SOMNIUM_DREAMS_GRAIN",
                "on",
                0,
                8,
            )
        self.assertEqual(env["SOMNIUM_DREAMS_GRAIN"], "1")
        self.assertEqual(env["SOMNIUM_DREAMS_STF"], "0")
        self.assertNotIn("SOMNIUM_DREAMS_FUTURE", env)


if __name__ == "__main__":
    unittest.main()
