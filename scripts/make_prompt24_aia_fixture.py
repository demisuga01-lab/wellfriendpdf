#!/usr/bin/env python3
"""Generate Prompt 24's local AIA path-building integration fixture.

The output embeds only the leaf signer certificate. Its AIA CA-Issuers URI
points to the local test server at 127.0.0.1:18781; the generated private keys
live only in a temporary PKCS#12 file and are never written to the repository.
"""

import datetime
import io
from pathlib import Path

from cryptography import x509
from cryptography.x509 import ocsp
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import BestAvailableEncryption, pkcs12
from cryptography.x509.oid import (
    AuthorityInformationAccessOID,
    ExtendedKeyUsageOID,
    NameOID,
)
from pyhanko.pdf_utils.incremental_writer import IncrementalPdfFileWriter
from pyhanko.sign import signers
from pyhanko.sign.fields import SigFieldSpec, append_signature_field


FIXTURES = Path(__file__).resolve().parents[1] / "crates" / "engine" / "tests" / "fixtures"
AIA_URI = "http://127.0.0.1:18781/intermediate.der"
LEAF_CRL_URI = "http://127.0.0.1:18781/leaf.crl"
INTERMEDIATE_CRL_URI = "http://127.0.0.1:18781/intermediate.crl"
LEAF_OCSP_URI = "http://127.0.0.1:18781/leaf.ocsp"
INTERMEDIATE_OCSP_URI = "http://127.0.0.1:18781/intermediate.ocsp"
TEST_OCSP_NONCE = bytes.fromhex("00112233445566778899aabbccddeeff")


def name(common_name: str) -> x509.Name:
    return x509.Name(
        [
            x509.NameAttribute(NameOID.COMMON_NAME, common_name),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Wellfriend Prompt24 Test PKI"),
            x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
        ]
    )


def ca_certificate(subject, issuer, public_key, issuer_key, serial, path_length, crl_uri=None):
    now = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    builder = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(public_key)
        .serial_number(serial)
        .not_valid_before(now)
        .not_valid_after(now + datetime.timedelta(days=3650))
        .add_extension(x509.BasicConstraints(ca=True, path_length=path_length), critical=True)
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
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(public_key), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(issuer_key.public_key()), critical=False)
    )
    if crl_uri:
        builder = builder.add_extension(
            x509.CRLDistributionPoints(
                [x509.DistributionPoint(
                    full_name=[x509.UniformResourceIdentifier(crl_uri)],
                    relative_name=None,
                    reasons=None,
                    crl_issuer=None,
                )]
            ),
            critical=False,
        )
    return builder.sign(issuer_key, hashes.SHA256())


def crl(issuer_cert, issuer_key, number, revoked_serials=(), delta_base=None):
    now = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    builder = (
        x509.CertificateRevocationListBuilder()
        .issuer_name(issuer_cert.subject)
        .last_update(now - datetime.timedelta(days=1))
        .next_update(now + datetime.timedelta(days=3650))
        .add_extension(x509.CRLNumber(number), critical=False)
    )
    if delta_base is not None:
        builder = builder.add_extension(x509.DeltaCRLIndicator(delta_base), critical=True)
    for serial in revoked_serials:
        builder = builder.add_revoked_certificate(
            x509.RevokedCertificateBuilder()
            .serial_number(serial)
            .revocation_date(now - datetime.timedelta(hours=1))
            .build()
        )
    return builder.sign(issuer_key, hashes.SHA256())


