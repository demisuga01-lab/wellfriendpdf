#!/usr/bin/env python3
"""Generate the independent ECDSA PDF fixture used by Prompt 24 tests.

The private key exists only in a short-lived temporary PKCS#12 file consumed
by pyHanko. The committed outputs are the signed PDF and its public cert.
"""

import datetime
import io
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.serialization import BestAvailableEncryption, pkcs12
from cryptography.x509.oid import NameOID
from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import signers
from pyhanko.sign.fields import SigFieldSpec, SigSeedSubFilter, append_signature_field


FIXTURES = Path(__file__).resolve().parents[1] / "crates" / "engine" / "tests" / "fixtures"
BASE_PDF = FIXTURES / "minimal.pdf"
OUT_PDF = FIXTURES / "sig_ecdsa_p256.pdf"
OUT_PADES_PDF = FIXTURES / "sig_pades_b_ecdsa_p256.pdf"
OUT_CERT = FIXTURES / "sig_ecdsa_p256_cert.pem"


def main() -> None:
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name(
        [
            x509.NameAttribute(NameOID.COMMON_NAME, "Wellfriend Prompt24 ECDSA Signer"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Wellfriend Test CA"),
            x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
        ]
    )
    now = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(0xECD5A256)
        .not_valid_before(now)
        .not_valid_after(now + datetime.timedelta(days=3650))
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
        .sign(key, hashes.SHA256())
    )
    temporary_p12 = FIXTURES / "_prompt24_ecdsa_fixture.p12"
    try:
        temporary_p12.write_bytes(
            pkcs12.serialize_key_and_certificates(
                name=b"wellfriendpdf-prompt24-ecdsa",
                key=key,
                cert=cert,
                cas=None,
                encryption_algorithm=BestAvailableEncryption(b"prompt24-fixture"),
            )
        )
        signer = signers.SimpleSigner.load_pkcs12(
            str(temporary_p12), passphrase=b"prompt24-fixture"
        )
        def sign(output_path: Path, field_name: str, subfilter=None) -> None:
            writer = IncrementalPdfFileWriter(io.BytesIO(BASE_PDF.read_bytes()))
            append_signature_field(writer, SigFieldSpec(sig_field_name=field_name))
            output = io.BytesIO()
            signers.sign_pdf(
                writer,
                signers.PdfSignatureMetadata(
                    field_name=field_name, md_algorithm="sha256", subfilter=subfilter
                ),
                signer=signer,
                output=output,
            )
            output_path.write_bytes(output.getvalue())

        sign(OUT_PDF, "Prompt24Ecdsa")
        sign(OUT_PADES_PDF, "Prompt24PadesEcdsa", SigSeedSubFilter.PADES)
        OUT_CERT.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        print(f"wrote {OUT_PDF.name}: {OUT_PDF.stat().st_size} bytes")
        print(f"wrote {OUT_PADES_PDF.name}: {OUT_PADES_PDF.stat().st_size} bytes")
        print(f"wrote {OUT_CERT.name}: {OUT_CERT.stat().st_size} bytes")
    finally:
        temporary_p12.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
