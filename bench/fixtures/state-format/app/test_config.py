import unittest

import config


class SettingsTests(unittest.TestCase):
    def test_values(self):
        settings = config.load_settings()
        self.assertEqual(settings["host"], "internal.example.com")
        self.assertEqual(settings["port"], 9090)
        self.assertIs(settings["debug"], True)

    def test_types(self):
        settings = config.load_settings()
        self.assertIsInstance(settings["host"], str)
        self.assertIsInstance(settings["port"], int)
        self.assertIsInstance(settings["debug"], bool)

    def test_keys(self):
        self.assertEqual(set(config.load_settings()), {"host", "port", "debug"})


if __name__ == "__main__":
    unittest.main()
