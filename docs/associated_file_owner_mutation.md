# Owner-specific associated-file mutation

Oxide supports full-rewrite add, update, and unlink for catalog `/AF`, page `/AF`, annotation `/FS` and `/AF`, structure-element `/AF`, Form XObject `/AF`, and supported image XObject `/AF`. Catalog EmbeddedFiles indexing remains distinct; owner mutation does not silently canonicalize every association into that name tree.

Update creates a new FileSpec for the selected owner. Other owners survive when a FileSpec is shared, including different `AFRelationship` values. Filename variants, description, MIME, size, SHA-256 provenance, relationship, payload hash, and owner identity are rescanned after reopen. Identical streams may be deduplicated.

Unlink removes only the selected owner reference. FileSpecs and embedded streams are deleted only when unreachable. External specifications are never fetched or executed. Paths are sanitized and reserved platform names rejected.
