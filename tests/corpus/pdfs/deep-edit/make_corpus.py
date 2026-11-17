#!/usr/bin/env python3
"""Deep-editing corpus for the Wellfriend PDF editor (QF-5210).

Writes complex real-world page constructs as small PDFs, using only the
Python standard library, Ghostscript (font subsetting, text to outlines) and
fonts installed on the build host. Re-running the script rebuilds the same
files (no timestamps, fixed IDs).

    python3 make_corpus.py OUT_DIR

Samples (see README in OUT_DIR):
  logo-a1-subset-truetype   heavy bold coloured logo, subset simple TrueType (gs)
  logo-a2-subset-cid        the same logo, subset CIDFontType2 / Identity-H (gs)
  logo-b-type3              logo in a Type 3 font without ToUnicode (glyph names g1..)
  logo-c-outlined           logo converted to vector outlines (gs -dNoOutputFonts)
  logo-d-form-nested        logo text in a Form XObject inside a Form XObject, placed twice
  logo-e-clip-gradient      logo as a text clip filled by a shading, and a
                            pattern-filled logo with a stroke outline (Tr 2)
  logo-f-raster             logo as an RGB image with a soft mask
  logo-g-layer              logo in an Optional Content group (BDC /OC ... EMC)
  dense-multicolumn         two columns, TJ kerning, an fi ligature, rotated text, Tz/Ts/Tc/Tw
  vector-annots-inline      grouped vector art, annotations next to page content, inline image
  book-cover                composite book cover (raster background, panel, subset logo
                            in a form with fill+stroke, subtitle, author)
"""
import os
import struct
import subprocess
import sys
import zlib

LATO_BLACK = "/usr/share/fonts/truetype/lato/Lato-Black.ttf"
SERIF = "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf"
SANS = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"

# ---------------------------------------------------------------- TrueType


class Ttf:
    def __init__(self, path):
        self.path = path
        self.data = open(path, "rb").read()
        d = self.data
        num = struct.unpack(">H", d[4:6])[0]
        self.tables = {}
        for i in range(num):
            tag, _, off, length = struct.unpack(">4sIII", d[12 + 16 * i: 28 + 16 * i])
            self.tables[tag.decode("latin1")] = (off, length)
        off, _ = self.tables["head"]
        self.upem = struct.unpack(">H", d[off + 18: off + 20])[0]
        self.bbox = struct.unpack(">hhhh", d[off + 36: off + 44])
        off, _ = self.tables["hhea"]
        self.ascent, self.descent = struct.unpack(">hh", d[off + 4: off + 8])
        nmetrics = struct.unpack(">H", d[off + 34: off + 36])[0]
        off, _ = self.tables["maxp"]
        self.nglyphs = struct.unpack(">H", d[off + 4: off + 6])[0]
        off, _ = self.tables["hmtx"]
        adv = [struct.unpack(">H", d[off + 4 * i: off + 4 * i + 2])[0] for i in range(nmetrics)]
        adv += [adv[-1]] * (self.nglyphs - nmetrics)
        self.advances = adv
        self.cmap = self._cmap()

    def _cmap(self):
        d = self.data
        off, _ = self.tables["cmap"]
        n = struct.unpack(">H", d[off + 2: off + 4])[0]
        sub = None
        for i in range(n):
            pid, eid, so = struct.unpack(">HHI", d[off + 4 + 8 * i: off + 12 + 8 * i])
            if pid == 3 and eid == 1:
                sub = off + so
        assert sub is not None
        assert struct.unpack(">H", d[sub: sub + 2])[0] == 4
        segx2 = struct.unpack(">H", d[sub + 6: sub + 8])[0]
        seg = segx2 // 2
        ends = struct.unpack(">%dH" % seg, d[sub + 14: sub + 14 + segx2])
        starts = struct.unpack(">%dH" % seg, d[sub + 16 + segx2: sub + 16 + 2 * segx2])
        deltas = struct.unpack(">%dh" % seg, d[sub + 16 + 2 * segx2: sub + 16 + 3 * segx2])
        ro_pos = sub + 16 + 3 * segx2
        ros = struct.unpack(">%dH" % seg, d[ro_pos: ro_pos + segx2])
        out = {}
        for i in range(seg):
            for c in range(starts[i], ends[i] + 1):
                if c == 0xFFFF:
                    continue
                if ros[i] == 0:
                    g = (c + deltas[i]) & 0xFFFF
                else:
                    p = ro_pos + 2 * i + ros[i] + 2 * (c - starts[i])
                    g = struct.unpack(">H", d[p: p + 2])[0]
                    if g:
                        g = (g + deltas[i]) & 0xFFFF
                if g:
                    out[c] = g
        return out

    def width(self, gid):
        return round(self.advances[gid] * 1000 / self.upem)

    def gid(self, ch):
        return self.cmap.get(ord(ch), 0)


