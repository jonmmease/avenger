"""Rebuild the three renamed, OFL-licensed audit fixtures."""
from io import BytesIO
from pathlib import Path

import brotli
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

FIXTURES = Path(__file__).resolve().parent
ROOT = FIXTURES.parents[3]


def read_font(path):
    return TTFont(BytesIO(brotli.decompress(path.read_bytes())), recalcTimestamp=False)


def write_font(font, name):
    for record in font["name"].names:
        if record.nameID in (1, 3, 4, 6, 16):
            record.string = name.encode(record.getEncoding())
    for name_id in (1, 3, 4, 6, 16):
        font["name"].setName(name, name_id, 3, 1, 0x409)
    output = BytesIO()
    font.save(output)
    (FIXTURES / (name + ".ttf.br")).write_bytes(brotli.compress(output.getvalue()))


lato = ROOT / "avenger-text/fonts/Lato/Lato-Medium.ttf.br"
font = read_font(lato)
del font["OS/2"]
write_font(font, "AuditNoScriptMetrics")
font = read_font(lato)
font["OS/2"].ySuperscriptXOffset = 400
font["OS/2"].ySubscriptXOffset = -200
write_font(font, "AuditScriptOffsets")
font = read_font(FIXTURES / "NotoSansHebrew.ttf.br")
font = instantiateVariableFont(font, {"wght": 400, "wdth": 100}, inplace=True)
write_font(font, "AuditHebrewRegular")
