# PadesLTV Interoperability

Schema: `pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1`

Independent interoperability now includes a pyHanko-generated standalone RFC 3161 token that Wellfriend validates through the public CLI, with wrong-imprint rejection on both sides. pyHanko also generates and validates PAdES B-T and DSS/VRI-bearing LTV fixtures that Wellfriend validates through the public CLI, including timestamp and pyHanko-compatible VRI binding. qpdf records structure-only permission/edit checks with warnings and is not counted as a conformance validator.