# ---------------------------------------------------------------- PDF writer


def esc(s):
    if isinstance(s, str):
        s = s.encode("cp1252")
    return b"(" + s.replace(b"\\", b"\\\\").replace(b"(", b"\\(").replace(b")", b"\\)") + b")"


class Pdf:
    def __init__(self):
        self.objects = []

    def reserve(self):
        self.objects.append(None)
        return len(self.objects)

    def set(self, num, body):
        if isinstance(body, str):
            body = body.encode("latin1")
        self.objects[num - 1] = body

    def add(self, body):
        num = self.reserve()
        self.set(num, body)
        return num

    def stream(self, dict_body, data, compress=True, num=None):
        if isinstance(data, str):
            data = data.encode("latin1")
        if isinstance(dict_body, str):
            dict_body = dict_body.encode("latin1")
        if compress:
            data = zlib.compress(data, 9)
            dict_body += b" /Filter /FlateDecode"
        body = b"<< " + dict_body + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream"
        if num is None:
            return self.add(body)
        self.set(num, body)
        return num

    def save(self, path, root, info=None):
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(self.objects):
            assert body is not None, "object %d never set" % (i + 1)
            offsets.append(len(out))
            out += b"%d 0 obj\n" % (i + 1) + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(self.objects) + 1)
        for o in offsets:
            out += b"%010d 00000 n \n" % o
        trailer = b"<< /Size %d /Root %d 0 R" % (len(self.objects) + 1, root)
        if info:
            trailer += b" /Info %d 0 R" % info
        trailer += b" /ID [<5746454449544f52434f52505553> <5746454449544f52434f52505553>] >>"
        out += b"trailer\n" + trailer + b"\nstartxref\n%d\n%%%%EOF\n" % xref
        with open(path, "wb") as f:
            f.write(out)

    # -- fonts --
    def truetype_simple(self, ttf, name):
        """A full embedded simple TrueType font, WinAnsiEncoding."""
        widths = []
        for code in range(32, 256):
            try:
                ch = bytes([code]).decode("cp1252")
            except UnicodeDecodeError:
                widths.append(0)
                continue
            widths.append(ttf.width(ttf.gid(ch)))
        ff = self.stream("/Length1 %d" % len(ttf.data), ttf.data)
        bb = [round(v * 1000 / ttf.upem) for v in ttf.bbox]
        desc = self.add(
            "<< /Type /FontDescriptor /FontName /%s /Flags 32 /FontBBox [%d %d %d %d] /ItalicAngle 0 "
            "/Ascent %d /Descent %d /CapHeight %d /StemV 160 /FontFile2 %d 0 R >>"
            % (name, *bb, round(ttf.ascent * 1000 / ttf.upem), round(ttf.descent * 1000 / ttf.upem),
               round(ttf.ascent * 700 / ttf.upem), ff))
        return self.add(
            "<< /Type /Font /Subtype /TrueType /BaseFont /%s /FirstChar 32 /LastChar 255 /Widths [%s] "
            "/Encoding /WinAnsiEncoding /FontDescriptor %d 0 R >>"
            % (name, " ".join(map(str, widths)), desc))

    def truetype_cid(self, ttf, name, extra_unicode=None):
        """A full embedded Type0/CIDFontType2 font, Identity-H, codes = glyph ids,
        with a ToUnicode CMap for the cmap (and `extra_unicode` {gid: text})."""
        ff = self.stream("/Length1 %d" % len(ttf.data), ttf.data)
        bb = [round(v * 1000 / ttf.upem) for v in ttf.bbox]
        desc = self.add(
            "<< /Type /FontDescriptor /FontName /%s /Flags 32 /FontBBox [%d %d %d %d] /ItalicAngle 0 "
            "/Ascent %d /Descent %d /CapHeight %d /StemV 160 /FontFile2 %d 0 R >>"
            % (name, *bb, round(ttf.ascent * 1000 / ttf.upem), round(ttf.descent * 1000 / ttf.upem),
               round(ttf.ascent * 700 / ttf.upem), ff))
        used = sorted(set(ttf.cmap.values()) | set((extra_unicode or {}).keys()))
        w = " ".join("%d [%d]" % (g, ttf.width(g)) for g in used)
        cid = self.add(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /%s /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) "
            "/Supplement 0 >> /FontDescriptor %d 0 R /DW 1000 /W [%s] /CIDToGIDMap /Identity >>" % (name, desc, w))
        rev = {}
        for code, g in ttf.cmap.items():
            if 0x20 <= code < 0xD800 and g not in rev:
                rev[g] = chr(code)
        rev.update(extra_unicode or {})
        entries = sorted(rev.items())
        cmap = ["/CIDInit /ProcSet findresource begin 12 dict begin begincmap",
                "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def",
                "/CMapName /Adobe-Identity-UCS def /CMapType 2 def",
                "1 begincodespacerange <0000> <FFFF> endcodespacerange"]
        for i in range(0, len(entries), 100):
            chunk = entries[i: i + 100]
            cmap.append("%d beginbfchar" % len(chunk))
            for g, text in chunk:
                hexs = "".join("%04X" % ord(c) for c in text)
                cmap.append("<%04X> <%s>" % (g, hexs))
            cmap.append("endbfchar")
        cmap += ["endcmap CMapName currentdict /CMap defineresource pop end end"]
        tu = self.stream("", "\n".join(cmap))
        return self.add(
            "<< /Type /Font /Subtype /Type0 /BaseFont /%s /Encoding /Identity-H /DescendantFonts [%d 0 R] /ToUnicode %d 0 R >>"
            % (name, cid, tu))


