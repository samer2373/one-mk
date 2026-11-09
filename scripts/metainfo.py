#!/usr/bin/env python3
"""Writes the AppStream metainfo (packaging/linux/io.github.onemk.OneMk.metainfo.xml)
for one release to stdout, with that release's notes.

    python3 -I scripts/metainfo.py <version> <YYYY-MM-DD>

The notes come from docs/release-notes/v<version>.md: its first paragraph,
then each "## Heading" as a line with the first sentence of every bullet
under it. AppStream allows only plain
paragraphs and lists in a release description, so the Markdown is reduced to
text. A release with no notes file gets none.
Used by the .deb, the AppImage and the Flatpak, so all three say the same.
"""
import re
import sys
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parent.parent
LONGEST = 220  # characters of one list item; the notes themselves have the rest


def plain(text):
    """Markdown inline syntax reduced to the words."""
    text = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", text)  # [words](link)
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"\1", text)
    text = re.sub(r"(?<![\w*])\*([^*\n]+)\*(?![\w*])", r"\1", text)
    return re.sub(r"\s+", " ", text).strip()


def lead(bullet):
    """The headline of one bullet: its first sentence (two if the first is a
    short label), cut at a word if long."""
    sentences = re.split(r"(?<=[.!?])\s(?=[A-Z])", plain(bullet))
    text = sentences[0]
    if len(text) < 40 and len(sentences) > 1:  # a label such as "Licences." says nothing alone
        text += " " + sentences[1]
    if len(text) > LONGEST:
        text = text[:LONGEST].rsplit(" ", 1)[0].rstrip(",;:( ") + "\u2026"
    return text


def notes(version):
    path = ROOT / "docs" / "release-notes" / f"v{version}.md"
    if not path.is_file():
        return ""
    paragraphs, sections, current = [], [], None
    block = []
    for line in path.read_text(encoding="utf-8").splitlines() + [""]:
        if line.startswith("# "):
            continue
        if line.startswith("## "):
            current = (plain(line[3:]), [])
            sections.append(current)
        elif line.startswith("- "):
            block = [line[2:]]
            if current:
                current[1].append(block)
        elif line.startswith("  ") and block and current and current[1] and current[1][-1] is block:
            block.append(line.strip())
        elif not line.strip():
            block = []
        elif current is None:
            paragraphs.append(plain(line))
    out = ["      <description>"]
    for paragraph in paragraphs[:1]:
        out.append(f"        <p>{escape(paragraph)}</p>")
    for heading, bullets in sections:
        items = [lead(" ".join(bullet)) for bullet in bullets if " ".join(bullet).strip()]
        if not items:
            continue
        out.append(f"        <p>{escape(heading)}</p>")
        out.append("        <ul>")
        out.extend(f"          <li>{escape(item)}</li>" for item in items)
        out.append("        </ul>")
    out.append("      </description>")
    return "\n" + "\n".join(out) + "\n    " if len(out) > 2 else ""


def main(argv):
    if len(argv) != 3 or not re.fullmatch(r"\d+\.\d+\.\d+([-.+][0-9A-Za-z.]+)?", argv[1]) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", argv[2]):
        sys.exit("usage: metainfo.py <version> <YYYY-MM-DD>")
    version, date = argv[1], argv[2]
    template = (ROOT / "packaging" / "linux" / "io.github.onemk.OneMk.metainfo.xml").read_text(encoding="utf-8")
    for placeholder, value in (("@VERSION@", version), ("@DATE@", date), ("@RELEASE_NOTES@", notes(version))):
        if placeholder not in template:
            sys.exit(f"metainfo.py: the template has no {placeholder}")
        template = template.replace(placeholder, value)
    sys.stdout.write(template)


if __name__ == "__main__":
    main(sys.argv)
