# Prompt 11B Output Intent Proofing

Prompt 11B adds basic output-intent proofing foundations. The color report
continues to discover `Catalog/OutputIntents` and `DestOutputProfile`; with
`native-cmm-lcms2`, each output profile is additionally checked by LittleCMS.

The helper `proof_srgb_via_output_intent` builds a LittleCMS soft-proofing
transform from sRGB preview pixels through the output-intent profile back to the
sRGB preview target.

This is not full PDF/X proofing. The current target is deterministic native CMM
proofing evidence and report plumbing. Full PDF/X certification, device-link
profile execution, separations, and overprint simulation remain later work.