def cid_hex(ttf, text):
    return "<" + "".join("%04X" % ttf.gid(c) for c in text) + ">"


def tj_kerned(ttf, text, kern=None):
    """A TJ array with a small kern between some pairs (simple WinAnsi font)."""
    parts = []
    for i, ch in enumerate(text):
        parts.append(esc(ch).decode("latin1"))
        if kern and i % 4 == 3:
            parts.append(str(kern))
    return "[" + " ".join(parts) + "] TJ"


def catalog(pdf, page_ids, extra=""):
    pages = pdf.reserve()
    root = pdf.add("<< /Type /Catalog /Pages %d 0 R %s >>" % (pages, extra))
    pdf.set(pages, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join("%d 0 R" % p for p in page_ids), len(page_ids)))
    return root, pages


def page(pdf, pages_ref_holder, content, resources, size=(612, 792), annots=None):
    """Page object whose /Parent is patched when the catalog is written."""
    extra = " /Annots [%s]" % " ".join("%d 0 R" % a for a in annots) if annots else ""
    contents = pdf.stream("", content)
    num = pdf.reserve()
    pages_ref_holder.append((num, "<< /Type /Page /Parent %%PAGES%% 0 R /MediaBox [0 0 %d %d] /Resources %s /Contents %d 0 R%s >>"
                             % (size[0], size[1], resources, contents, extra)))
    return num


def finish(pdf, holder, path, cat_extra=""):
    ids = [n for n, _ in holder]
    pages = pdf.reserve()
    for num, body in holder:
        pdf.set(num, body.replace("%PAGES%", str(pages)))
    pdf.set(pages, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join("%d 0 R" % p for p in ids), len(ids)))
    root = pdf.add("<< /Type /Catalog /Pages %d 0 R %s >>" % (pages, cat_extra))
    pdf.save(path, root)


LOGO = "WELLBOOK"
ORANGE = "0.91 0.39 0.08 rg"
BODY = "A novel by Jane Example. Chapter one begins here."


def body_text(font="/FS", y=520):
    return "BT %s 16 Tf 0.1 0.1 0.1 rg 72 %d Td %s Tj ET\n" % (font, y, esc(BODY).decode("latin1"))


def gs(args, src, dst):
    # The host's Ghostscript confinement only lets it read and write under
    # /tmp, so it works on staged copies there.
    import shutil
    import tempfile
    with tempfile.TemporaryDirectory(dir="/tmp") as work:
        staged_in = os.path.join(work, "in.pdf")
        staged_out = os.path.join(work, "out.pdf")
        shutil.copyfile(src, staged_in)
        subprocess.run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite",
                        "-dCompatibilityLevel=1.7", "-dAutoRotatePages=/None", *args,
                        "-sOutputFile=" + staged_out, staged_in], check=True)
        shutil.copyfile(staged_out, dst)


