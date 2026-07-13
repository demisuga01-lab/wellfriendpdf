# Prompt 21 Persistent Store Design

The Prompt 21 store uses a HAMT-style 32-way Arc trie for ID maps and an RRB-style chunked persistent vector for operation sequences. Inserts copy only the path or active chunk, version graph nodes carry deterministic hashes, and restore rejects snapshot hash/schema mismatches before decode.
