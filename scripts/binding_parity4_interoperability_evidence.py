#!/usr/bin/env python3
"""Generate Signature Validation interoperability evidence from installed local tools.

This script intentionally records unavailable tools as unsupported evidence,
not as passes. The positive rows come from independent Python libraries that
parse or validate the committed Signature Validation fixtures without calling WellfriendPdf.
"""

from __future__ import annotations

import datetime as dt
import hashlib
import importlib.metadata
import json
import shutil
import sys
import traceback
from pathlib import Path
from typing import Any, Callable

from asn1crypto import cms, pem, x509 as asn1_x509
from cryptography import x509
from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa
from cryptography.x509 import ocsp
from cryptography.x509.oid import ExtendedKeyUsageOID
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.validation import validate_pdf_signature
from pyhanko_certvalidator import ValidationContext


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "crates" / "engine" / "tests" / "fixtures"
ARTIFACT_ROOT = ROOT / "target" / "signature_validation-signature-validation"
DOCS = ROOT / "docs"
SCHEMA = "signature_validation.interoperability.v1"
VALIDATION_TIME = dt.datetime(2026, 7, 20, 12, 0, 0, tzinfo=dt.timezone.utc)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def package_version(name: str) -> str:
    try:
        return importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        return "unavailable"


def iso(value: Any) -> str | None:
    if value is None:
        return None
    if hasattr(value, "isoformat"):
        return value.isoformat()
    return str(value)


def load_der_cert(name: str) -> x509.Certificate:
    return x509.load_der_x509_certificate((FIXTURES / name).read_bytes())


def load_pem_cert(name: str) -> x509.Certificate:
    return x509.load_pem_x509_certificate((FIXTURES / name).read_bytes())


def load_pyhanko_signer_cert(pdf_name: str) -> x509.Certificate:
    with (FIXTURES / pdf_name).open("rb") as handle:
        reader = PdfFileReader(handle)
        embedded = reader.embedded_signatures[0]
        return x509.load_der_x509_certificate(embedded.signer_cert.dump())


def verify_sig(public_key: Any, signature: bytes, data: bytes, hash_algorithm: Any) -> None:
    if isinstance(public_key, rsa.RSAPublicKey):
        public_key.verify(signature, data, padding.PKCS1v15(), hash_algorithm)
    elif isinstance(public_key, ec.EllipticCurvePublicKey):
        public_key.verify(signature, data, ec.ECDSA(hash_algorithm))
    else:
        raise AssertionError(f"unsupported public key type {type(public_key).__name__}")


def verify_cert_signature(cert: x509.Certificate, issuer: x509.Certificate) -> None:
    verify_sig(
        issuer.public_key(),
        cert.signature,
        cert.tbs_certificate_bytes,
        cert.signature_hash_algorithm,
    )


def row(
    tool: str,
    version: str,
    operation: str,
    fixture: str,
    expected: str,
    actual: str,
    classification: str,
    details: dict[str, Any] | None = None,
    unsupported_reason: str | None = None,
) -> dict[str, Any]:
    fixture_path = FIXTURES / fixture if fixture else None
    payload: dict[str, Any] = {
        "tool": tool,
        "version": version,
        "operation": operation,
        "fixture": fixture,
        "fixture_sha256": sha256(fixture_path) if fixture_path and fixture_path.exists() else None,
        "validation_time": VALIDATION_TIME.isoformat(),
        "expected_result": expected,
        "actual_result": actual,
        "result_classification": classification,
        "details": details or {},
        "unsupported_reason": unsupported_reason,
    }
    return payload


def checked(
    rows: list[dict[str, Any]],
    tool: str,
    version: str,
    operation: str,
    fixture: str,
    expected: str,
    fn: Callable[[], dict[str, Any]],
) -> None:
    try:
        details = fn()
        rows.append(row(tool, version, operation, fixture, expected, "matched", "pass", details))
    except Exception as exc:  # pragma: no cover - exercised only on interop failures
        rows.append(
            row(
                tool,
                version,
                operation,
                fixture,
                expected,
                f"{type(exc).__name__}: {exc}",
                "fail",
                {"traceback_tail": traceback.format_exc().splitlines()[-8:]},
            )
        )