# ---------------------------------------------------------------- samples


def logo_source(path, cid, lato, serif):
    pdf = Pdf()
    holder = []
    if cid:
        fl = pdf.truetype_cid(lato, "Lato-Black")
        logo = "BT /FL 64 Tf %s 0.35 0.05 0.02 RG 2 w 2 Tr 1.5 Tc 64 600 Td %s Tj ET\n" % (ORANGE, cid_hex(lato, LOGO))
    else:
        fl = pdf.truetype_simple(lato, "Lato-Black")
        logo = "BT /FL 64 Tf %s 0.35 0.05 0.02 RG 2 w 2 Tr 1.5 Tc 64 600 Td %s Tj ET\n" % (ORANGE, esc(LOGO).decode("latin1"))
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    content = logo + body_text()
    page(pdf, holder, content, "<< /Font << /FL %d 0 R /FS %d 0 R >> >>" % (fl, fs))
    finish(pdf, holder, path)


# 5x7 block letters (rows top to bottom) for the Type 3 font and the raster logo.
BLOCKS = {
    "W": ["X...X", "X...X", "X...X", "X.X.X", "X.X.X", "XX.XX", "X...X"],
    "E": ["XXXXX", "X....", "X....", "XXXX.", "X....", "X....", "XXXXX"],
    "L": ["X....", "X....", "X....", "X....", "X....", "X....", "XXXXX"],
    "B": ["XXXX.", "X...X", "X...X", "XXXX.", "X...X", "X...X", "XXXX."],
    "O": [".XXX.", "X...X", "X...X", "X...X", "X...X", "X...X", ".XXX."],
    "K": ["X...X", "X..X.", "X.X..", "XX...", "X.X..", "X..X.", "X...X"],
}


def block_glyph_proc(letter):
    """Glyph procedure in a 1000-unit space: 140-unit cells, d1 (uncoloured)."""
    rows = BLOCKS[letter]
    ops = ["760 0 0 0 700 980 d1"]
    for r, row in enumerate(rows):
        for c, cell in enumerate(row):
            if cell == "X":
                ops.append("%d %d 140 140 re" % (c * 140, (6 - r) * 140))
    ops.append("f")
    return "\n".join(ops)


def logo_type3(path, serif):
    pdf = Pdf()
    holder = []
    letters = sorted(set(LOGO))
    procs = {}
    for i, letter in enumerate(letters):
        procs["g%d" % (i + 1)] = pdf.stream("", block_glyph_proc(letter))
    code_of = {letter: 65 + i for i, letter in enumerate(letters)}
    diffs = " ".join("%d /g%d" % (65 + i, i + 1) for i in range(len(letters)))
    t3 = pdf.add(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 700 980] /FontMatrix [0.001 0 0 0.001 0 0] "
        "/CharProcs << %s >> /Encoding << /Type /Encoding /Differences [%s] >> /FirstChar 65 /LastChar %d "
        "/Widths [%s] /Resources << >> >>"
        % (" ".join("/%s %d 0 R" % (k, v) for k, v in procs.items()), diffs, 65 + len(letters) - 1,
           " ".join(["800"] * len(letters))))
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    codes = bytes(code_of[c] for c in LOGO)
    content = "BT /T3 64 Tf %s 64 600 Td %s Tj ET\n" % (ORANGE, esc(codes).decode("latin1")) + body_text()
    page(pdf, holder, content, "<< /Font << /T3 %d 0 R /FS %d 0 R >> >>" % (t3, fs))
    finish(pdf, holder, path)


def logo_form_nested(path, lato, serif):
    pdf = Pdf()
    holder = []
    fl = pdf.truetype_simple(lato, "Lato-Black")
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    inner = pdf.stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 420 90] /Resources << /Font << /FL %d 0 R >> >>" % fl,
        "BT /FL 64 Tf %s 8 18 Td %s Tj ET\n" % (ORANGE, esc(LOGO).decode("latin1")))
    outer = pdf.stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 460 120] /Matrix [1 0 0 1 0 0] "
        "/Resources << /XObject << /Fi %d 0 R >> >>" % inner,
        "0.1 0.15 0.3 rg 0 0 460 120 re f\nq 1 0 0 1 20 15 cm /Fi Do Q\n")
    content = "q 1 0 0 1 60 560 cm /Fo Do Q\nq 0.5 0 0 0.5 60 300 cm /Fi Do Q\n" + body_text()
    page(pdf, holder, content, "<< /Font << /FS %d 0 R >> /XObject << /Fo %d 0 R /Fi %d 0 R >> >>" % (fs, outer, inner))
    finish(pdf, holder, path)


