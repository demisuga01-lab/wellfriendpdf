"""Local-AI OCR backend example for the Wellfriend Python binding.

This is a runnable template for the Python OCR seam:

    recognize(image_bytes: bytes, info: dict) -> list[dict]

Replace `_recognize_with_your_model` with your local model call. The default
implementation uses `pytesseract` when it is installed so the example can run
end-to-end without fabricating OCR output.
"""

from __future__ import annotations

from pathlib import Path
import sys
from typing import Any

import wellfriendpdf


class LocalAiOcrBackend:
    """A Python OCR backend object accepted by `wellfriendpdf.Document.to_markdown`."""

    name = "local-ai-python-reference"
    version = "template-1"

    def max_concurrency(self) -> int:
        # The current PyO3 wrapper advertises max_concurrency=1 to the Rust
        # engine because calls enter Python under the GIL. A model that releases
        # the GIL still works, but widening concurrency requires a separate
        # interpreter/process pool owned by the integrator.
        return 1

    def recognize(self, image_bytes: bytes, info: dict[str, Any]) -> list[dict[str, Any]]:
        # YOUR MODEL HERE:
        #
        # image_bytes is raw uint8 grayscale, row-major, top-left origin.
        # info contains width, height, dpi, languages, and psm.
        #
        # Return one dict per word:
        # {"text": str, "bbox": [x0, y0, x1, y1], "confidence": 0..1, "line_id": int}
        return self._recognize_with_your_model(image_bytes, info)

    def _recognize_with_your_model(
        self, image_bytes: bytes, info: dict[str, Any]
    ) -> list[dict[str, Any]]:
        # Default runnable path: real OCR through pytesseract, if available.
        return self._recognize_with_pytesseract(image_bytes, info)

    def _recognize_with_pytesseract(
        self, image_bytes: bytes, info: dict[str, Any]
    ) -> list[dict[str, Any]]:
        try:
            from PIL import Image
            import pytesseract
        except ImportError as exc:
            raise RuntimeError(
                "Install pillow+pytesseract for the default example path, or replace "
                "_recognize_with_your_model with your local model call."
            ) from exc

        width = int(info["width"])
        height = int(info["height"])
        image = Image.frombytes("L", (width, height), image_bytes)
        languages = info.get("languages") or ["eng"]
        lang = "+".join(str(lang) for lang in languages)
        psm = info.get("psm")
        config = f"--psm {int(psm)}" if psm is not None else ""

        data = pytesseract.image_to_data(
            image,
            lang=lang,
            config=config,
            output_type=pytesseract.Output.DICT,
        )

        words: list[dict[str, Any]] = []
        line_ids: dict[tuple[int, int, int], int] = {}
        for i, text in enumerate(data.get("text", [])):
            text = str(text).strip()
            if not text:
                continue
            try:
                confidence = float(data["conf"][i])
            except (ValueError, TypeError):
                confidence = -1.0
            if confidence < 0.0:
                continue

            left = float(data["left"][i])
            top = float(data["top"][i])
            word_width = float(data["width"][i])
            word_height = float(data["height"][i])
            key = (
                int(data.get("block_num", [0])[i]),
                int(data.get("par_num", [0])[i]),
                int(data.get("line_num", [0])[i]),
            )
            line_id = line_ids.setdefault(key, len(line_ids))
            words.append(
                {
                    "text": text,
                    "bbox": [left, top, left + word_width, top + word_height],
                    "confidence": max(0.0, min(confidence / 100.0, 1.0)),
                    "line_id": line_id,
                }
            )
        return words


def main() -> None:
    root = Path(__file__).resolve().parents[3]
    default_pdf = root / "extraction-benchmark" / "corpus" / "invoice_scanned.pdf"
    pdf = Path(sys.argv[1]) if len(sys.argv) > 1 else default_pdf
    if not pdf.exists():
        raise SystemExit(f"PDF fixture not found: {pdf}")

    doc = wellfriendpdf.open(pdf)
    markdown = doc.to_markdown(ocr=LocalAiOcrBackend(), ocr_lang="eng", ocr_dpi=300)
    print(markdown)


if __name__ == "__main__":
    main()
