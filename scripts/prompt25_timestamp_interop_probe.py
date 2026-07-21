"""Generate independent Prompt 25 RFC 3161 timestamp interoperability evidence."""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

from asn1crypto import cms, keys, x509 as asn1_x509
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
from pyhanko.sign.timestamps import DummyTimeStamper
from pyhanko.sign.validation.generic_cms import validate_tst_signed_data
from pyhanko.sign.validation.status import TimestampSignatureStatus
from pyhanko_certvalidator import ValidationContext
from pyhanko_certvalidator.registry import SimpleCertificateStore


REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = REPO_ROOT / "target" / "prompt25-signature-ltv-edits"
INTEROP_ROOT = ARTIFACT_ROOT / "interop" / "timestamp"


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cert_name(common_name: str) -> x509.Name:
    return x509.Name(
        [
            x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Oxide Prompt25 Test"),
            x509.NameAttribute(NameOID.COMMON_NAME, common_name),
        ]
    )


def build_certificates(fixed_time: datetime) -> dict[str, bytes]:
    root_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    tsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    root_subject = cert_name("Prompt25 Interop Timestamp Root")
    tsa_subject = cert_name("Prompt25 Interop TSA")
    not_before = fixed_time - timedelta(days=30)
    not_after = fixed_time + timedelta(days=365)

    root_cert = (
        x509.CertificateBuilder()
        .subject_name(root_subject)
        .issuer_name(root_subject)
        .public_key(root_key.public_key())
        .serial_number(0x25010001)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=True, path_length=1), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(root_key.public_key()), critical=False)
        .sign(root_key, hashes.SHA256())
    )

    root_ski = root_cert.extensions.get_extension_for_class(x509.SubjectKeyIdentifier).value
    tsa_cert = (
        x509.CertificateBuilder()
        .subject_name(tsa_subject)
        .issuer_name(root_subject)
        .public_key(tsa_key.public_key())
        .serial_number(0x25010002)
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.TIME_STAMPING]),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(tsa_key.public_key()), critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier(
                key_identifier=root_ski.digest,
                authority_cert_issuer=None,
                authority_cert_serial_number=None,
            ),
            critical=False,
        )
        .sign(root_key, hashes.SHA256())
    )

    return {
        "root_der": root_cert.public_bytes(serialization.Encoding.DER),
        "tsa_der": tsa_cert.public_bytes(serialization.Encoding.DER),
        "tsa_key_der": tsa_key.private_bytes(
            serialization.Encoding.DER,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        ),
    }


async def validate_with_pyhanko(token_der: bytes, root_der: bytes, signature_value: bytes) -> dict:
    token = cms.ContentInfo.load(token_der)
    signed_data = token["content"]
    digest = hashlib.sha256(signature_value).digest()
    root = asn1_x509.Certificate.load(root_der)
    vc = ValidationContext(trust_roots=[root], allow_fetching=False)
    kwargs = await validate_tst_signed_data(
        signed_data,
        vc,
        lambda md_algorithm: digest
        if md_algorithm in ("sha256", "sha-256")
        else hashlib.new(md_algorithm.replace("-", ""), signature_value).digest(),
    )
    status = TimestampSignatureStatus(**kwargs)
    return {
        "tool": "pyHanko",
        "valid": bool(status.valid),
        "trusted": bool(status.trusted),
        "intact": bool(status.intact),
        "summary": status.summary(),
        "timestamp": status.timestamp.isoformat() if status.timestamp else None,
        "signing_cert_sha256": sha256_hex(status.signing_cert.dump()) if status.signing_cert else None,
    }