def logo_clip_gradient(path, lato, serif):
    pdf = Pdf()
    holder = []
    fl = pdf.truetype_simple(lato, "Lato-Black")
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    fn = pdf.add("<< /FunctionType 2 /Domain [0 1] /C0 [0.98 0.75 0.1] /C1 [0.85 0.1 0.2] /N 1 >>")
    sh = pdf.add("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [64 600 520 660] /Function %d 0 R /Extend [true true] >>" % fn)
    fn2 = pdf.add("<< /FunctionType 2 /Domain [0 1] /C0 [0.1 0.6 0.9] /C1 [0.2 0.1 0.6] /N 1 >>")
    pat = pdf.add("<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB "
                  "/Coords [64 400 400 460] /Function %d 0 R /Extend [true true] >> >>" % fn2)
    content = (
        "q BT /FL 64 Tf 7 Tr 64 600 Td %s Tj ET /Sh1 sh Q\n" % esc(LOGO).decode("latin1")
        + "BT /Pattern cs /P1 scn 0.05 0.05 0.2 RG 2.5 w 2 Tr /FL 60 Tf 64 400 Td (PRESS) Tj ET\n"
        + body_text(y=300))
    page(pdf, holder, content,
         "<< /Font << /FL %d 0 R /FS %d 0 R >> /Shading << /Sh1 %d 0 R >> /Pattern << /P1 %d 0 R >> >>" % (fl, fs, sh, pat))
    finish(pdf, holder, path)


def raster_logo_pixels(scale=6):
    """RGB + alpha of the block-letter logo (orange on transparent)."""
    cols = len(LOGO) * 6 - 1
    w, h = cols * scale, 7 * scale
    rgb = bytearray()
    alpha = bytearray()
    for y in range(h):
        r = y // scale
        for x in range(w):
            c = x // scale
            li, lc = divmod(c, 6)
            on = lc < 5 and BLOCKS[LOGO[li]][r][lc] == "X"
            rgb += bytes([232, 99, 20]) if on else bytes([255, 255, 255])
            alpha.append(255 if on else 0)
    return w, h, bytes(rgb), bytes(alpha)


def logo_raster(path, serif):
    pdf = Pdf()
    holder = []
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    w, h, rgb, alpha = raster_logo_pixels()
    smask = pdf.stream("/Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray /BitsPerComponent 8" % (w, h), alpha)
    img = pdf.stream("/Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask %d 0 R" % (w, h, smask), rgb)
    content = "q %d 0 0 %d 64 590 cm /Im1 Do Q\n" % (w * 1.4, h * 1.4) + body_text()
    page(pdf, holder, content, "<< /Font << /FS %d 0 R >> /XObject << /Im1 %d 0 R >> >>" % (fs, img))
    finish(pdf, holder, path)


def logo_layer(path, lato, serif):
    pdf = Pdf()
    holder = []
    fl = pdf.truetype_simple(lato, "Lato-Black")
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    ocg = pdf.add("<< /Type /OCG /Name (Logo layer) >>")
    content = ("/OC /L1 BDC\nBT /FL 64 Tf %s 64 600 Td %s Tj ET\n0.91 0.39 0.08 RG 4 w 64 585 m 520 585 l S\nEMC\n"
               % (ORANGE, esc(LOGO).decode("latin1")) + body_text())
    page(pdf, holder, content, "<< /Font << /FL %d 0 R /FS %d 0 R >> /Properties << /L1 %d 0 R >> >>" % (fl, fs, ocg))
    finish(pdf, holder, path, "/OCProperties << /OCGs [%d 0 R] /D << /Order [%d 0 R] /ON [%d 0 R] >> >>" % (ocg, ocg, ocg))


LOREM = ("Officially the field office reported that the final figures were efficient and "
         "financially fine; the affiliated firms filed their fiscal files in time. ") * 3


