# Prompt 07 Transparency Compositing Architecture

Prompt 07 extends the Prompt 06 native replay path with a bounded native
transparency foundation. The renderer detects `/Group << /S /Transparency >>`
on Form XObjects and routes those forms through the same `PixelBuffer`
compositing path used for ordinary painted objects. Non-group Form XObjects keep
the existing native replay behavior.

## Render Stack

- Graphics state carries stroking alpha `CA`, nonstroking alpha `ca`, blend mode
  `BM`, soft mask `SMask`, CTM, and clip state.
- Form transparency groups create an intermediate RGBA surface, replay the form
  into that surface, and composite the result back into the parent buffer using
  the active alpha, blend mode, clip, and soft-mask state.
- Soft masks render their `/G` group into an intermediate mask surface and then
  convert that surface to alpha or luminosity coverage for subsequent painting.
- Text, image XObjects, inline images, annotation appearances, and Form XObjects
  paint through the shared renderer state rather than bypassing alpha/blend
  state.

## Bounded Surfaces

Every Prompt 07 offscreen surface is admitted through the renderer decode
scheduler before allocation. The estimate is `width * height * 4` bytes for the
RGBA surface. If the scheduler rejects the request or cancellation is already
set, the renderer fails closed for that group or mask, records scheduler
metrics, and avoids unbounded allocation.

The current implementation keeps page-coordinate surfaces and clips by group or
mask bounds. That preserves deterministic addressing and avoids a large
coordinate rewrite in this pass. Cropped coordinate surfaces remain a memory
optimization for a later renderer prompt.

## Group Semantics

- Isolated groups start from a transparent surface.
- Non-isolated groups copy the parent backdrop into the intermediate surface and
  remove the backdrop contribution before compositing the group result back.
- Group BBox and Matrix are honored on the common Form XObject path.
- Nested groups are supported through recursive render state with scheduler
  admission on every offscreen surface.
- Group color spaces currently use the renderer's device-space posture unless
  the existing color conversion layer can resolve the space. Full ICC-managed
  group color-space parity is not claimed.

## Failure Modes

Malformed group dictionaries, impossible bounds, denied surface allocation,
missing XObjects, unsupported color spaces, and recursive or unknown resources
must report structured diagnostics or fail closed. They must not panic, allocate
unbounded memory, or silently disappear as release-grade parity.

Prompt 08 owns shadings and patterns. This architecture is designed so those
future paint sources can target the same group stack and compositing path.