def indirect_crl(crl_signer_cert, crl_signer_key, certificate_issuer, revoked_serial):
    now = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    revoked = (
        x509.RevokedCertificateBuilder()
        .serial_number(revoked_serial)
        .revocation_date(now - datetime.timedelta(hours=1))
        .add_extension(
            x509.CertificateIssuer(
                [x509.DirectoryName(certificate_issuer)]
            ),
            critical=True,
        )
        .build()
    )
    return (
        x509.CertificateRevocationListBuilder()
        .issuer_name(crl_signer_cert.subject)
        .last_update(now - datetime.timedelta(days=1))
        .next_update(now + datetime.timedelta(days=3650))
        .add_extension(x509.CRLNumber(30), critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(
                crl_signer_key.public_key()
            ),
            critical=False,
        )
        .add_extension(
            x509.IssuingDistributionPoint(
                full_name=[x509.UniformResourceIdentifier(LEAF_CRL_URI)],
                relative_name=None,
                only_contains_user_certs=True,
                only_contains_ca_certs=False,
                only_some_reasons=None,
                indirect_crl=True,
                only_contains_attribute_certs=False,
            ),
            critical=True,
        )
        .add_revoked_certificate(revoked)
        .sign(crl_signer_key, hashes.SHA256())
    )


def ocsp_good_response(cert, issuer, issuer_key, nonce=None):
    now = datetime.datetime.now(datetime.timezone.utc)
    builder = (
        ocsp.OCSPResponseBuilder()
        .add_response(
            cert,
            issuer,
            hashes.SHA1(),
            ocsp.OCSPCertStatus.GOOD,
            now - datetime.timedelta(minutes=1),
            now + datetime.timedelta(days=30),
            None,
            None,
        )
        .responder_id(ocsp.OCSPResponderEncoding.NAME, issuer)
    )
    if nonce is not None:
        builder = builder.add_extension(x509.OCSPNonce(nonce), critical=False)
    return builder.sign(issuer_key, hashes.SHA256())


def delegated_ocsp_good_response(cert, issuer, responder, responder_key):
    now = datetime.datetime.now(datetime.timezone.utc)
    return (
        ocsp.OCSPResponseBuilder()
        .add_response(
            cert,
            issuer,
            hashes.SHA1(),
            ocsp.OCSPCertStatus.GOOD,
            now - datetime.timedelta(minutes=1),
            now + datetime.timedelta(days=30),
            None,
            None,
        )
        .responder_id(ocsp.OCSPResponderEncoding.NAME, responder)
        .certificates([responder])
        .sign(responder_key, hashes.SHA256())
    )


