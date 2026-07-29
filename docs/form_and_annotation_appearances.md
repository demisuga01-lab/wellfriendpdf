# Form and Annotation Appearances

Form values are serialized through the existing canonical form appearance
writer. Annotation create/edit/delete and XFDF import regenerate supported
appearance resources through annotation/media redaction. This preserves the owning document
graph and avoids a binding-specific rendering path.

Flattening is an explicit action: the canonical appearance is painted into page
content and the live annotation/widget is removed or detached under the
selected policy. Unsupported appearance states fail closed rather than leaving
an invisible live object beneath replacement paint.

Appearance updates use canonical form and annotation writers, including normal,
rollover, and down states where supported. Viewer regeneration is not relied on
for supported paths.
