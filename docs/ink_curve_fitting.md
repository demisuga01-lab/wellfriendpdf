# Ink curve fitting

The deterministic fitting pipeline validates finite bounded coordinates,
removes duplicates, applies minimum-distance filtering, collapses collinear
points, optionally smooths with a bounded pass count, preserves detected
corners, simplifies with Douglas-Peucker, estimates tangents, uses chord-length
parameters, solves cubic controls, performs bounded Newton reparameterization,
and recursively splits at maximum error. Points, segments, recursion, Newton
iterations, coordinates, and aggregate work are capped.

Reports include points before/after, segments, maximum and RMS deviation,
compression ratio, elapsed microseconds, recursion depth, and a SHA-256 digest
of canonical control points. Policies are preserve raw, fitted only, raw plus
fitted, fit on import, fit on appearance generation, disabled, strict error,
and performance threshold.

For PDF Ink annotations, `/InkList` remains a point-list interchange surface.
Wellfriend preserves raw points in `/WellfriendRawInkList` when configured, stores cubic
controls in `/WellfriendFittedInk`, and generates a cubic Form XObject appearance.
It does not claim to recover pressure, tilt, velocity, timing, or original pen
dynamics.
