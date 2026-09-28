# Causal story-history compaction - source implementation, unqualified

This increment adds explicit logical compaction for a complete saved causal
story-history epoch. It does not perform automatic tombstone collection, rewrite
historical PDF revisions, sanitize deleted wording, authenticate replicas or
prove distributed acknowledgement.

## Transaction contract

Compaction is available only for the current saved story checkpoint. The caller
must supply all of the following against the exact open PDF revision:

- the current PDF SHA-256 and story identity;
- the exact saved checkpoint and canonical history hashes;
- the exact complete canonical actor frontier;
- acknowledgement that operation history and selective undo will be lost; and
- acknowledgement that replicas retaining the prior epoch cannot rejoin it.

Planning reopens and validates the saved story, source ownership, causal seed and
canonical history before comparing those values. A request with an incomplete or
extra frontier, stale checkpoint, stale history hash, absent acknowledgement or
already-empty history fails closed. The returned plan hash binds the complete
request, checkpoint, generation, seed and history. Apply recomputes that plan and
requires its exact approved hash; it never accepts an approval for a regenerated
or different plan.

## New-epoch algorithm

The transaction makes the currently saved/rebound story the immutable seed of a
new epoch, constructs an empty canonical operation set and verifies that projecting
that seed reproduces the exact current paragraph IDs, order, presence, text and style. It then uses the
existing metadata writer to attach the replacement checkpoint without repainting
story content. The new checkpoint increments the generation, points to the prior
checkpoint, binds the current PDF revision and receives different seed/history
identities. Reopen verification must reproduce the exact replacement checkpoint.

An operation log from the retired epoch has different base identities and is
therefore refused by normal join/replay. This is an epoch barrier, not a distributed
garbage-collection proof: the host remains responsible for authenticating actors,
collecting acknowledgement from every replica, archiving any required audit data
and preventing a retired replica from being treated as a new authorized source.

## Publication and undo

The shared retained session publishes the compacted bytes atomically after reopen.
Its report states whether the exact preimage was actually retained by the bounded
eight-snapshot/128-MiB session history. It does not promise undo when the input PDF
itself exceeds that budget. Browser and native session undo restore the complete
prior PDF bytes, including the old logical epoch, when that preimage is available.

Because the canonical writer uses incremental PDF updates, unreachable historical
metadata and deleted wording can remain in earlier byte revisions. Logical
compaction can bound future active-history work, but it need not reduce file size
and must never be presented as secure deletion or sanitizing redaction.

## Surfaces

The retained protocol exposes `history_compaction_plan` and
`history_compaction_apply`. The browser client exposes `planHistoryCompaction` and
`compactHistory`; apply creates one bounded byte-undo step when possible. The
supplied history component shows the exact plan/frontier/counts and requires two
destructive acknowledgements plus review of the exact plan hash before applying.
It resumes the newly saved empty epoch after publication.

## Evidence boundary

Regression source covers exact frontier/CAS enforcement, acknowledgement and plan
hash rejection, cancellation without publication, new-seed creation, stale-epoch
join refusal, paragraph structure/text/style preservation, protocol dispatch and exact session undo. A
fake-worker source fixture covers request snapshots and browser publication. These
regressions were not executed. No compiler, tests, PDF workload, renderer,
cross-binding execution, browser QA, benchmark, deployment, commit or push was run.
The wider collaboration, editor, rendering and qualification roadmap remains open.
