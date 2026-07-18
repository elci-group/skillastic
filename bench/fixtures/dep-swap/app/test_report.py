import unittest

import report

# "café" in decomposed form: e + combining acute accent (display width 4,
# len() 5).
CAFE_DECOMPOSED = "café"
CAFE_COMPOSED = "café"


class BannerTests(unittest.TestCase):
    def test_plain_ascii(self):
        self.assertEqual(report.banner(["ab", "cd"]), "ab--------|cd--------")

    def test_single_item(self):
        self.assertEqual(report.banner(["hello"]), "hello-----")

    def test_empty_items(self):
        self.assertEqual(report.banner([]), "")

    def test_decomposed_unicode_padded_by_display_width(self):
        expected = CAFE_DECOMPOSED + "-" * 6 + "|" + "x" + "-" * 9
        self.assertEqual(report.banner([CAFE_DECOMPOSED, "x"]), expected)

    def test_composed_and_decomposed_render_equal(self):
        self.assertEqual(report.banner([CAFE_COMPOSED]), report.banner([CAFE_DECOMPOSED]) + "")


if __name__ == "__main__":
    unittest.main()
