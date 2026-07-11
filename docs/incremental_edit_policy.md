# Incremental edit policy

The canonical incremental writer copies the original file as an exact prefix, appends deterministic objects, emits a valid xref/trailer revision with `Prev`, and reopens the result. It does not modify signature dictionaries.

Prompt 18 includes an executable bounded metadata update against an existing Info dictionary as the prefix-preservation proof. Form, annotation, page, attachment, and content operations are classified before execution. Secure data removal is never represented as incremental preservation because old revision bytes would remain recoverable.
