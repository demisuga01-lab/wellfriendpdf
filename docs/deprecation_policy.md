# Deprecation policy

Public behavior is deprecated by documenting its replacement, preserving the old
behavior for at least one compatible minor release where practical, and emitting an
actionable warning in language-appropriate surfaces. C ABI aliases must be explicit,
documented, and ownership-safe. Silent semantic changes to parsing, signature,
redaction, or validation results are never treated as ordinary deprecations.

For unreleased surfaces, removal is allowed only after the release API inventory is
updated and examples/docs no longer present the removed surface.