def run_oxide_cli(token_path: Path, sig_path: Path, root_path: Path) -> dict:
    cmd = [
        "cargo",
        "run",
        "-p",
        "oxide-cli",
        "--",
        "timestamp-verify",
        "--json",
        "--signature-value",
        str(sig_path),
        "--trust-anchor",
        str(root_path),
        str(token_path),
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
    stdout = proc.stdout.strip()
    parsed = None
    if stdout:
        try:
            parsed = json.loads(stdout)
        except json.JSONDecodeError:
            parsed = None
    return {
        "tool": "oxide-cli",
        "command": cmd,
        "exit_code": proc.returncode,
        "stdout_sha256": sha256_hex(proc.stdout.encode("utf-8")),
        "stderr_tail": proc.stderr.splitlines()[-20:],
        "parsed": parsed,
    }


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default=str(ARTIFACT_ROOT / "timestamp-interoperability-prompt25.json"))
    args = parser.parse_args()

    INTEROP_ROOT.mkdir(parents=True, exist_ok=True)
    fixed_time = datetime(2026, 7, 21, 0, 0, tzinfo=timezone.utc)
    certs = build_certificates(fixed_time)
    root = asn1_x509.Certificate.load(certs["root_der"])
    tsa_cert = asn1_x509.Certificate.load(certs["tsa_der"])
    tsa_key = keys.PrivateKeyInfo.load(certs["tsa_key_der"])

    store = SimpleCertificateStore()
    store.register(root)
    timestamper = DummyTimeStamper(
        tsa_cert,
        tsa_key,
        certs_to_embed=store,
        fixed_dt=fixed_time,
        include_nonce=True,
    )

    signature_value = b"prompt25 independent timestamp interoperability signature value"
    message_digest = hashlib.sha256(signature_value).digest()
    token = await timestamper.async_timestamp(message_digest, "sha256")
    token_der = token.dump()
    wrong_signature_value = signature_value + b"!"

    root_path = INTEROP_ROOT / "tsa-root.der"
    tsa_path = INTEROP_ROOT / "tsa-leaf.der"
    token_path = INTEROP_ROOT / "pyhanko-rfc3161-token.der"
    sig_path = INTEROP_ROOT / "signature-value.bin"
    wrong_sig_path = INTEROP_ROOT / "wrong-signature-value.bin"
    for path, data in [
        (root_path, certs["root_der"]),
        (tsa_path, certs["tsa_der"]),
        (token_path, token_der),
        (sig_path, signature_value),
        (wrong_sig_path, wrong_signature_value),
    ]:
        path.write_bytes(data)

    pyhanko_valid = await validate_with_pyhanko(token_der, certs["root_der"], signature_value)
    try:
        pyhanko_wrong = await validate_with_pyhanko(token_der, certs["root_der"], wrong_signature_value)
    except Exception as exc:
        pyhanko_wrong = {
            "tool": "pyHanko",
            "valid": False,
            "trusted": False,
            "intact": False,
            "error": f"{type(exc).__name__}: {exc}",
        }

    oxide_valid = run_oxide_cli(token_path, sig_path, root_path)
    oxide_wrong = run_oxide_cli(token_path, wrong_sig_path, root_path)

    payload = {
        "schema": "prompt25.timestamp-interoperability.v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "fixture_dir": str(INTEROP_ROOT),
        "fixed_gen_time": fixed_time.isoformat(),
        "inputs": {
            "token_sha256": sha256_hex(token_der),
            "signature_value_sha256": sha256_hex(signature_value),
            "wrong_signature_value_sha256": sha256_hex(wrong_signature_value),
            "tsa_root_sha256": sha256_hex(certs["root_der"]),
            "tsa_leaf_sha256": sha256_hex(certs["tsa_der"]),
        },
        "independent_generation": {
            "tool": "pyHanko DummyTimeStamper",
            "expected_profile": "RFC3161 TimeStampToken with TSTInfo SHA-256 messageImprint",
            "private_key_written": False,
        },
        "independent_validation": {
            "valid_imprint": pyhanko_valid,
            "wrong_imprint": pyhanko_wrong,
        },
        "oxide_validation": {
            "valid_imprint": oxide_valid,
            "wrong_imprint": oxide_wrong,
        },
        "result": "passed"
        if (
            pyhanko_valid.get("trusted") is True
            and pyhanko_wrong.get("intact") is False
            and oxide_valid.get("exit_code") == 0
            and oxide_valid.get("parsed", {}).get("status") == "valid"
            and oxide_wrong.get("parsed", {}).get("status") != "valid"
        )
        else "failed",
    }

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({"output": str(output), "result": payload["result"]}, indent=2))
    return 0 if payload["result"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