def pades_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def validate_good() -> dict[str, Any]:
        _, _, der = pem.unarmor((FIXTURES / "sig_ecdsa_p256_cert.pem").read_bytes())
        trust_root = asn1_x509.Certificate.load(der)
        vc = ValidationContext(
            trust_roots=[trust_root],
            moment=VALIDATION_TIME,
            allow_fetching=False,
            revocation_mode="soft-fail",
        )
        with (FIXTURES / "sig_pades_b_ecdsa_p256.pdf").open("rb") as handle:
            reader = PdfFileReader(handle)
            status = validate_pdf_signature(
                reader.embedded_signatures[0],
                signer_validation_context=vc,
                skip_diff=True,
            )
        assert bool(status.bottom_line)
        assert bool(status.intact)
        assert bool(status.valid)
        assert bool(status.trusted)
        assert not bool(status.revoked)
        return {
            "embedded_signatures": 1,
            "status_class": type(status).__name__,
            "summary": status.summary(),
            "bottom_line": bool(status.bottom_line),
            "intact": bool(status.intact),
            "valid": bool(status.valid),
            "trusted": bool(status.trusted),
            "revoked": bool(status.revoked),
        }

    checked(
        rows,
        "pyHanko",
        package_version("pyHanko"),
        "Validate independent PAdES/PDF detached signature fixture",
        "sig_pades_b_ecdsa_p256.pdf",
        "INTACT:TRUSTED,UNTOUCHED",
        validate_good,
    )
    return rows


def cms_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def parse_signed_data() -> dict[str, Any]:
        with (FIXTURES / "sig_pades_b_ecdsa_p256.pdf").open("rb") as handle:
            reader = PdfFileReader(handle)
            embedded = reader.embedded_signatures[0]
            signed_data = embedded.signed_data
            content_info = cms.ContentInfo.load(embedded.pkcs7_content.original_bytes)
            signer_info = embedded.signer_info
        signed_attrs = signer_info["signed_attrs"]
        attribute_oids = [attr["type"].dotted for attr in signed_attrs]
        assert content_info["content_type"].native == "signed_data"
        assert signed_data["encap_content_info"]["content_type"].native == "data"
        assert "1.2.840.113549.1.9.3" in attribute_oids
        assert "1.2.840.113549.1.9.4" in attribute_oids
        return {
            "content_type": content_info["content_type"].native,
            "encap_content_type": signed_data["encap_content_info"]["content_type"].native,
            "signer_infos": len(signed_data["signer_infos"]),
            "certificates": len(signed_data["certificates"]),
            "signed_attribute_oids": sorted(attribute_oids),
            "signature_algorithm": signer_info["signature_algorithm"]["algorithm"].native,
            "digest_algorithm": signer_info["digest_algorithm"]["algorithm"].native,
        }

    checked(
        rows,
        "pyHanko/asn1crypto",
        f"pyHanko {package_version('pyHanko')}; asn1crypto {package_version('asn1crypto')}",
        "Parse CMS ContentInfo/SignedData and signed attributes from PDF signature",
        "sig_pades_b_ecdsa_p256.pdf",
        "signed_data with one signer and required contentType/messageDigest attributes",
        parse_signed_data,
    )
    return rows


def pkix_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def verify_aia_chain() -> dict[str, Any]:
        root = load_der_cert("aia_root.der")
        intermediate = load_der_cert("aia_intermediate.der")
        leaf = load_pyhanko_signer_cert("sig_aia_leaf_only.pdf")
        verify_cert_signature(intermediate, root)
        verify_cert_signature(leaf, intermediate)
        assert leaf.not_valid_before_utc <= VALIDATION_TIME <= leaf.not_valid_after_utc
        assert intermediate.not_valid_before_utc <= VALIDATION_TIME <= intermediate.not_valid_after_utc
        assert root.not_valid_before_utc <= VALIDATION_TIME <= root.not_valid_after_utc
        basic = intermediate.extensions.get_extension_for_class(x509.BasicConstraints).value
        key_usage = intermediate.extensions.get_extension_for_class(x509.KeyUsage).value
        assert basic.ca
        assert key_usage.key_cert_sign
        return {
            "chain_subjects": [
                leaf.subject.rfc4514_string(),
                intermediate.subject.rfc4514_string(),
                root.subject.rfc4514_string(),
            ],
            "leaf_serial": leaf.serial_number,
            "intermediate_ca": basic.ca,
            "intermediate_key_cert_sign": key_usage.key_cert_sign,
        }

    checked(
        rows,
        "cryptography",
        package_version("cryptography"),
        "Verify Signature Validation AIA fixture certificate signatures and CA constraints",
        "sig_aia_leaf_only.pdf",
        "leaf -> intermediate -> root signatures and CA constraints valid",
        verify_aia_chain,
    )
    return rows


