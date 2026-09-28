"""Regression source only; not executed during the source-only implementation."""
import copy
import hashlib
import json
from concurrent.futures import ThreadPoolExecutor

import pytest
import wellfriendpdf


def source_pdf():
    # One actual source text operand, no runtime dependency on a fixture server.
    content = b"BT /F1 12 Tf 1 0 0 1 10 150 Tm (OLD) Tj ET"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        b"<< /Length " + str(len(content)).encode("ascii") + b" >>\nstream\n" + content + b"\nendstream",
    ]
    pdf = bytearray(b"%PDF-1.7\n")
    offsets = [0]
    for number, body in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode("ascii") + body + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("ascii"))
    for offset in offsets[1:]:
        pdf.extend(f"{offset:010d} 00000 n \n".encode("ascii"))
    pdf.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode("ascii"))
    return bytes(pdf)


def command(session, op, **kwargs):
    return json.loads(session.command_json(json.dumps({"op": op, **kwargs})))


def draft(session):
    return {
        "story_id": "python-session",
        "input_sha256": json.loads(session.status_json())["revision_sha256"],
        "frames": [{"id": "body", "page": 1, "logical_range": [0, 3],
                    "expected_text": "OLD", "rect": [10, 10, 190, 180]}],
        "paragraphs": [{"id": "p", "text": "New wording", "preferred_font": "Helvetica",
                        "font_size": 12, "line_height": 14}],
        "fonts": [],
        "allow_font_substitution": True,
    }


def test_receipt_save_reopen_reedit_and_exact_history():
    original = source_pdf()
    with wellfriendpdf.StoryEditSession(original) as session:
        request = draft(session)
        layout = json.loads(session.preview_json(json.dumps(request)))
        assert session.bytes() == original
        changed = copy.deepcopy(request)
        changed["paragraphs"][0]["text"] = "Not approved"
        with pytest.raises(wellfriendpdf.WellfriendError):
            session.checkpoint_json(json.dumps(changed), json.dumps(layout["receipt"]))
        assert session.bytes() == original
        report = json.loads(session.checkpoint_json(json.dumps(request), json.dumps(layout["receipt"])))
        edited = session.bytes()
        assert isinstance(edited, bytes)
        assert report["output_sha256"] == hashlib.sha256(edited).hexdigest()
        text = wellfriendpdf.open(edited).page(1).text
        assert "New wording" in text and "OLD" not in text
        assert session.undo() and session.bytes() == original
        assert session.redo() and session.bytes() == edited
        with pytest.raises(wellfriendpdf.WellfriendError):
            session.checkpoint_json(json.dumps(request), json.dumps(layout["receipt"]))
    with wellfriendpdf.StoryEditSession(edited) as reopened:
        request = command(reopened, "saved_stories")[0]["request"]
        request["paragraphs"][0]["text"] = "Second edit"
        layout = json.loads(reopened.preview_json(json.dumps(request)))
        reopened.checkpoint_json(json.dumps(request), json.dumps(layout["receipt"]))
        text = wellfriendpdf.open(reopened.bytes()).page(1).text
        assert "Second edit" in text and "New wording" not in text


def test_cancellation_preserves_preview_and_revision():
    original = source_pdf()
    with wellfriendpdf.StoryEditSession(original) as session:
        request = draft(session)
        layout = json.loads(session.preview_json(json.dumps(request)))
        token = wellfriendpdf.RenderCancellation()
        with ThreadPoolExecutor(max_workers=1) as pool:
            pool.submit(token.cancel).result(timeout=10)
        for operation in (
            lambda: session.checkpoint_json(json.dumps(request), json.dumps(layout["receipt"]), token),
            lambda: session.undo(token),
            lambda: session.redo(token),
            lambda: session.render_page_png(1, cancellation=token),
        ):
            with pytest.raises(wellfriendpdf.WellfriendError):
                operation()
        status = json.loads(session.status_json())
        assert status["preview_receipt"] == layout["receipt"]
        assert status["history"]["undo_steps"] == 0
        assert session.bytes() == original
        with pytest.raises(wellfriendpdf.WellfriendError):
            wellfriendpdf.StoryEditSession(original, token)


def test_lifecycle_thread_transfer_and_strict_inputs():
    original = source_pdf()
    session = wellfriendpdf.StoryEditSession(original)
    with ThreadPoolExecutor(max_workers=1) as pool:
        status = json.loads(pool.submit(session.status_json).result(timeout=10))
    assert status["revision_sha256"] == hashlib.sha256(original).hexdigest()
    assert command(session, "source_model", page=1)["logical_text"] == "OLD"
    assert len(command(session, "pages")) == 1
    assert not session.undo() and not session.redo()
    for invalid in ('{"op":"unknown"}', '{"op":"undo","op":"redo"}',
                    '{"op":"status","ignore_policy":true}', '{"op":"checkpoint"}'):
        with pytest.raises(wellfriendpdf.WellfriendError):
            session.command_json(invalid)
    with pytest.raises(ValueError):
        session.preview_json('{}, "op":"undo"')
    with pytest.raises(wellfriendpdf.WellfriendError):
        session.preview_json('{"story_id":"first","story_id":"second"}')
    for dpi in (0, 301):
        with pytest.raises(wellfriendpdf.WellfriendError):
            session.render_page_png(1, dpi=dpi)
    assert session.bytes() == original
    session.close()
    session.close()
    with pytest.raises(wellfriendpdf.WellfriendError):
        session.status_json()
    with pytest.raises(wellfriendpdf.WellfriendError):
        session.bytes()
    with pytest.raises(ValueError):
        wellfriendpdf.StoryEditSession(b"")
    with pytest.raises(TypeError):
        wellfriendpdf.StoryEditSession(bytearray(original))


def test_exact_password_bytes_are_borrowed_and_not_retained_for_plain_input():
    original = source_pdf()
    with wellfriendpdf.StoryEditSession(original, password=b"\xff\x00a") as session:
        security = json.loads(session.status_json())["source_security"]
        assert security == {
            "source_input_sha256": hashlib.sha256(original).hexdigest(),
            "source_was_encrypted": False,
            "working_copy_decrypted": False,
            "password_retained": False,
            "authenticated_as_owner": False,
            "permissions": None,
            "modification_permitted": True,
        }


def test_context_manager_does_not_suppress_body_exception():
    session = wellfriendpdf.StoryEditSession(source_pdf())
    with pytest.raises(LookupError, match="caller failure"):
        with session:
            raise LookupError("caller failure")
    with pytest.raises(wellfriendpdf.WellfriendError):
        session.status_json()
