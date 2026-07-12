# Editable vector object model

Prompt 20 reconstructs vector objects from actual content operations. A path
starts with `m`, `l`, `c`, `v`, `y`, `h`, or `re` and ends at its painting or
end-path operator. Oxide does not combine adjacent paths into inferred semantic
ellipses, icons, charts, or other shapes.

Every object carries a SHA-256-derived stable ID; page, object, generation,
stream index, byte range, Form invocation and Oxide group stacks,
marked-content depth, OCG context, and resource-owner provenance; path
segments; bbox; matrix; fill rule and paint
mode; stroke width/dash/cap/join/miter; fill/stroke colors; opacity/blend and
ExtGState reference posture; clipping flags; confidence; edit safety; and
diagnostics.

The implemented inventory covers page-owned streams, reachable Form XObjects to
depth eight, and indirect annotation `/AP` streams. Pattern and shading
references remain references rather than being misrepresented as solid colors.
Shared Forms require explicit reject/edit-all/clone-one policy. Annotation
appearance streams shared by multiple annotations are diagnosed and rejected
until an ownership-specific clone is requested by a future surface; they are
never silently edited across annotations.
