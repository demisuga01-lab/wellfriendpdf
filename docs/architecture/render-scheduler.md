# Render scheduler

Rendering is scheduled through bounded Standard/Research resource policy. Work
is classified before admission so one expensive rendering or OCR job cannot
exhaust unrelated metadata, parsing, editing, or standards work.

Scheduler responsibilities:

- estimate CPU, memory, temporary disk, output size, and parallelism ceiling;
- keep queues bounded and cancellable;
- prefer visible tiles, then adjacent tiles, then background prefetch;
- avoid oversubscribing serial document mutations;
- preserve deterministic document meaning independent of scheduling;
- report progress and cancellation at safe boundaries.

The progressive renderer exposes resumable tile-step reports. In this task,
progressive tile steps pass the cancellation token into the actual tile render
path instead of polling only between full-page crops.
