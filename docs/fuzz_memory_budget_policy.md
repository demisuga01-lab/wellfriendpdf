# Fuzz memory budget policy

The Wellfriend PDF SDK VPS budget is 32 GiB aggregate.
Prompt 28 cargo-fuzz runs use one target at a time with a 16 GiB process-tree RSS cap unless a stricter cap is documented.
