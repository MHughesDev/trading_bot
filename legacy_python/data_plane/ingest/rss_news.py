"""Minimal RSS/Atom fetch for crypto headlines (no new dependencies)."""

from __future__ import annotations

import hashlib
import logging
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    pass

import httpx

logger = logging.getLogger(__name__)

_NS = {"atom": "http://www.w3.org/2005/Atom"}


def _parse_http_date(s: str | None) -> datetime | None:
    if not s or not s.strip():
        return None
    try:
        dt = parsedate_to_datetime(s.strip())
        if dt.tzinfo is None:
            return dt.replace(tzinfo=UTC)
        return dt.astimezone(UTC)
    except Exception:
        return None


def parse_feed_xml(text: str, feed_url: str) -> list[tuple[str, str, str | None, datetime]]:
    """
    Return list of (title, link, guid_or_none, published_at) from RSS 2.0 or Atom.
    """
    out: list[tuple[str, str, str | None, datetime]] = []
    try:
        root = ET.fromstring(text)  # nosec B314
    except ET.ParseError:
        logger.warning("feed parse error for %s", feed_url)
        return out

    tag = root.tag.split("}")[-1] if "}" in root.tag else root.tag
    if tag == "rss":
        channel = root.find("channel")
        if channel is None:
            return out
        for item in channel.findall("item"):
            title_el = item.find("title")
            link_el = item.find("link")
            guid_el = item.find("guid")
            pub_el = item.find("pubDate")
            title = (title_el.text or "").strip() if title_el is not None else ""
            link = (link_el.text or "").strip() if link_el is not None else ""
            guid = (guid_el.text or "").strip() if guid_el is not None else None
            if not title:
                continue
            pub = _parse_http_date(pub_el.text if pub_el is not None else None) or datetime.now(UTC)
            out.append((title, link or feed_url, guid, pub))
        return out

    if tag == "feed":  # Atom
        for entry in root.findall("atom:entry", _NS) or root.findall("entry"):
            title_el = entry.find("atom:title", _NS) or entry.find("title")
            link_el = entry.find("atom:link", _NS) or entry.find("link")
            updated_el = entry.find("atom:updated", _NS) or entry.find("updated")
            id_el = entry.find("atom:id", _NS) or entry.find("id")
            title = (title_el.text or "").strip() if title_el is not None else ""
            href = ""
            if link_el is not None:
                href = link_el.get("href") or (link_el.text or "").strip()
            guid = (id_el.text or "").strip() if id_el is not None else None
            if not title:
                continue
            pub_raw = updated_el.text if updated_el is not None else None
            pub = _parse_http_date(pub_raw) or datetime.now(UTC)
            out.append((title, href or feed_url, guid, pub))
        return out

    return out


async def fetch_feed_items(
    client: httpx.AsyncClient,
    url: str,
    *,
    timeout: float = 15.0,
) -> list[tuple[str, str, str | None, datetime]]:
    r = await client.get(url, timeout=timeout)
    r.raise_for_status()
    return parse_feed_xml(r.text, url)


def dedup_key(title: str, link: str | None, guid: str | None) -> str:
    raw = f"{guid or ''}|{link or ''}|{title}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:32]
