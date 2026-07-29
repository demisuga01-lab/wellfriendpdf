# Crypto Standards Fuzz fuzz seed policy

Allowed seeds:

- repository-generated minimal PDFs
- minimized fuzz artifacts produced by this project
- public/legal seeds with license and source recorded
- compact synthetic parser cases

Disallowed seeds:

- copyrighted PDFs without permission
- giant corpora committed to Git
- private customer files
- files containing secrets, credentials, signatures from real identities, or private keys
- VPS temp directories or generated package outputs

Seed promotion command:

```bash
python scripts/promote_fuzz_seed.py <target> <minimized-seed> --reason "regression for Crypto Standards Fuzz parser finding"
```

The promotion script records destination, hash, size, target, and reason in
`parser-seed-promotion.json`.
