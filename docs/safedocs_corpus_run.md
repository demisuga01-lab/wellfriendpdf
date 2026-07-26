# SafeDocs corpus run

The runner first looks for a local/VPS SafeDocs corpus root. If absent or infeasible, it records exact `unavailable_external_corpus` provenance and runs the closest committed malformed/public corpus fallback.
A full SafeDocs claim is made only when every file in the available SafeDocs root is attempted.