def main() -> None:
    now = datetime.datetime(2026, 1, 1, tzinfo=datetime.timezone.utc)
    root_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    root_subject = name("Wellfriend Prompt24 AIA Root")
    root = ca_certificate(root_subject, root_subject, root_key.public_key(), root_key, 0xA1A00001, 1)

    intermediate_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    intermediate_subject = name("Wellfriend Prompt24 AIA Intermediate")
    intermediate = ca_certificate(
        intermediate_subject,
        root.subject,
        intermediate_key.public_key(),
        root_key,
        0xA1A00002,
        0,
        INTERMEDIATE_CRL_URI,
    )
    # The intermediate is checked against the root during strict path
    # revocation evaluation, so it advertises a distinct issuer OCSP endpoint.
    intermediate = (
        x509.CertificateBuilder()
        .subject_name(intermediate.subject)
        .issuer_name(intermediate.issuer)
        .public_key(intermediate_key.public_key())
        .serial_number(intermediate.serial_number)
        .not_valid_before(intermediate.not_valid_before_utc)
        .not_valid_after(intermediate.not_valid_after_utc)
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
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
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(intermediate_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(root_key.public_key()), critical=False)
        .add_extension(
            x509.CRLDistributionPoints(
                [x509.DistributionPoint(
                    full_name=[x509.UniformResourceIdentifier(INTERMEDIATE_CRL_URI)],
                    relative_name=None,
                    reasons=None,
                    crl_issuer=None,
                )]
            ),
            critical=False,
        )
        .add_extension(
            x509.AuthorityInformationAccess(
                [
                    x509.AccessDescription(
                        AuthorityInformationAccessOID.OCSP,
                        x509.UniformResourceIdentifier(INTERMEDIATE_OCSP_URI),
                    )
                ]
            ),
            critical=False,
        )
        .sign(root_key, hashes.SHA256())
    )

    leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    leaf_subject = name("Wellfriend Prompt24 AIA Leaf")
    leaf = (
        x509.CertificateBuilder()
        .subject_name(leaf_subject)
        .issuer_name(intermediate.subject)
        .public_key(leaf_key.public_key())
        .serial_number(0xA1A00003)
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
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(leaf_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .add_extension(
            x509.AuthorityInformationAccess(
                [
                    x509.AccessDescription(
                        AuthorityInformationAccessOID.CA_ISSUERS,
                        x509.UniformResourceIdentifier(AIA_URI),
                    ),
                    x509.AccessDescription(
                        AuthorityInformationAccessOID.OCSP,
                        x509.UniformResourceIdentifier(LEAF_OCSP_URI),
                    ),
                ]
            ),
            critical=False,
        )
        .add_extension(
            x509.CRLDistributionPoints(
                [x509.DistributionPoint(
                    full_name=[x509.UniformResourceIdentifier(LEAF_CRL_URI)],
                    relative_name=None,
                    reasons=None,
                    crl_issuer=None,
                )]
            ),
            critical=False,
        )
        .sign(intermediate_key, hashes.SHA256())
    )

    # This leaf is deliberately chain-valid but its KeyUsage forbids document
    # signatures. It exercises the engine's signer-certificate use policy
    # after CMS math and PKIX path construction have succeeded.
    bad_usage_leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    bad_usage_leaf = (
        x509.CertificateBuilder()
        .subject_name(name("Wellfriend Prompt24 Key Usage Restricted Leaf"))
        .issuer_name(intermediate.subject)
        .public_key(bad_usage_leaf_key.public_key())
        .serial_number(0xA1A00004)
        .not_valid_before(now)
        .not_valid_after(now + datetime.timedelta(days=3650))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=True,
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
            x509.SubjectKeyIdentifier.from_public_key(bad_usage_leaf_key.public_key()),
            critical=False,
        )
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()),
            critical=False,
        )
        .sign(intermediate_key, hashes.SHA256())
    )

    crl_signer_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    crl_signer = (
        x509.CertificateBuilder()
        .subject_name(name("Wellfriend Prompt24 Delegated CRL Signer"))
        .issuer_name(intermediate.subject)
        .public_key(crl_signer_key.public_key())
        .serial_number(0xA1A00005)
        .not_valid_before(now)
        .not_valid_after(now + datetime.timedelta(days=3650))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(crl_signer_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .sign(intermediate_key, hashes.SHA256())
    )

    indirect_leaf_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    indirect_leaf = (
        x509.CertificateBuilder()
        .subject_name(name("Wellfriend Prompt24 Indirect CRL Leaf"))
        .issuer_name(intermediate.subject)
        .public_key(indirect_leaf_key.public_key())
        .serial_number(0xA1A00006)
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
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(indirect_leaf_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .add_extension(
            x509.CRLDistributionPoints(
                [x509.DistributionPoint(
                    full_name=[x509.UniformResourceIdentifier(LEAF_CRL_URI)],
                    relative_name=None,
                    reasons=None,
                    crl_issuer=[x509.DirectoryName(crl_signer.subject)],
                )]
            ),
            critical=False,
        )
        .sign(intermediate_key, hashes.SHA256())
    )

    ocsp_responder_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    ocsp_responder = (
        x509.CertificateBuilder()
        .subject_name(name("Wellfriend Prompt24 Delegated OCSP Responder"))
        .issuer_name(intermediate.subject)
        .public_key(ocsp_responder_key.public_key())
        .serial_number(0xA1A00007)
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
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.OCSP_SIGNING]), critical=False
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(ocsp_responder_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .sign(intermediate_key, hashes.SHA256())
    )

    bad_ocsp_responder_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    bad_ocsp_responder = (
        x509.CertificateBuilder()
        .subject_name(name("Wellfriend Prompt24 Wrong EKU OCSP Responder"))
        .issuer_name(intermediate.subject)
        .public_key(bad_ocsp_responder_key.public_key())
        .serial_number(0xA1A00008)
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
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.CLIENT_AUTH]), critical=False
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(bad_ocsp_responder_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(intermediate_key.public_key()), critical=False)
        .sign(intermediate_key, hashes.SHA256())
    )

    temporary_p12 = FIXTURES / "_prompt24_aia_fixture.p12"
    temporary_bad_usage_p12 = FIXTURES / "_prompt24_aia_bad_usage_fixture.p12"
    temporary_indirect_leaf_p12 = FIXTURES / "_prompt24_aia_indirect_leaf_fixture.p12"
    try:
        temporary_p12.write_bytes(
            pkcs12.serialize_key_and_certificates(
                name=b"wellfriendpdf-prompt24-aia-leaf",
                key=leaf_key,
                cert=leaf,
                cas=None,
                encryption_algorithm=BestAvailableEncryption(b"prompt24-aia-fixture"),
            )
        )
        signer = signers.SimpleSigner.load_pkcs12(
            str(temporary_p12), passphrase=b"prompt24-aia-fixture"
        )
        writer = IncrementalPdfFileWriter(io.BytesIO((FIXTURES / "minimal.pdf").read_bytes()))
        append_signature_field(writer, SigFieldSpec(sig_field_name="Prompt24Aia"))
        output = io.BytesIO()
        signers.sign_pdf(
            writer,
            signers.PdfSignatureMetadata(field_name="Prompt24Aia", md_algorithm="sha256"),
            signer=signer,
            output=output,
        )
        (FIXTURES / "sig_aia_leaf_only.pdf").write_bytes(output.getvalue())

        temporary_bad_usage_p12.write_bytes(
            pkcs12.serialize_key_and_certificates(
                name=b"wellfriendpdf-prompt24-aia-bad-usage-leaf",
                key=bad_usage_leaf_key,
                cert=bad_usage_leaf,
                cas=None,
                encryption_algorithm=BestAvailableEncryption(b"prompt24-aia-fixture"),
            )
        )
        bad_usage_signer = signers.SimpleSigner.load_pkcs12(
            str(temporary_bad_usage_p12), passphrase=b"prompt24-aia-fixture"
        )
        bad_usage_writer = IncrementalPdfFileWriter(
            io.BytesIO((FIXTURES / "minimal.pdf").read_bytes())
        )
        append_signature_field(bad_usage_writer, SigFieldSpec(sig_field_name="Prompt24BadUsage"))
        bad_usage_output = io.BytesIO()
        signers.sign_pdf(
            bad_usage_writer,
            signers.PdfSignatureMetadata(
                field_name="Prompt24BadUsage", md_algorithm="sha256"
            ),
            signer=bad_usage_signer,
            output=bad_usage_output,
        )
        (FIXTURES / "sig_aia_bad_key_usage.pdf").write_bytes(bad_usage_output.getvalue())

        temporary_indirect_leaf_p12.write_bytes(
            pkcs12.serialize_key_and_certificates(
                name=b"wellfriendpdf-prompt24-aia-indirect-leaf",
                key=indirect_leaf_key,
                cert=indirect_leaf,
                cas=None,
                encryption_algorithm=BestAvailableEncryption(b"prompt24-aia-fixture"),
            )
        )
        indirect_leaf_signer = signers.SimpleSigner.load_pkcs12(
            str(temporary_indirect_leaf_p12), passphrase=b"prompt24-aia-fixture"
        )
        indirect_leaf_writer = IncrementalPdfFileWriter(
            io.BytesIO((FIXTURES / "minimal.pdf").read_bytes())
        )
        append_signature_field(
            indirect_leaf_writer, SigFieldSpec(sig_field_name="Prompt24IndirectCrl")
        )
        indirect_leaf_output = io.BytesIO()
        signers.sign_pdf(
            indirect_leaf_writer,
            signers.PdfSignatureMetadata(
                field_name="Prompt24IndirectCrl", md_algorithm="sha256"
            ),
            signer=indirect_leaf_signer,
            output=indirect_leaf_output,
        )
        (FIXTURES / "sig_aia_indirect_leaf_only.pdf").write_bytes(
            indirect_leaf_output.getvalue()
        )
        (FIXTURES / "aia_root.der").write_bytes(root.public_bytes(serialization.Encoding.DER))
        (FIXTURES / "aia_intermediate.der").write_bytes(
            intermediate.public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_crl_signer.der").write_bytes(
            crl_signer.public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_indirect_leaf_revoked.crl").write_bytes(
            indirect_crl(
                crl_signer,
                crl_signer_key,
                intermediate.subject,
                indirect_leaf.serial_number,
            ).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_leaf_good.crl").write_bytes(
            crl(intermediate, intermediate_key, 1).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_leaf_revoked.crl").write_bytes(
            crl(intermediate, intermediate_key, 2, [leaf.serial_number]).public_bytes(
                serialization.Encoding.DER
            )
        )
        (FIXTURES / "aia_leaf_delta_base_good.crl").write_bytes(
            crl(intermediate, intermediate_key, 10).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_leaf_delta_revoked.crl").write_bytes(
            crl(
                intermediate,
                intermediate_key,
                11,
                [leaf.serial_number],
                delta_base=10,
            ).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_intermediate_good.crl").write_bytes(
            crl(root, root_key, 1).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_leaf_good.ocsp").write_bytes(
            ocsp_good_response(leaf, intermediate, intermediate_key).public_bytes(
                serialization.Encoding.DER
            )
        )
        (FIXTURES / "aia_leaf_nonce_good.ocsp").write_bytes(
            ocsp_good_response(leaf, intermediate, intermediate_key, TEST_OCSP_NONCE).public_bytes(
                serialization.Encoding.DER
            )
        )
        (FIXTURES / "aia_leaf_delegated_good.ocsp").write_bytes(
            delegated_ocsp_good_response(
                leaf, intermediate, ocsp_responder, ocsp_responder_key
            ).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_leaf_delegated_wrong_eku.ocsp").write_bytes(
            delegated_ocsp_good_response(
                leaf,
                intermediate,
                bad_ocsp_responder,
                bad_ocsp_responder_key,
            ).public_bytes(serialization.Encoding.DER)
        )
        (FIXTURES / "aia_intermediate_good.ocsp").write_bytes(
            ocsp_good_response(intermediate, root, root_key).public_bytes(
                serialization.Encoding.DER
            )
        )
        print("wrote sig_aia_leaf_only.pdf")
        print("wrote sig_aia_bad_key_usage.pdf")
        print("wrote sig_aia_indirect_leaf_only.pdf")
        print("wrote aia_root.der")
        print("wrote aia_intermediate.der")
        print("wrote aia_crl_signer.der")
        print("wrote aia_indirect_leaf_revoked.crl")
        print("wrote aia_leaf_good.crl")
        print("wrote aia_leaf_revoked.crl")
        print("wrote aia_leaf_delta_base_good.crl")
        print("wrote aia_leaf_delta_revoked.crl")
        print("wrote aia_intermediate_good.crl")
        print("wrote aia_leaf_good.ocsp")
        print("wrote aia_leaf_nonce_good.ocsp")
        print("wrote aia_leaf_delegated_good.ocsp")
        print("wrote aia_leaf_delegated_wrong_eku.ocsp")
        print("wrote aia_intermediate_good.ocsp")
    finally:
        temporary_p12.unlink(missing_ok=True)
        temporary_bad_usage_p12.unlink(missing_ok=True)
        temporary_indirect_leaf_p12.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
