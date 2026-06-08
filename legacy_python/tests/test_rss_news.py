"""RSS parse and dedup (FB-F2)."""

from __future__ import annotations

from data_plane.ingest.rss_news import dedup_key, parse_feed_xml


def test_parse_rss2_basic() -> None:
    xml = """<?xml version="1.0"?>
    <rss><channel>
      <item><title>BTC rally</title><link>https://ex.com/a</link><guid>g1</guid>
      <pubDate>Mon, 01 Jan 2024 12:00:00 GMT</pubDate></item>
    </channel></rss>"""
    rows = parse_feed_xml(xml, "https://feed")
    assert len(rows) == 1
    assert rows[0][0] == "BTC rally"
    assert rows[0][1] == "https://ex.com/a"


def test_dedup_key_stable() -> None:
    assert dedup_key("t", "u", "g") == dedup_key("t", "u", "g")
