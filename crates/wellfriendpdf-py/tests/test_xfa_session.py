"""Interactive XFA form sessions through the Python binding."""
import json

import pytest
import wellfriendpdf

TEMPLATE = b"""<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form1" layout="tb">
  <pageSet><pageArea name="P"><contentArea w="200pt" h="200pt"/><medium short="200pt" long="200pt"/></pageArea></pageSet>
  <field name="qty" w="160pt" h="16pt"><ui><numericEdit/></ui><value><decimal/></value>
    <event activity="exit"><script>xfa.host.messageBox("exit")</script></event></field>
  <field name="total" w="160pt" h="16pt"><ui><numericEdit/></ui><value><decimal/></value>
    <calculate><script>qty * 2</script></calculate></field>
  <field name="agree" w="20pt" h="20pt"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items></field>
</subform></template>"""

CONFIG = (
    b'<config xmlns="http://www.xfa.org/schema/xci/3.0/"><acrobat><acrobat7>'
    b"<dynamicRender>required</dynamicRender></acrobat7></acrobat></config>"
)
DATA = b'<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data/></xfa:datasets>'


def stream(data):
    return b"<< /Length " + str(len(data)).encode("ascii") + b" >>\nstream\n" + data + b"\nendstream"


def xfa_pdf():
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        b"<< /Fields [] /XFA [(config) 5 0 R (template) 6 0 R (datasets) 7 0 R] >>",
        stream(CONFIG),
        stream(TEMPLATE),
        stream(DATA),
    ]
    pdf = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode("ascii") + body + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets:
        pdf.extend(f"{offset:010d} 00000 n \n".encode("ascii"))
    pdf.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode("ascii")
    )
    return bytes(pdf)


def test_xfa_session_events_render_and_save():
    source = xfa_pdf()
    session = wellfriendpdf.Document.from_bytes(source).xfa_session()
    outcome = session.set_value("form1.qty", "3")
    assert outcome["committed"] is True
    assert outcome["messages"] == ["exit"]
    total = json.loads(session.command_json('{"op":"value","som":"form1.total"}'))
    assert total["value"] == "6"
    typed = session.type_text("form1.qty", "2x")
    assert typed["committed"] is True
    assert [event["activity"] for event in typed["events"]].count("change") == 2
    total = json.loads(session.command_json('{"op":"value","som":"form1.total"}'))
    assert total["value"] == "64"
    session.click("form1.agree")
    fields = {field["name"]: field for field in session.fields()["fields"]}
    assert fields["agree"]["checked"] is True
    assert "<qty>32</qty>" in session.export_data_xml()
    assert session.render_page_png(1, 36).startswith(b"\x89PNG")
    saved = session.save_pdf()
    assert saved.startswith(source)
    again = wellfriendpdf.Document.from_bytes(saved).xfa_session()
    assert json.loads(again.command_json('{"op":"value","som":"form1.qty"}'))["value"] == "32"
    with pytest.raises(wellfriendpdf.WellfriendError):
        session.click("form1.nonesuch")
