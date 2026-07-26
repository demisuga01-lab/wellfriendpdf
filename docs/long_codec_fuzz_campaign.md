# Long codec fuzz campaign

Codec fuzzing exercises `filters`, `predictor`, `image_decoders`, and `decode_scanner`.
The campaign covers filter chains, Flate/LZW/ASCII filters, RunLength, predictors, image metadata, DCT, JPX, JBIG2, CCITT, and decode discovery with exact limits from the existing codec policy.