def ocsp_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def verify_issuer_response() -> dict[str, Any]:
        response = ocsp.load_der_ocsp_response((FIXTURES / "aia_leaf_good.ocsp").read_bytes())
        issuer = load_der_cert("aia_intermediate.der")
        leaf = load_pyhanko_signer_cert("sig_aia_leaf_only.pdf")
        assert response.response_status == ocsp.OCSPResponseStatus.SUCCESSFUL
        assert response.certificate_status == ocsp.OCSPCertStatus.GOOD
        assert response.serial_number == leaf.serial_number
        verify_sig(
            issuer.public_key(),
            response.signature,
            response.tbs_response_bytes,
            response.signature_hash_algorithm,
        )
        assert response.this_update_utc <= VALIDATION_TIME <= response.next_update_utc
        return {
            "status": response.certificate_status.name,
            "serial": response.serial_number,
            "responder_name": response.responder_name.rfc4514_string(),
            "this_update": iso(response.this_update_utc),
            "next_update": iso(response.next_update_utc),
            "produced_at": iso(response.produced_at_utc),
            "signature_hash": response.signature_hash_algorithm.name,
        }

    def verify_delegated_response() -> dict[str, Any]:
        response = ocsp.load_der_ocsp_response(
            (FIXTURES / "aia_leaf_delegated_good.ocsp").read_bytes()
        )
        issuer = load_der_cert("aia_intermediate.der")
        assert len(response.certificates) == 1
        responder = response.certificates[0]
        verify_cert_signature(responder, issuer)
        eku = responder.extensions.get_extension_for_class(x509.ExtendedKeyUsage).value
        assert ExtendedKeyUsageOID.OCSP_SIGNING in eku
        verify_sig(
            responder.public_key(),
            response.signature,
            response.tbs_response_bytes,
            response.signature_hash_algorithm,
        )
        assert response.certificate_status == ocsp.OCSPCertStatus.GOOD
        return {
            "status": response.certificate_status.name,
            "delegated_responder": responder.subject.rfc4514_string(),
            "ocsp_signing_eku": True,
            "this_update": iso(response.this_update_utc),
            "next_update": iso(response.next_update_utc),
        }

    checked(
        rows,
        "cryptography",
        package_version("cryptography"),
        "Parse and verify issuer-signed OCSP good response",
        "aia_leaf_good.ocsp",
        "successful good response signed by certificate issuer and fresh at validation time",
        verify_issuer_response,
    )
    checked(
        rows,
        "cryptography",
        package_version("cryptography"),
        "Parse and verify delegated OCSP responder response",
        "aia_leaf_delegated_good.ocsp",
        "successful good response signed by delegated responder with OCSPSigning EKU",
        verify_delegated_response,
    )
    return rows


def crl_rows() -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def verify_base_crl() -> dict[str, Any]:
        crl = x509.load_der_x509_crl((FIXTURES / "aia_leaf_revoked.crl").read_bytes())
        issuer = load_der_cert("aia_intermediate.der")
        leaf = load_pyhanko_signer_cert("sig_aia_leaf_only.pdf")
        verify_sig(
            issuer.public_key(),
            crl.signature,
            crl.tbs_certlist_bytes,
            crl.signature_hash_algorithm,
        )
        assert crl.last_update_utc <= VALIDATION_TIME <= crl.next_update_utc
        revoked = crl.get_revoked_certificate_by_serial_number(leaf.serial_number)
        assert revoked is not None
        return {
            "issuer": crl.issuer.rfc4514_string(),
            "revoked_serial": revoked.serial_number,
            "entry_count": len(list(crl)),
            "this_update": iso(crl.last_update_utc),
            "next_update": iso(crl.next_update_utc),
            "signature_hash": crl.signature_hash_algorithm.name,
        }

    def verify_delta_crl() -> dict[str, Any]:
        base = x509.load_der_x509_crl((FIXTURES / "aia_leaf_delta_base_good.crl").read_bytes())
        delta = x509.load_der_x509_crl((FIXTURES / "aia_leaf_delta_revoked.crl").read_bytes())
        issuer = load_der_cert("aia_intermediate.der")
        leaf = load_pyhanko_signer_cert("sig_aia_leaf_only.pdf")
        for candidate in (base, delta):
            verify_sig(
                issuer.public_key(),
                candidate.signature,
                candidate.tbs_certlist_bytes,
                candidate.signature_hash_algorithm,
            )
            assert candidate.last_update_utc <= VALIDATION_TIME <= candidate.next_update_utc
        base_number = base.extensions.get_extension_for_class(x509.CRLNumber).value.crl_number
        delta_number = delta.extensions.get_extension_for_class(x509.CRLNumber).value.crl_number
        delta_base_number = delta.extensions.get_extension_for_class(
            x509.DeltaCRLIndicator
        ).value.crl_number
        revoked = delta.get_revoked_certificate_by_serial_number(leaf.serial_number)
        assert revoked is not None
        assert delta_base_number == base_number
        assert delta_number > base_number
        return {
            "base_number": base_number,
            "delta_number": delta_number,
            "delta_base_number": delta_base_number,
            "delta_revoked_serial": revoked.serial_number,
        }

    checked(
        rows,
        "cryptography",
        package_version("cryptography"),
        "Parse and verify base CRL and revoked serial",
        "aia_leaf_revoked.crl",
        "valid CRL signature and matching revoked leaf serial",
        verify_base_crl,
    )
    checked(
        rows,
        "cryptography",
        package_version("cryptography"),
        "Parse and verify delta CRL relationship",
        "aia_leaf_delta_revoked.crl",
        "valid base/delta signatures, delta indicator matches base number, delta revokes leaf",
        verify_delta_crl,
    )
    return rows


