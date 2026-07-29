"""Generate Pades LTV PAdES B-T/B-LT interoperability evidence."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import subprocess
from datetime import datetime, timedelta, timezone
from pathlib import Path

from asn1crypto import keys, x509 as asn1_x509
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign import signers, validation
from pyhanko.sign.fields import SigFieldSpec, SigSeedSubFilter, append_signature_field
from pyhanko.sign.timestamps import DummyTimeStamper
from pyhanko.sign.validation import validate_pdf_signature
from pyhanko.sign.validation.settings import KeyUsageConstraints
from pyhanko_certvalidator import ValidationContext
from pyhanko_certvalidator.registry import SimpleCertificateStore


REPO_ROOT = Path(__file__).resolve().parents[1]
FIXTURES = REPO_ROOT / "crates" / "engine" / "tests" / "fixtures"
ARTIFACT_ROOT = REPO_ROOT / "target" / "pades_ltv-signature-ltv-edits"
INTEROP_ROOT = ARTIFACT_ROOT / "interop" / "pades-ltv"
FIXED_TIME = datetime(2026, 7, 21, 0, 0, tzinfo=timezone.utc)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def name(common_name: str) -> x509.Name:
    return x509.Name(
        [
            x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Wellfriend PadesLTV LTV Test"),
            x509.NameAttribute(NameOID.COMMON_NAME, common_name),
        ]
    )


def key_usage_ca() -> x509.KeyUsage:
    return x509.KeyUsage(
        digital_signature=False,
        content_commitment=False,
        key_encipherment=False,
        data_encipherment=False,
        key_agreement=False,
        key_cert_sign=True,
        crl_sign=True,
        encipher_only=False,
        decipher_only=False,
    )


def key_usage_signing(*, crl_sign: bool = False) -> x509.KeyUsage:
    return x509.KeyUsage(
        digital_signature=True,
        content_commitment=False,
        key_encipherment=False,
        data_encipherment=False,
        key_agreement=False,
        key_cert_sign=False,
        crl_sign=crl_sign,
        encipher_only=False,
        decipher_only=False,
    )


def build_pki() -> dict[str, bytes]:
    root_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    intermediate_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    signer_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    tsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    not_before = FIXED_TIME - timedelta(days=30)
    not_after = FIXED_TIME + timedelta(days=365)
    root_subject = name("PadesLTV PAdES Root")
    intermediate_subject = name("PadesLTV PAdES Intermediate")
    signer_subject = name("PadesLTV PAdES Signer")
    tsa_subject = name("PadesLTV PAdES TSA")

    root = (
        x509.CertificateBuilder()
        .subject_name(root_subject)
        .issuer_name(root_subject)
        .public_key(root_key.public_key())
        .serial_number(0x25A00001)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=True, path_length=2), critical=True)
        .add_extension(key_usage_ca(), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(root_key.public_key()), critical=False)
        .sign(root_key, hashes.SHA256())
    )

    intermediate = (
        x509.CertificateBuilder()
        .subject_name(intermediate_subject)
        .issuer_name(root_subject)
        .public_key(intermediate_key.public_key())
        .serial_number(0x25A00002)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .add_extension(key_usage_ca(), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(intermediate_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(root_key.public_key()), critical=False)
        .sign(root_key, hashes.SHA256())
    )

    signer = (
        x509.CertificateBuilder()
        .subject_name(signer_subject)
        .issuer_name(intermediate_subject)
        .public_key(signer_key.public_key())
        .serial_number(0x25A00003)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(key_usage_signing(), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(signer_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .sign(intermediate_key, hashes.SHA256())
    )

    tsa = (
        x509.CertificateBuilder()
        .subject_name(tsa_subject)
        .issuer_name(root_subject)
        .public_key(tsa_key.public_key())
        .serial_number(0x25A00004)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(key_usage_signing(), critical=True)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.TIME_STAMPING]), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(tsa_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(root_key.public_key()), critical=False)
        .sign(root_key, hashes.SHA256())
    )

    root_crl = (
        x509.CertificateRevocationListBuilder()
        .issuer_name(root.subject)
        .last_update(FIXED_TIME - timedelta(days=1))
        .next_update(FIXED_TIME + timedelta(days=90))
        .add_extension(x509.CRLNumber(1), critical=False)
        .sign(root_key, hashes.SHA256())
    )
    intermediate_crl = (
        x509.CertificateRevocationListBuilder()
        .issuer_name(intermediate.subject)
        .last_update(FIXED_TIME - timedelta(days=1))
        .next_update(FIXED_TIME + timedelta(days=90))
        .add_extension(x509.CRLNumber(1), critical=False)
        .sign(intermediate_key, hashes.SHA256())
    )

    return {
        "root_der": root.public_bytes(serialization.Encoding.DER),
        "intermediate_der": intermediate.public_bytes(serialization.Encoding.DER),
        "signer_der": signer.public_bytes(serialization.Encoding.DER),
        "signer_key_der": signer_key.private_bytes(
            serialization.Encoding.DER,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        ),
        "tsa_der": tsa.public_bytes(serialization.Encoding.DER),
        "tsa_key_der": tsa_key.private_bytes(
            serialization.Encoding.DER,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        ),
        "root_crl_der": root_crl.public_bytes(serialization.Encoding.DER),
        "intermediate_crl_der": intermediate_crl.public_bytes(serialization.Encoding.DER),
    }


def validation_context(pki: dict[str, bytes], *, hard_fail: bool) -> ValidationContext:
    return ValidationContext(
        trust_roots=[asn1_x509.Certificate.load(pki["root_der"])],
        other_certs=[
            asn1_x509.Certificate.load(pki["intermediate_der"]),
            asn1_x509.Certificate.load(pki["tsa_der"]),
        ],
        crls=[pki["root_crl_der"], pki["intermediate_crl_der"]],
        moment=FIXED_TIME,
        best_signature_time=FIXED_TIME,
        allow_fetching=False,
        revocation_mode="hard-fail" if hard_fail else "soft-fail",
    )


def make_pdf(pki: dict[str, bytes]) -> tuple[bytes, bytes]:
    store = SimpleCertificateStore()
    store.register(asn1_x509.Certificate.load(pki["intermediate_der"]))
    store.register(asn1_x509.Certificate.load(pki["root_der"]))
    signer = signers.SimpleSigner(
        signing_cert=asn1_x509.Certificate.load(pki["signer_der"]),
        signing_key=keys.PrivateKeyInfo.load(pki["signer_key_der"]),
        cert_registry=store,
        embed_roots=True,
    )

    ts_store = SimpleCertificateStore()
    ts_store.register(asn1_x509.Certificate.load(pki["root_der"]))
    timestamper = DummyTimeStamper(
        asn1_x509.Certificate.load(pki["tsa_der"]),
        keys.PrivateKeyInfo.load(pki["tsa_key_der"]),
        certs_to_embed=ts_store,
        fixed_dt=FIXED_TIME,
        include_nonce=True,
    )

    writer = IncrementalPdfFileWriter(io.BytesIO((FIXTURES / "minimal.pdf").read_bytes()))
    append_signature_field(writer, SigFieldSpec(sig_field_name="PadesLTVPadesBT"))
    signed = io.BytesIO()
    signers.sign_pdf(
        writer,
        signers.PdfSignatureMetadata(
            field_name="PadesLTVPadesBT",
            md_algorithm="sha256",
            subfilter=SigSeedSubFilter.PADES,
        ),
        signer=signer,
        timestamper=timestamper,
        output=signed,
    )
    bt_pdf = signed.getvalue()

    reader = PdfFileReader(io.BytesIO(bt_pdf))
    embedded = reader.embedded_signatures[0]
    blt = validation.add_validation_info(
        embedded,
        validation_context(pki, hard_fail=True),
        output=io.BytesIO(),
        force_write=True,
    ).getvalue()
    return bt_pdf, blt


def pyhanko_validate(pdf_bytes: bytes, pki: dict[str, bytes]) -> dict:
    reader = PdfFileReader(io.BytesIO(pdf_bytes))
    embedded = reader.embedded_signatures[0]
    status = validate_pdf_signature(
        embedded,
        signer_validation_context=validation_context(pki, hard_fail=False),
        ts_validation_context=validation_context(pki, hard_fail=False),
        key_usage_settings=KeyUsageConstraints(key_usage={"digital_signature"}),
        skip_diff=True,
    )
    return {
        "tool": "pyHanko",
        "valid": bool(status.valid),
        "trusted": bool(status.trusted),
        "intact": bool(status.intact),
        "summary": status.summary(),
        "timestamp_valid": bool(status.timestamp_validity.valid) if status.timestamp_validity else False,
        "timestamp_trusted": bool(status.timestamp_validity.trusted) if status.timestamp_validity else False,
    }


def run_wellfriendpdf_signature_verify(pdf_path: Path, root_path: Path, intermediate_path: Path) -> dict:
    cmd = [
        "cargo",
        "run",
        "-p",
        "wellfriendpdf-cli",
        "--",
        "signature-verify",
        "--json",
        "--trust-anchor",
        str(root_path),
        "--intermediate",
        str(intermediate_path),
        "--revocation",
        "not-checked",
        str(pdf_path),
    ]
    proc = subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=300,
        check=False,
    )
    parsed = None
    if proc.stdout.strip():
        try:
            parsed = json.loads(proc.stdout)
        except json.JSONDecodeError:
            pass
    return {
        "tool": "wellfriendpdf-cli",
        "command": cmd,
        "exit_code": proc.returncode,
        "stdout_sha256": sha256_hex(proc.stdout.encode("utf-8")),
        "stderr_tail": proc.stderr.splitlines()[-20:],
        "parsed": parsed,
    }


def pades_ltv_summary(parsed: dict | None) -> dict:
    try:
        if isinstance(parsed, list):
            sig = parsed[0]
        else:
            sig = parsed["signatures"][0]
        return {
            "overall": sig.get("overall"),
            "status": sig.get("status"),
            "coverage": sig.get("coverage"),
            "timestamp_status": sig.get("pades_ltv", {}).get("signature_timestamp_status"),
            "ltv_status": sig.get("pades_ltv", {}).get("ltv_status"),
            "achieved_pades_level": sig.get("pades_ltv", {}).get("achieved_pades_level"),
            "dss_status": sig.get("pades_ltv", {}).get("dss", {}).get("status"),
            "vri_matched": sig.get("pades_ltv", {}).get("dss", {}).get("vri_matched"),
        }
    except Exception as exc:
        return {"error": f"{type(exc).__name__}: {exc}"}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default=str(ARTIFACT_ROOT / "pades-ltv-interoperability-pades_ltv.json"))
    args = parser.parse_args()

    INTEROP_ROOT.mkdir(parents=True, exist_ok=True)
    pki = build_pki()
    paths = {
        "root": INTEROP_ROOT / "pades-root.der",
        "intermediate": INTEROP_ROOT / "pades-intermediate.der",
        "signer": INTEROP_ROOT / "pades-signer.der",
        "tsa": INTEROP_ROOT / "pades-tsa.der",
        "root_crl": INTEROP_ROOT / "pades-root.crl",
        "intermediate_crl": INTEROP_ROOT / "pades-intermediate.crl",
    }
    for key, path in paths.items():
        source_key = {
            "root": "root_der",
            "intermediate": "intermediate_der",
            "signer": "signer_der",
            "tsa": "tsa_der",
            "root_crl": "root_crl_der",
            "intermediate_crl": "intermediate_crl_der",
        }[key]
        path.write_bytes(pki[source_key])

    bt_pdf, blt_pdf = make_pdf(pki)
    bt_path = INTEROP_ROOT / "pyhanko-pades-bt.pdf"
    blt_path = INTEROP_ROOT / "pyhanko-pades-blt.pdf"
    bt_path.write_bytes(bt_pdf)
    blt_path.write_bytes(blt_pdf)

    pyhanko_bt = pyhanko_validate(bt_pdf, pki)
    pyhanko_blt = pyhanko_validate(blt_pdf, pki)
    wellfriendpdf_bt = run_wellfriendpdf_signature_verify(bt_path, paths["root"], paths["intermediate"])
    wellfriendpdf_blt = run_wellfriendpdf_signature_verify(blt_path, paths["root"], paths["intermediate"])

    payload = {
        "schema": "pades_ltv.pades-ltv-interoperability.v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "fixture_dir": str(INTEROP_ROOT),
        "fixed_time": FIXED_TIME.isoformat(),
        "inputs": {
            "bt_pdf_sha256": sha256_hex(bt_pdf),
            "blt_pdf_sha256": sha256_hex(blt_pdf),
            "root_sha256": sha256_hex(pki["root_der"]),
            "intermediate_sha256": sha256_hex(pki["intermediate_der"]),
            "tsa_sha256": sha256_hex(pki["tsa_der"]),
            "root_crl_sha256": sha256_hex(pki["root_crl_der"]),
            "intermediate_crl_sha256": sha256_hex(pki["intermediate_crl_der"]),
        },
        "independent_generation": {
            "tool": "pyHanko",
            "profiles": ["PAdES B-T using DummyTimeStamper", "PAdES DSS/VRI validation-info append"],
            "private_keys_written": False,
        },
        "pyhanko_validation": {
            "bt": pyhanko_bt,
            "blt": pyhanko_blt,
        },
        "wellfriendpdf_validation": {
            "bt": {**wellfriendpdf_bt, "pades_ltv_summary": pades_ltv_summary(wellfriendpdf_bt.get("parsed"))},
            "blt": {**wellfriendpdf_blt, "pades_ltv_summary": pades_ltv_summary(wellfriendpdf_blt.get("parsed"))},
        },
    }
    bt_ok = (
        pyhanko_bt.get("trusted") is True
        and wellfriendpdf_bt.get("exit_code") == 0
        and pades_ltv_summary(wellfriendpdf_bt.get("parsed")).get("timestamp_status") == "valid"
        and pades_ltv_summary(wellfriendpdf_bt.get("parsed")).get("achieved_pades_level") in {"baseline_t", "baseline_lt"}
    )
    blt_summary = pades_ltv_summary(wellfriendpdf_blt.get("parsed"))
    blt_ok = (
        pyhanko_blt.get("trusted") is True
        and wellfriendpdf_blt.get("exit_code") in {0, 13}
        and blt_summary.get("timestamp_status") == "valid"
        and blt_summary.get("vri_matched") is True
        and blt_summary.get("achieved_pades_level") in {"baseline_lt", "baseline_l_t"}
    )
    payload["result"] = "passed" if bt_ok and blt_ok else "failed"

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({"output": str(output), "result": payload["result"]}, indent=2))
    return 0 if payload["result"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