def dense_multicolumn(path, serif, sans):
    pdf = Pdf()
    holder = []
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    fh = pdf.truetype_simple(sans, "LiberationSans")
    lig = sans.cmap.get(0xFB01)
    extra = {lig: "fi"} if lig else {}
    fc = pdf.truetype_cid(sans, "LiberationSans-CID", extra)
    lines = []
    lines.append("BT /FH 22 Tf 2 Tc 6 Tw 80 Tz 0.15 0.2 0.45 rg 72 730 Td (Quarterly Field Report) Tj ET")
    lines.append("BT /FH 9 Tf 0 Tc 0 Tw 100 Tz 4 Ts 300 730 Td (note) Tj 0 Ts ET")
    words = LOREM.split()
    for col, x in enumerate((72, 320)):
        y = 690
        chunk = words[col::2]
        line = []
        out = []
        for w in chunk:
            line.append(w)
            if len(" ".join(line)) > 38:
                out.append(" ".join(line))
                line = []
        if line:
            out.append(" ".join(line))
        lines.append("BT /FS 10 Tf 12 TL 0 g %d %d Td" % (x, y))
        for i, text in enumerate(out):
            if i == 2:
                lines.append("0.3 Tc 1.5 Tw")
            if i == 4:
                lines.append("0 Tc 0 Tw")
            lines.append(tj_kerned(serif, text, kern=-40 if i % 2 else 30) + " T*")
        lines.append("ET")
    if lig:
        # "fi" as one ligature glyph in a CID font, between ordinary glyphs.
        s = "<%04X%04X%04X%04X%04X>" % (sans.gid("P"), lig, sans.gid("l"), sans.gid("e"), sans.gid("s"))
        lines.append("BT /FC 14 Tf 0 0 0.5 rg 72 470 Td %s Tj ET" % s)  # "Pfiles"
    lines.append("BT /FH 12 Tf 0.6 0 0 rg 0.866 0.5 -0.5 0.866 450 420 Tm (Rotated thirty degrees) Tj ET")
    lines.append("BT /FH 12 Tf 0 0.4 0 rg 0 1 -1 0 560 120 Tm (Vertical margin note) Tj ET")
    page(pdf, holder, "\n".join(lines) + "\n", "<< /Font << /FS %d 0 R /FH %d 0 R /FC %d 0 R >> >>" % (fs, fh, fc))
    finish(pdf, holder, path)


def vector_annots_inline(path, serif):
    pdf = Pdf()
    holder = []
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    k = 0.5523
    circle = ("100 150 m 100 %.2f %.2f 200 150 200 c %.2f 200 200 %.2f 200 150 c "
              "200 %.2f %.2f 100 150 100 c %.2f 100 100 %.2f 100 150 c h" % (
                  150 + 50 * k, 150 - 50 * k, 150 + 50 * k, 150 + 50 * k, 150 - 50 * k, 150 + 50 * k, 150 - 50 * k, 150 - 50 * k))
    group = ("q 1 0 0 1 60 420 cm\n"
             "0.2 0.5 0.8 rg %s f\n"
             "0.95 0.8 0.1 rg 260 100 m 280 160 l 340 160 l 290 195 l 310 255 l 260 220 l 210 255 l 230 195 l 180 160 l 240 160 l h f\n"
             "0.1 0.1 0.1 RG 3 w 380 100 120 150 re S\nQ\n" % circle)
    inline = ("q 120 0 0 60 72 300 cm BI /W 4 /H 2 /CS /RGB /BPC 8 ID "
              "\xff\x00\x00\x00\xff\x00\x00\x00\xff\xff\xff\x00\x00\x00\x00\x80\x80\x80\xff\x00\xff\x00\xff\xff\nEI Q\n")
    page_rect = "0.85 0.85 0.85 rg 320 280 200 80 re f\n"
    content = group + inline + page_rect + body_text(y=220)
    ap = pdf.stream("/Type /XObject /Subtype /Form /BBox [0 0 200 80]", "0.9 0.1 0.1 RG 4 w 2 2 196 76 re S\n")
    sq = pdf.add("<< /Type /Annot /Subtype /Square /Rect [320 280 520 360] /C [0.9 0.1 0.1] /BS << /W 4 >> "
                 "/AP << /N %d 0 R >> /Contents (Square annotation over page content) /F 4 >>" % ap)
    ft_ap = pdf.stream("/Type /XObject /Subtype /Form /BBox [0 0 180 30] /Resources << /Font << /FS %d 0 R >> >>" % fs,
                       "BT /FS 12 Tf 0 0 0 rg 4 10 Td (Free text annotation) Tj ET\n")
    ft = pdf.add("<< /Type /Annot /Subtype /FreeText /Rect [72 180 252 210] /DA (/Helv 12 Tf 0 g) "
                 "/AP << /N %d 0 R >> /Contents (Free text annotation) /F 4 >>" % ft_ap)
    page(pdf, holder, content, "<< /Font << /FS %d 0 R >> >>" % fs, annots=[sq, ft])
    finish(pdf, holder, path)


