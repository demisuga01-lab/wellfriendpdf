# Inline predictor redaction

The parser preserves `<< >>` dictionaries, expands `/DP`, and pairs DecodeParms arrays with matching filters. TIFF predictor 2 and PNG predictors 10–15 are supported when `Colors`, `BitsPerComponent`, and `Columns` exactly match the bounded image layout.

The order is parse, validate, decode filters, reverse predictor, inverse-map the polygon, rewrite samples, deterministically reapply the predictor, Flate encode, rebuild the dictionary, and reparse surrounding content. PNG output uses filter-zero rows; TIFF output uses bounded horizontal differencing.

Malformed counts, oversized rows, unsupported predictors, non-final codec filters, and ambiguous dictionaries remove or fail closed. Surrounding operators remain unchanged.
