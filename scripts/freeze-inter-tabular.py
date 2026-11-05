#!/usr/bin/env python3
"""Build crates/soyuz-app/assets/fonts/inter/InterTabular-Regular.ttf from Inter-Regular.ttf.

egui cannot switch OpenType features, so Inter's tabular figures are not reachable from
the app. This script makes them the default glyphs ("freezes" the `tnum` feature) in a
small font that holds only the characters `tnum` changes: digits and number punctuation.
Every other character falls through to Inter itself, so the file costs little.

Needs fonttools (`pip install fonttools`; 4.68 was used). Run it from the repository root.
"""
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

SRC = Path("crates/soyuz-app/assets/fonts/inter/Inter-Regular.ttf")
OUT = Path("crates/soyuz-app/assets/fonts/inter/InterTabular-Regular.ttf")
FAMILY = "Inter Tabular"


def tnum_mapping(font):
    gsub = font["GSUB"].table
    out = {}
    for rec in gsub.FeatureList.FeatureRecord:
        if rec.FeatureTag != "tnum":
            continue
        for i in rec.Feature.LookupListIndex:
            for st in gsub.LookupList.Lookup[i].SubTable:
                st = getattr(st, "ExtSubTable", st)
                out.update(getattr(st, "mapping", {}))
    return out


def main():
    font = TTFont(SRC)
    mapping = tnum_mapping(font)
    if not mapping:
        sys.exit("no tnum feature")
    unicodes = {}
    for table in font["cmap"].tables:
        if not table.isUnicode():
            continue
        for cp, glyph in list(table.cmap.items()):
            if glyph in mapping:
                table.cmap[cp] = mapping[glyph]
                unicodes[cp] = mapping[glyph]

    opts = subset.Options()
    opts.layout_features = []
    opts.hinting = True
    opts.notdef_outline = False
    opts.glyph_names = False
    opts.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    opts.drop_tables += ["GSUB", "GPOS", "GDEF"]
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=sorted(unicodes))
    sub.subset(font)

    # Copyright and licence records stay; only the names of the font change.
    ps = FAMILY.replace(" ", "")
    for rec in font["name"].names:
        if rec.nameID in (1, 16):
            rec.string = FAMILY
        elif rec.nameID == 3:
            rec.string = f"{rec.toUnicode()};{ps}"
        elif rec.nameID == 4:
            rec.string = FAMILY
        elif rec.nameID == 6:
            rec.string = f"{ps}-Regular"
    font.save(OUT, reorderTables=True)

    widths = {font["hmtx"][font.getBestCmap()[ord(c)]][0] for c in "0123456789"}
    if len(widths) != 1:
        sys.exit(f"digits are not tabular: {widths}")
    print(f"{OUT}: {OUT.stat().st_size} bytes, {len(unicodes)} characters, digit advance {widths}")


if __name__ == "__main__":
    main()