def book_cover_source(path, lato, serif, sans):
    pdf = Pdf()
    holder = []
    W, H = 432, 648
    fl = pdf.truetype_simple(lato, "Lato-Black")
    fs = pdf.truetype_simple(serif, "LiberationSerif")
    fh = pdf.truetype_simple(sans, "LiberationSans")
    # Raster background: a vertical gradient with a band, 72 x 108 pixels.
    bw, bh = 72, 108
    px = bytearray()
    for y in range(bh):
        for x in range(bw):
            t = y / bh
            band = 1.0 if 40 < y < 60 else 0.0
            px += bytes([int(20 + 40 * t + 60 * band), int(40 + 60 * t), int(90 + 100 * t)])
    bg = pdf.stream("/Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceRGB /BitsPerComponent 8" % (bw, bh), bytes(px))
    title = pdf.stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 400 120] /Resources << /Font << /FL %d 0 R >> >>" % fl,
        "BT /FL 66 Tf 0.97 0.62 0.12 rg 0.3 0.05 0.0 RG 2.5 w 2 Tr 1 Tc 10 40 Td %s Tj ET\n" % esc(LOGO).decode("latin1"))
    content = (
        "q %d 0 0 %d 0 0 cm /Bg Do Q\n" % (W, H)
        + "0.05 0.07 0.15 rg 16 400 400 150 re f\n"
        + "q 1 0 0 1 16 420 cm /Title Do Q\n"
        + "BT /FS 20 Tf 1 1 1 rg 40 380 Td (The Long Way Round) Tj ET\n"
        + "BT /FH 14 Tf 0.95 0.95 0.95 rg 2 Tc 40 80 Td (JANE EXAMPLE) Tj ET\n")
    page(pdf, holder, content, "<< /Font << /FS %d 0 R /FH %d 0 R >> /XObject << /Bg %d 0 R /Title %d 0 R >> >>" % (fs, fh, bg, title), size=(W, H))
    finish(pdf, holder, path)


def qpdf_check(path):
    """`qpdf --check` (confined to /tmp on the build host, like gs)."""
    import shutil
    import tempfile
    with tempfile.TemporaryDirectory(dir="/tmp") as work:
        staged = os.path.join(work, "check.pdf")
        shutil.copyfile(path, staged)
        subprocess.run(["qpdf", "--check", staged], check=True, stdout=subprocess.DEVNULL)


def main():
    out = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else ".")
    os.makedirs(out, exist_ok=True)
    src = os.path.join(out, "_src")
    os.makedirs(src, exist_ok=True)
    lato = Ttf(LATO_BLACK)
    serif = Ttf(SERIF)
    sans = Ttf(SANS)
    p = lambda name: os.path.join(out, name + ".pdf")
    s = lambda name: os.path.join(src, name + ".pdf")

    logo_source(s("logo-a1"), False, lato, serif)
    gs([], s("logo-a1"), p("logo-a1-subset-truetype"))
    logo_source(s("logo-a2"), True, lato, serif)
    gs([], s("logo-a2"), p("logo-a2-subset-cid"))
    logo_type3(p("logo-b-type3"), serif)
    gs(["-dNoOutputFonts"], s("logo-a1"), p("logo-c-outlined"))
    logo_form_nested(p("logo-d-form-nested"), lato, serif)
    logo_clip_gradient(p("logo-e-clip-gradient"), lato, serif)
    logo_raster(p("logo-f-raster"), serif)
    logo_layer(p("logo-g-layer"), lato, serif)
    dense_multicolumn(p("dense-multicolumn"), serif, sans)
    vector_annots_inline(p("vector-annots-inline"), serif)
    book_cover_source(s("book-cover"), lato, serif, sans)
    gs([], s("book-cover"), p("book-cover"))
    for name in sorted(os.listdir(out)):
        if name.endswith(".pdf"):
            qpdf_check(os.path.join(out, name))
    print("corpus written to", out)


if __name__ == "__main__":
    main()