def support_matrix() -> list[dict[str, Any]]:
    tools = [
        ("openssl", ["openssl", "version"], "external CMS/PKIX/OCSP/CRL CLI"),
        ("mvn", ["mvn", "-version"], "Maven Java package tests"),
        ("gradle", ["gradle", "--version"], "Gradle Java package tests"),
        ("wasm-pack", ["wasm-pack", "--version"], "WASM package smoke"),
    ]
    rows: list[dict[str, Any]] = []
    for name, command, purpose in tools:
        executable = shutil.which(command[0])
        rows.append(
            {
                "tool": name,
                "purpose": purpose,
                "available": executable is not None,
                "path": executable,
                "classification": "available" if executable else "unsupported_unavailable_not_counted_as_pass",
            }
        )
    rows.extend(
        [
            {
                "tool": "cryptography",
                "purpose": "independent PKIX/OCSP/CRL parsing and signature verification",
                "available": True,
                "version": package_version("cryptography"),
                "classification": "available",
            },
            {
                "tool": "pyHanko",
                "purpose": "independent PDF/CMS/PAdES validation",
                "available": True,
                "version": package_version("pyHanko"),
                "classification": "available",
            },
            {
                "tool": "asn1crypto",
                "purpose": "independent ASN.1/CMS inspection",
                "available": True,
                "version": package_version("asn1crypto"),
                "classification": "available",
            },
        ]
    )
    return rows


def write_json(name: str, payload: Any) -> None:
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    (ARTIFACT_ROOT / name).write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def artifact_payload(category: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "category": category,
        "generated_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "validation_time": VALIDATION_TIME.isoformat(),
        "workspace": "E:/wellpdfsdk",
        "rows": rows,
        "summary": {
            "pass": sum(1 for row in rows if row.get("result_classification") == "pass"),
            "fail": sum(1 for row in rows if row.get("result_classification") == "fail"),
            "unsupported": sum(
                1
                for row in rows
                if "unsupported" in str(row.get("result_classification", ""))
                or "unsupported" in str(row.get("classification", ""))
            ),
        },
    }


def write_doc(all_payloads: dict[str, dict[str, Any]]) -> None:
    DOCS.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Signature Validation Interoperability",
        "",
        f"Schema: `{SCHEMA}`",
        "",
        "This evidence was generated from local, independently implemented libraries and",
        "does not count unavailable tools as passed. OpenSSL, Maven, Gradle, and",
        "wasm-pack availability is recorded separately in the support matrix.",
        "",
        f"Validation time: `{VALIDATION_TIME.isoformat()}`.",
        "",
    ]
    for name, payload in all_payloads.items():
        lines.append(f"## {name.upper()}")
        rows = payload["rows"]
        if not rows:
            lines.append("")
            lines.append("No rows were generated.")
            lines.append("")
            continue
        for item in rows:
            classification = item.get("result_classification") or item.get("classification")
            lines.append(
                f"- `{classification}`: {item.get('tool')} - {item.get('operation') or item.get('purpose')}"
            )
        lines.append("")
    (DOCS / "signature_validation_interoperability.md").write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    payloads = {
        "cms": artifact_payload("cms", cms_rows()),
        "pkix": artifact_payload("pkix", pkix_rows()),
        "ocsp": artifact_payload("ocsp", ocsp_rows()),
        "crl": artifact_payload("crl", crl_rows()),
        "pades": artifact_payload("pades", pades_rows()),
    }
    support = {
        "schema": SCHEMA,
        "generated_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "rows": support_matrix(),
    }
    write_json("cms-interoperability-signature_validation.json", payloads["cms"])
    write_json("pkix-interoperability-signature_validation.json", payloads["pkix"])
    write_json("ocsp-interoperability-signature_validation.json", payloads["ocsp"])
    write_json("crl-interoperability-signature_validation.json", payloads["crl"])
    write_json("pades-interoperability-signature_validation.json", payloads["pades"])
    write_json("independent-tool-support-matrix-signature_validation.json", support)
    write_doc(payloads | {"tools": support})
    failed = sum(payload["summary"]["fail"] for payload in payloads.values())
    print(json.dumps({"schema": SCHEMA, "failed": failed, "artifacts": list(payloads)}, indent=2))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
