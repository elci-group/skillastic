import unittest

import service


class ServiceTests(unittest.TestCase):
    def test_email_user1(self):
        self.assertEqual(service.get_user_email(1), "alice@example.com")

    def test_email_user2(self):
        self.assertEqual(service.get_user_email(2), "bob@example.com")

    def test_email_user3(self):
        self.assertEqual(service.get_user_email(3), "carol@example.com")

    def test_display_name_user1(self):
        self.assertEqual(service.get_user_display_name(1), "Alice Johnson")

    def test_display_name_user2(self):
        self.assertEqual(service.get_user_display_name(2), "Bob Smith")

    def test_display_name_user3(self):
        self.assertEqual(service.get_user_display_name(3), "Carol Williams")

    def test_unknown_user_returns_none(self):
        self.assertIsNone(service.get_user_email(999))
        self.assertIsNone(service.get_user_display_name(999))


if __name__ == "__main__":
    unittest.main()
