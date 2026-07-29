# Parser Memory Architecture

The normal parser path is lazy and range-oriented. File-backed open reads the
prefix/tail/xref windows it needs and loads indirect objects on demand. Object
streams are decoded through a bounded cache. Parser-report performs additional
audit work, but repair scans and revision traversal remain bounded.

Current low-risk Binding SurfaceB work intentionally does not rewrite the object table
into arenas or a structure-of-arrays layout. That migration should be measured
first against open, first-page access, last-page access, random object lookup,
repair scan, and Arlington validation benchmarks.

Safe future steps:

- intern common PDF names with a cap on unique names per document
- keep raw source spans only while the backing range is pinned, otherwise copy
- store object metadata in a cache-friendly table once lookup benchmarks prove
  it is the limiting factor
- keep the scalar repair scanner as the correctness baseline before adding SIMD
