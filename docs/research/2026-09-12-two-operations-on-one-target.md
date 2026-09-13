# Two operations on one target: refuse the batch, or apply them in order?

Research note, 2026-09-12, for issue #8 (`apply`: JSON batch, atomic, stdin on dash).
Primary sources only: RFC text from rfc-editor.org, first-party project documentation,
and the projects' own source code. Every claim carries a URL and, for specifications, a
section number. Quotations were checked against raw copies fetched on 2026-09-12, not
against search-engine summaries or third-party write-ups.

Texts read in full: RFC 6902, RFC 7386/RFC 7396, RFC 5789, RFC 8259 §4; git
`Documentation/git-apply.adoc` and `apply.c` from git.kernel.org; kubernetes.io
server-side-apply reference; `kubernetes/apimachinery` `pkg/api/errors/errors.go` and
`pkg/util/managedfields/internal/conflict.go`; `kubernetes-sigs/structured-merge-diff`
`typed/validate.go`; `kubernetes/community` API conventions; AWS DynamoDB API reference
for `BatchWriteItem`, `TransactWriteItems` and Common Error Types; PostgreSQL 18 docs for
`INSERT`, `MERGE` and the error-code appendix; POSIX (IEEE Std 1003.1-2024) `patch`.

Runnable probes: `git apply` behaviour under git 2.50.1 (Apple Git-155) on three
hand-built patches; and xlsplice's own `batch::run`, `worksheet::value_splices` and
`splice::apply` driven from a scratch crate with a path dependency on this repository,
over a copy of `tests/fixtures/plain.xlsx` and over hand-built worksheet parts. Nothing in
this repository was modified and no fixture was touched.

## Summary

1. **The sources do not disagree about ordering. They disagree about whether one
   operation can be stated without reference to what came before it.** Where an operation
   carries no assertion about prior state, every format examined that applies a batch to
   one snapshot refuses a repeated target. Where an operation can state what it expects to
   find (JSON Patch's `test`, a diff hunk's context lines), ordering is defined and a
   repeat is legal.
2. **RFC 6902 (JSON Patch) defines order and applies everything**: "Operations are applied
   sequentially in the order they appear in the array... the resulting document becomes the
   target of the next operation" (§3). It never forbids two operations on one path. But its
   operations are position-free pointer-plus-value, so ordering is the only thing that makes
   them composable at all.
3. **RFC 7396 (JSON Merge Patch, which obsoletes RFC 7386) cannot express the question.** A
   merge patch is a JSON object keyed by member name, and RFC 8259 §4 says "The names within
   an object SHOULD be unique", warning that otherwise "the behavior of software that
   receives such an object is unpredictable". xlsplice's batch is an array, so it can express
   what merge patch cannot.
4. **RFC 5789 §2.2 is decisive on sub-question 2.** Every failure mode it lists is a 4xx:
   400, 404, 409, 412, 415, 422. There is no 5xx in the section. A self-contradictory
   request is the caller's, and the caller is told to fix it.
5. **git apply refuses a self-contradictory patch and applies a self-consistent one.**
   Probed: two hunks on the same line in one diff → refused, exit 1, working tree untouched;
   two diffs for one file where the second's context matches the first's *result* → applied
   in order, last write wins, exit 0; two diffs where the second's context matches the
   *original* → refused, exit 1. Context lines are what let git tell the two apart. `set`
   in xlsplice carries no equivalent.
6. **Kubernetes draws exactly the distinction this ticket needs, and prices the two halves
   differently.** A conflict *between managers* is `409 Conflict` and has a force override
   (`--force-conflicts`). A *duplicate key inside one request* is a validation error —
   `duplicate entries for key` in structured-merge-diff — surfaced as `Invalid`, which is
   `422 Unprocessable Entity` with no override. Conflict-between-parties and
   contradiction-within-one-request are not the same error.
7. **DynamoDB refuses duplicate targets in both its batch and its transactional API.**
   `BatchWriteItem` "rejects the entire batch write operation" if "You try to perform
   multiple operations on the same item"; `TransactWriteItems` says "no two actions can
   target the same item". Both are HTTP 400 — the input-validation class, not the
   conflict class and not the 500 class.
8. **SQL raises a cardinality violation, and the SQL standard requires it.** PostgreSQL's
   `INSERT ... ON CONFLICT DO UPDATE` "will not be allowed to affect any single existing row
   more than once"; `MERGE` says a repeated `UPDATE` or `DELETE` "will cause a cardinality
   violation; the latter behavior is required by the SQL standard". That is SQLSTATE
   `21000`, Class 21, which PostgreSQL keeps distinct from Class XX `internal_error`
   (`XX000`).
9. **POSIX `patch` separates "your patch did not fully apply" (exit 1) from "an error
   occurred" (exit >1)** — a direct precedent for keeping a caller's bad batch out of the
   unexpected-failure bucket.
10. **What xlsplice does today is worse than exit 1 — it is sometimes exit 0 and a broken
    package.** Probed against the library: two `set` operations on a cell that holds a value
    hit the overlap guard at `src/splice.rs:74` and fail `internal`, exit 1, nothing written
    — as the ticket says. But on a cell written `<c r="A1"></c>` both splices are *empty*
    ranges at the same byte, the guard's `start < reached` test never fires, and the batch
    succeeds, writing `<c r="A1" t="inlineStr" t="inlineStr">…` — a part that no longer
    parses as XML, at exit 0. The overlap guard is a byte-range guard, not a duplicate-target
    guard, and it must not be relied on as one.
11. **Recommendation: refuse, as a usage error, exit 2**, detected during the whole-batch
    validation on the *resolved* address, naming both operation indices. Do not add a
    force flag in v1. Leave the overlap guard where it is and leave it `internal`: once the
    duplicate-target check exists, an overlap really is a bug in xlsplice's own splice
    computation, which is what `internal` means.

---

## 1. The question, and what xlsplice does today

### 1.1 The batch is computed against one snapshot; it is not applied in sequence

`batch::run` (`src/batch.rs:149`) reads the package once, resolves **every** target before
any part is read (`src/batch.rs:156`, whose comment says so: "Every target is resolved
before any part is read"), then for each part parses it once and computes every splice that
lands on it against that one tree, before applying any of them (`splice_part`,
`src/batch.rs:219`). `splice::apply` then sorts the splices and applies them from the end of
the part backwards, which is ADR-0002's rule and is what keeps the earlier byte ranges
valid.

Nothing re-reads, re-parses or re-resolves between operations. There is no "the resulting
document becomes the target of the next operation". The operations in a batch are, by
construction, a set of independent edits to one snapshot, keyed by target — structurally a
map, not a sequence. That is the single fact the rest of this note turns on, and §11 names
it as the condition under which the recommendation would change.

A second structural fact: `OperationReport.changed` is documented as "Whether it changed a
byte. A write of the value already there did not" (`src/batch.rs:105`). Under last-write-wins
there is no honest value for a superseded operation. It did not change a byte of the result,
but reporting `false` means what a no-op write means — that the cell already held the value —
which would be a lie. The report shape the spec fixes cannot describe a superseded operation.

### 1.2 What the current code actually does with a duplicate target (probed)

Driving the library from a scratch crate (path dependency on this repository, fixture copied
out of `tests/fixtures/`):

```
1. duplicate target -> code=internal exit=1 msg=two splices overlap at byte 956;
   one part cannot be asked for two different things in the same place
   out.xlsx written? false
```

That is two `set` operations on `Sheet1!A1` of `plain.xlsx` through the real `batch::run`
path. It matches the ticket's description. But the guard is a byte-range test
(`if start < reached`, `src/splice.rs:74`) and two splices with the same *empty* range do
not trip it. Sweeping the cell shapes through `value_splices` + `splice::apply`:

| Cell as the part writes it | Two operations | Result |
|---|---|---|
| `<c r="A1"><v>1</v></c>` | two numbers | refused, `internal` |
| `<c r="A1" t="b"><v>1</v></c>` | two bools | refused, `internal` |
| `<c r="A1"/>` | two numbers | refused, `internal` |
| `<c r="A1" s="4"/>` | two texts | refused, `internal` |
| `<c r="A1"></c>` | two numbers | **no error**: `<c r="A1"><v>1</v><v>2</v></c>` |
| `<c r="A1"></c>` | two texts | **no error**: `<c r="A1" t="inlineStr" t="inlineStr">…`, does not re-parse |

For the last row the library returned a part that `roxmltree` will not parse, and
`batch::run` would land it and exit 0. Excel would demand a repair on that package — the
exact failure ADR-0001 exists to prevent.

Excel does not write `<c r="A1"></c>`; it writes `<c r="A1"/>` (confirmed in both
`plain.xlsx` and `feature.xlsx`). So the hole is not reachable from an Excel-saved fixture.
It is reachable from any package some other producer has been through, which is precisely
the corpus this tool is aimed at. The conclusion is not "the guard needs a better test" —
it is that a duplicate target must be caught *before* splices are computed, where the
question is about targets rather than about bytes.

---

## 2. RFC 6902, JSON Patch: order is defined, everything applies, the whole thing is atomic

<https://www.rfc-editor.org/rfc/rfc6902.txt>

§3, Document Structure:

> Evaluation of a JSON Patch document begins against a target JSON document. Operations are
> applied sequentially in the order they appear in the array. Each operation in the sequence
> is applied to the target document; the resulting document becomes the target of the next
> operation. Evaluation continues until all operations are successfully applied or until an
> error condition is encountered.

Nothing in RFC 6902 refuses two operations on the same `path`. The example in §3 itself does
`remove /a/b/c`, then `add /a/b/c`, then `replace /a/b/c` — three operations on one location,
legal and meaningful because each is defined against the document the previous one produced.

§5, Error Handling, gives the atomicity:

> If a normative requirement is violated by a JSON Patch document, or if an operation is not
> successful, evaluation of the JSON Patch document SHOULD terminate and application of the
> entire patch document SHALL NOT be deemed successful.
>
> See [RFC5789], Section 2.2 for considerations regarding handling errors when JSON Patch is
> used with the HTTP PATCH method, including suggested status codes to use to indicate
> various conditions.
>
> Note that the HTTP PATCH method is atomic, as per [RFC5789]. Therefore, the following patch
> would result in no changes being made to the document at all (because the "test" operation
> results in an error)

RFC 6902 delegates *which* error to RFC 5789 §2.2 entirely; it defines no codes of its own.

Two details matter for the comparison. First, §4.3 makes `replace` depend on the state it
finds — "The target location MUST exist for the operation to be successful" — and defines it
as "functionally identical to a `remove` operation for a value, followed immediately by an
`add` operation at the same location". Second, §4.6 gives the format a way for the caller to
*assert* the state it expects: "The `test` operation tests that a value at the target location
is equal to a specified value." A JSON Patch caller who means to override an earlier operation
can say so; one who wants the batch checked against its assumptions can say that too.
xlsplice's `set` has neither faculty.

**Reading**: JSON Patch is the strongest case for "apply in order", and it is a real one. But
it buys composability with an explicit sequential model — re-evaluating the document between
operations — that ADR-0002's one-parse-per-part rule deliberately does not have.

---

## 3. RFC 7396 (obsoletes RFC 7386), JSON Merge Patch: the question cannot be asked

RFC 7386 <https://www.rfc-editor.org/rfc/rfc7386.txt> was published in October 2014 and is
marked **Obsoleted by RFC 7396** <https://www.rfc-editor.org/rfc/rfc7396.txt>, published the
same month with corrections. The ticket named 7386; 7396 is the text to cite. The substance
below is identical in both.

§2, Processing Merge Patch Documents, is a pseudocode function:

> ```
> define MergePatch(Target, Patch):
>   if Patch is an Object:
>     if Target is not an Object:
>       Target = {} # Ignore the contents and set it to an empty Object
>     for each Name/Value pair in Patch:
>       if Value is null:
>         if Name exists in Target:
>           remove the Name/Value pair from Target
>       else:
>         Target[Name] = MergePatch(Target[Name], Value)
>     return Target
>   else:
>     return Patch
> ```

There is no ordering question because a merge patch is not a list: it is an object keyed by
member name, and a target appears at most once. The uniqueness is the JSON object's, and
RFC 8259 §4 <https://www.rfc-editor.org/rfc/rfc8259.txt> is explicit about what it is worth:

> The names within an object SHOULD be unique.
>
> An object whose names are all unique is interoperable in the sense that all software
> implementations receiving that object will agree on the name-value mappings. When the names
> within an object are not unique, the behavior of software that receives such an object is
> unpredictable. Many implementations report the last name/value pair only. Other
> implementations report an error or fail to parse the object, and some implementations
> report all of the name/value pairs, including duplicates.

**Reading**: last-write-wins is what JSON parsers *most commonly* do with a duplicate key, and
RFC 8259 calls the whole area unpredictable and tells authors not to go there. That is an
argument against building a duplicate-target convention, not for one. Merge patch's shape —
a map from target to value, applied whole against one snapshot — is exactly the shape of
xlsplice's batch as `batch::run` computes it. The reason xlsplice uses an array rather than an
object is that a batch mixes operation kinds (`set`, `clear`, `props.set`, `props.unset`,
`calc`) and must be reported back by index. The array is a carrier, not a claim of sequential
semantics.

---

## 4. RFC 5789, HTTP PATCH: whole-or-nothing, and every failure is the caller's

<https://www.rfc-editor.org/rfc/rfc5789.txt>

§2, The PATCH Method — the atomicity every other patch RFC defers to:

> The server MUST apply the entire set of changes atomically and never provide (e.g., in
> response to a GET during this operation) a partially modified representation. If the entire
> patch document cannot be successfully applied, then the server MUST NOT apply any of the
> changes.

This is the same promise as the spec's "A batch is validated whole before anything is
spliced, then applied whole. Any failure leaves the file untouched", and as the exit-code
table's "Any non-zero code guarantees nothing was written".

§2.2, Error Handling, is the section that answers sub-question 2. Its five named conditions,
in full:

> **Malformed patch document:** When the server determines that the patch document provided
> by the client is not properly formatted, it SHOULD return a 400 (Bad Request) response. The
> definition of badly formatted depends on the patch document chosen.
>
> **Unsupported patch document:** Can be specified using a 415 (Unsupported Media Type)
> response when the client sends a patch document format that the server does not support…
>
> **Unprocessable request:** Can be specified with a 422 (Unprocessable Entity) response
> ([RFC4918], Section 11.2) when the server understands the patch document and the syntax of
> the patch document appears to be valid, but the server is incapable of processing the
> request. This might include attempts to modify a resource in a way that would cause the
> resource to become invalid; for instance, a modification to a well-formed XML document that
> would cause it to no longer be well-formed. There may also be more specific errors like
> "Conflicting State" that could be signaled with this status code, but the more specific
> error would generally be more helpful.
>
> **Resource not found:** Can be specified with a 404 (Not Found) status code when the client
> attempted to apply a patch document to a non-existent resource…
>
> **Conflicting state:** Can be specified with a 409 (Conflict) status code when the request
> cannot be applied given the state of the resource. For example, if the client attempted to
> apply a structural modification and the structures assumed to exist did not exist (with XML,
> a patch might specify changing element 'foo' to element 'bar' but element 'foo' might not
> exist).

Plus 412 for a failed precondition. **Not one 5xx appears in §2.2.** A patch document that
the server cannot carry out is, without exception, treated as something the client got wrong.

Two of these bear directly on the choice of code. "Malformed patch document… The definition
of badly formatted depends on the patch document chosen" hands the format's author the right
to declare a duplicate target ill-formed. And "Conflicting state… cannot be applied **given
the state of the resource**" is explicitly about the *resource* — for xlsplice, the package —
not about the request contradicting itself. A batch that names one cell twice is wrong
whatever package it is pointed at.

The 422 paragraph is also a fair description of today's silent failure mode: "a modification
to a well-formed XML document that would cause it to no longer be well-formed". §1.2 shows
xlsplice performing exactly that modification and exiting 0.

---

## 5. git apply: in order — but only when the caller demonstrates it knew

Documentation and source read from git.kernel.org on 2026-09-12:
<https://git.kernel.org/pub/scm/git/git.git/plain/Documentation/git-apply.adoc> and
<https://git.kernel.org/pub/scm/git/git.git/plain/apply.c>. The rendered man page is
<https://git-scm.com/docs/git-apply>.

**Atomicity.** From the `--reject` entry in the adoc:

> For atomicity, `git apply` by default fails the whole patch and does not touch the working
> tree when some of the hunks do not apply. This option makes it apply the parts of the patch
> that are applicable, and leave the rejected hunks in corresponding `*.rej` files.

The implementation is a two-pass: `apply_patch` calls `check_patch_list(state, list)` over
every patch in the input and only then `write_out_results(state, list)` (`apply.c`, the two
calls sit about ten lines apart near the end of `apply_patch`). Same shape as
`batch::run`: validate whole, then land whole.

**Sequential application to one path is explicit, and it is in-memory.** `load_preimage`:

```c
previous = previous_patch(state, patch, &status);
if (status)
        return error(_("path %s has been renamed/deleted"), patch->old_name);
if (previous) {
        /* We have a patched copy in memory; use that. */
        strbuf_add(&buf, previous->result, previous->resultsize);
} else {
        status = load_patch_target(state, &buf, ce, st, patch, ...);
```

and the table it consults, from the comment above `PATH_TO_BE_DELETED`:

```c
/*
 * item->util in the filename table records the status of the path.
 * Usually it points at a patch (whose result records the contents
 * of it after applying it), but it could be PATH_WAS_DELETED for a
 * path that a previously applied patch has already removed, or
 * PATH_TO_BE_DELETED for a path that a later patch would remove.
 ...
```

So a second diff for a path in the same patch file is applied to the first diff's result.
That is last-write-wins, and git built it deliberately.

**But it is gated on context.** Probed with git 2.50.1 on a five-line file, all three patches
needing `--unidiff-zero` because, per the adoc, "By default, `git apply` expects that the
patch being applied is a unified diff with at least one line of context. This provides good
safety measures":

| Patch | Result |
|---|---|
| A — two hunks in **one** diff, both `@@ -3,1 +3,1 @@`, both removing `c` | `error: patch failed: f.txt:3` / `error: f.txt: patch does not apply`, exit 1, file unchanged |
| B — **two** diffs for `f.txt`; second removes `FIRST`, i.e. the first's result | exit 0, file now holds `SECOND`. Last write wins |
| C — **two** diffs for `f.txt`; second removes `c`, i.e. the original | `error: patch failed: f.txt:3`, exit 1, file unchanged |

A and C are the self-contradictory batches, and git refuses both and writes nothing. B is the
self-consistent one, and git applies it. The context lines are the whole mechanism: they are
the caller's assertion about what it expects to find, and they let git distinguish "I meant
to override my earlier change" from "I built this patch out of two stale pieces".

**Reading**: git apply is not a vote for last-write-wins. It is a vote for *last-write-wins
where the caller proves it knew about the earlier write, and refusal where it did not*. An
xlsplice `set` is a target, a type and a value; it asserts nothing about what the cell held.
Given a duplicate target, xlsplice is in git's case C with no way to tell it from case B.

**On exit codes, git apply is no help.** It has no code table: every failure above is exit 1,
and git does not distinguish a bad patch from an internal fault. That is a reason not to read
anything into the `1`, not a precedent for xlsplice's `internal`.

---

## 6. Kubernetes server-side apply: two kinds of conflict, deliberately priced differently

<https://kubernetes.io/docs/reference/using-api/server-side-apply/>

**A conflict is between managers, and it is forcible.** From the Conflicts section:

> A _conflict_ is a special status error that occurs when an `Apply` operation tries to change
> a field that another manager also claims to manage. This prevents an applier from
> unintentionally overwriting the value set by another user.

and from Field management:

> A Server-Side Apply **patch** request requires the client to provide its identity as a field
> manager. When using Server-Side Apply, trying to change a field that is controlled by a
> different manager results in a rejected request unless the client forces an override.

The escape hatch, from Conflicts:

> If overwriting the value was intentional (or if the applier is an automated process like a
> controller) the applier should set the `force` query parameter to true (for `kubectl apply`,
> you use the `--force-conflicts` command line parameter), and make the request again. This
> forces the operation to succeed, changes the value of the field, and removes the field from
> all other managers' entries in `managedFields`.

The status code is 409. `k8s.io/apimachinery/pkg/api/errors/errors.go` (master, fetched
2026-09-12, <https://github.com/kubernetes/apimachinery/blob/master/pkg/api/errors/errors.go>):

```go
func NewApplyConflict(causes []metav1.StatusCause, message string) *StatusError {
	return &StatusError{ErrStatus: metav1.Status{
		Status: metav1.StatusFailure,
		Code:   http.StatusConflict,
		Reason: metav1.StatusReasonConflict,
```

reached from `NewConflictError` in
`k8s.io/apimachinery/pkg/util/managedfields/internal/conflict.go`, which formats
`"Apply failed with %d conflicts: …"`.

**A duplicate inside one request is not that.** `kubernetes-sigs/structured-merge-diff`,
`typed/validate.go`, `visitListItems`
(<https://github.com/kubernetes-sigs/structured-merge-diff/blob/master/typed/validate.go>):

```go
if observedKeys.Has(pe) && !v.allowDuplicates {
        errs = append(errs, errorf("duplicate entries for key %v", pe.String())...)
}
observedKeys.Insert(pe)
```

`allowDuplicates` is set `false` for every ordinary walk. This is a *validation* error, not a
conflict: it travels as `Invalid`, which `errors.go` maps to
`Code: http.StatusUnprocessableEntity` — 422. There is no force flag for it, because there is
nothing to force: the request contradicts itself.

**And the conventions say what each code means to the caller.** From
`kubernetes/community`, `contributors/devel/sig-architecture/api-conventions.md`
(<https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md>):

> `409 StatusConflict` … Suggested client recovery behavior: … GET and compare the fields in
> the pre-existing object, merge changes (if still valid according to preconditions), and
> retry with the updated request (including `ResourceVersion`).
>
> `422 StatusUnprocessableEntity` — Indicates that the requested create or update operation
> cannot be completed due to invalid data provided as part of the request. Suggested client
> recovery behavior: Do not retry. Fix the request.
>
> `500 StatusInternalServerError` — Indicates that the server can be reached and understood
> the request, but either an unexpected internal error occurred and the outcome of the call is
> unknown… Suggested client recovery behavior: Retry with exponential backoff.

**Reading**: this is the clearest source in the set, because it separates the two things that
"conflict" runs together. A clash between two *parties* over one field is 409 and overridable.
A duplicate key inside *one* party's request is 422, "Do not retry. Fix the request." xlsplice
has no second party — there is one batch from one caller — so a repeated target falls on the
422 side. And note what 500 means to a machine caller: retry with backoff. That is the wrong
instruction to give the wrapper or the agent about a batch that will fail identically forever.

---

## 7. DynamoDB: the batch API and the transactional API both refuse duplicate targets

**`BatchWriteItem`**
(<https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_BatchWriteItem.html>).
The API is deliberately *not* atomic — "The individual `PutItem` and `DeleteItem` operations
specified in `BatchWriteItem` are atomic; however `BatchWriteItem` as a whole is not" — and
it processes the requests **in parallel**: "`BatchWriteItem` performs the specified put and
delete operations in parallel". Having given up ordering, it must refuse anything whose
result would depend on order:

> If one or more of the following is true, DynamoDB rejects the entire batch write operation:
> - One or more tables specified in the `BatchWriteItem` request does not exist.
> - Primary key attributes specified on an item in the request do not match those in the
>   corresponding table's primary key schema.
> - **You try to perform multiple operations on the same item in the same `BatchWriteItem`
>   request. For example, you cannot put and delete the same item in the same
>   `BatchWriteItem` request.**
> - **Your request contains at least two items with identical hash and range keys (which
>   essentially is two put operations).**
> - There are more than 25 requests in the batch.
> …

Note the fourth bullet: two *puts* of the same key are refused even though they are not
strictly contradictory. The rule is about the target, not about the values.

**`TransactWriteItems`**
(<https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_TransactWriteItems.html>),
which *is* atomic and *is* an ordered array, refuses just the same:

> `TransactWriteItems` is a synchronous write operation that groups up to 100 action requests.
> These actions can target items in different tables, but not in different AWS accounts or
> Regions, and **no two actions can target the same item**. For example, you cannot both
> `ConditionCheck` and `Update` the same item.

and under `TransactionCanceledException`, among the circumstances in which "DynamoDB cancels
a `TransactWriteItems` request":

> - **More than one action in the `TransactWriteItems` operation targets the same item.**

This is the closest structural analogue to xlsplice's batch in the whole set: an ordered
array of typed operations, applied whole or not at all, against one snapshot, with per-item
results returned in request order. It refuses.

**The class of error is input validation.** `TransactionCanceledException` is **HTTP 400**.
The generic `ValidationError`, from Common Error Types
(<https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/CommonErrors.html>), is
"The input doesn't meet the required format or constraints. Check that all required parameters
are included and that values are valid. HTTP Status Code: 400". `InternalFailure` —
"The request can't be processed right now because of an internal server issue. Try again
later." — is 500, and duplicate targets are nowhere near it. DynamoDB *has* a conflict
vocabulary (`TransactionConflict`, `ReplicatedWriteConflictException`) and reserves it for
clashes with *other* in-flight writers, exactly as Kubernetes does.

---

## 8. SQL: a cardinality violation, and the standard requires it

**`INSERT ... ON CONFLICT DO UPDATE`**, PostgreSQL 18, Parameters → `ON CONFLICT` Clause
(<https://www.postgresql.org/docs/current/sql-insert.html>):

> `INSERT` with an `ON CONFLICT DO UPDATE` clause is a "deterministic" statement. This means
> that the command will not be allowed to affect any single existing row more than once; a
> cardinality violation error will be raised when this situation arises. Rows proposed for
> insertion should not duplicate each other in terms of attributes constrained by an arbiter
> index or constraint.

**`MERGE`**, Notes (<https://www.postgresql.org/docs/current/sql-merge.html>):

> You should ensure that the join produces at most one candidate change row for each target
> row. In other words, a target row shouldn't join to more than one data source row. If it
> does, then only one of the candidate change rows will be used to modify the target row;
> later attempts to modify the row will cause an error. … If the repeated action is an
> `INSERT`, this will cause a uniqueness violation, while a repeated `UPDATE` or `DELETE` will
> cause a cardinality violation; **the latter behavior is required by the SQL standard**.

Note that `MERGE` *does* define an order for its `WHEN` clauses, from the Description:

> For each candidate change row, the status of `MATCHED`, `NOT MATCHED BY SOURCE`, or
> `NOT MATCHED [BY TARGET]` is set just once, after which `WHEN` clauses are evaluated in the
> order specified. For each candidate change row, the first clause to evaluate as true is
> executed. No more than one `WHEN` clause is executed for any candidate change row.

So SQL is ordered *and* refuses a second write to one row in one statement. Ordering settles
which rule applies; it does not license touching a row twice.

**The class matters here too.** PostgreSQL's error-code appendix
(<https://www.postgresql.org/docs/current/errcodes-appendix.html>) puts
`cardinality_violation` at SQLSTATE `21000` in **Class 21 — Cardinality Violation**, one of
the caller-facing classes, and keeps **Class XX — Internal Error** (`XX000 internal_error`,
`XX001 data_corrupted`, `XX002 index_corrupted`) entirely separate. A database that repeatedly
told callers "internal error" for a statement they wrote wrong would be considered broken.

The unremarkable contrast is worth stating once: two `UPDATE` statements on the same row
inside one transaction are fine, and the second wins. The difference is that they are two
statements, each evaluated against the state the previous one produced. Within a single
statement — one snapshot, one pass — a repeated target is an error. xlsplice's batch is one
statement, not two.

---

## 9. POSIX `patch`: "did not fully apply" is not "an error occurred"

IEEE Std 1003.1-2024, `patch`
(<https://pubs.opengroup.org/onlinepubs/9799919799/utilities/patch.html>).

EXIT STATUS, in full:

> The following exit values shall be returned:
> - **0** Successful completion.
> - **1** One or more lines were written to a reject file.
> - **>1** An error occurred.

And from EXTENDED DESCRIPTION, immediately before "Filename Determination":

> Each hunk within a patch shall be the diff output to change a line range within the original
> file. **The line numbers for successive hunks within a patch shall occur in ascending
> order.**

**Reading**: two points. First, the input format itself is normatively required to be
ordered and non-overlapping — the situation this note is about is not a legal patch file.
Second, POSIX spends a whole exit value on the distinction between "your input did not apply"
and "an error occurred". Even a utility with three exit values thought that distinction worth
paying for. xlsplice has six and is currently spending `internal` on the first case.

---

## 10. The trade-off, for this tool and these two callers

The generic case for last-write-wins is composition: a caller assembling a batch from several
places (defaults, then a per-run override) never has to de-duplicate. The generic case for
refusing is that a duplicate target is almost always a bug in whatever built the batch, and
silently taking the last one hides it.

For xlsplice, four specifics push hard one way.

**The architecture is a map, not a sequence (§1.1).** Last-write-wins is not free here. Either
the duplicate targets are collapsed before splices are computed — which *is* the refusal
check, plus a policy of silently dropping the earlier operation — or operations become
sequential, which means re-parsing the part per operation and giving up ADR-0002's
one-tree-per-part rule and the apply-from-the-end-backwards discipline that depends on it.
The cheap implementation of "last write wins" is indistinguishable in code from the refusal
check, and strictly worse in what it tells the caller.

**The wrapper's duplicate targets are its bugs.** The Python program hydrates a template from
a mapping of name to value. A duplicate target there means two entries resolved to one cell —
two defined names anchored at the same cell, an A1 address that collides with a name, a row
expansion that overlapped. Excel will happily hold either value, so the wrapper will never
notice; the workbook will just be quietly wrong, and the vendor add-in will read it. This is
the same class of harm as writing over a template formula, and the spec already chose refusal
there ("a mis-addressed write cannot silently destroy a template formula", story 20), with an
explicit flag to license it. Consistency argues for the same posture.

Note that the duplicate has to be detected on the **resolved** address, not on the target
string: `Sheet1!A1`, `sheet1!$A$1` and a defined name anchored at `Sheet1!A1` are one cell, and
the wrapper's realistic mistake is exactly that kind of aliasing. `batch::run` already resolves
every target to a part and an address before computing anything (`src/batch.rs:156`), so the
information is in hand.

**The agent needs to be told, not accommodated.** Spec story 42: error messages that "name the
fix… so that I can correct myself without a second round trip". An agent that composed a batch
with a repeated target and got exit 0 learns nothing and writes a wrong workbook. One that
gets "operations 2 and 7 both write Inputs!B4; a batch may name a cell once — drop one or merge
them" fixes its batch and moves on. This is the machine-first case, and it is the reason a
skill file can state one rule rather than a resolution order.

**Exit 1 is actively harmful to both callers.** `internal` is defined as "Unexpected failure,
including a caught panic". The wrapper maps exit codes to exception classes (story 41); mapping
its own malformed batch onto the tool's crash class means the traceback points at xlsplice.
For the agent it is worse: the whole convention, stated plainly in Kubernetes' API conventions
and in DynamoDB's retry guidance, is that the internal class means *retry with backoff* and the
caller class means *do not retry, fix the request*. Exit 1 tells a machine caller to try again
or to file a bug against this tool. Both are wrong and both waste a round trip.

The honest cost of refusing: a caller that genuinely wanted to layer a batch must collapse its
own duplicates before calling. That is a few lines of Python over a dict, and it puts the
decision about which value wins where the knowledge lives.

---

## 11. Recommendation

**Refuse the batch. Exit 2, `usage`.**

**1. Detect it in the whole-batch validation, on the resolved address.** Immediately after the
resolve loop in `batch::run` (`src/batch.rs:156-164`), before any part is read, walk the
`ResolvedOperation`s keeping a map from `(at.part, at.address.cell)` to the index that first
claimed it. Two operations landing on one cell fail the batch. Nothing has been spliced and
nothing written, which the exit-code table already guarantees.

**2. The code is `usage`, exit 2, not `refused`, exit 4.** The `refused` family is one thing —
the formula guard, the shared-formula master, a defined name that is not a reference — and its
shape is "the batch is well formed and the target is real, but something about *the package*
says no, and there is or will be a flag to override it". A duplicate target has nothing to do
with the package: the same batch is wrong against every package. It is a defect in the document
the caller supplied, which is what RFC 5789 §2.2 calls a malformed patch document ("The
definition of badly formatted depends on the patch document chosen") and what DynamoDB and
Kubernetes both classify as input validation rather than conflict. Issue #8 has already put
this kind of thing in `usage`: "an unknown `op` is a usage error listing the known kinds".
Duplicate targets belong in the same family, and keeping them together is what lets the skill
file say one thing about exit 2: *the batch you handed me is wrong; fix it and call again*.

The objection that exit 2 means "the command line was wrong" and the batch may arrive on stdin
is real but already settled by #8's unknown-`op` rule. Worth saying in the help topic that exit
2 covers the batch document as well as the command line.

**3. Refuse regardless of the values.** Two `set` operations writing the *same* value to one
cell are also refused. DynamoDB does this explicitly ("at least two items with identical hash
and range keys"), PostgreSQL does it ("Rows proposed for insertion should not duplicate each
other"), and the reason is the same: the rule has to be statable in one sentence for an agent
to follow it, and a duplicate is evidence the batch was built wrong even when it is harmless
this time. ADR-0003's idempotence promise is about *re-running* a batch, not about repetition
*inside* one.

**4. The message names both indices and the canonical address.** The spec requires a failure to
name the failing operation by index; a duplicate has two. Name both, and name the cell in the
package's own spelling, because the common case is aliasing that the caller cannot see —
`operations 2 and 7 both write Sheet1!B4 (operation 7 through the defined name 'Total'); a
batch names each cell once`.

**5. No force flag in v1.** Nothing in the spec asks for one, the exit-code table is frozen,
and a flag is additive later. The condition under which to add it: if the wrapper ever has a
genuine layering case it cannot resolve itself. If it is ever added, follow Kubernetes and make
it explicit and per-invocation (`--last-write-wins`), not per-operation, and keep the refusal
the default.

**6. Leave the overlap guard at `src/splice.rs:74` alone, and leave it `internal`.** Once
duplicate targets are refused upstream, two splices overlapping means xlsplice computed two
byte ranges wrongly for two different cells, which is a bug in this tool and is what `internal`
is for. Its doc comment currently explains the check as "a caller asking for two different
things in the same place" (`src/splice.rs:12-15`) — after this change that is no longer the
reachable cause, and the comment should say so. Add a line recording that the guard does not
catch two coincident *empty* ranges and must not be relied on to catch duplicate targets; §1.2
is the evidence.

### Where the sources disagree, and what it depends on

They do disagree, and it would be dishonest to file RFC 6902 under "refuse". JSON Patch defines
sequential application over an evolving document and never forbids a repeated path; git apply
deliberately supports a second diff for a path in the same input. Against them, DynamoDB
(batch and transactional), SQL (`ON CONFLICT`, `MERGE`, and the standard behind it), and
Kubernetes' structured merge all refuse. RFC 7396 and RFC 5789 do not take a position on
duplicates at all; 7396 cannot express the case and 5789 only says what class of failure any
refusal falls into.

**X, the thing it depends on: whether a batch is a sequence applied to an evolving package, or
a set of edits computed against one snapshot.**

Every source that refuses is in the second family: one snapshot, one pass, targets identified
by key. Every source that applies in order is in the first, and each of them gives the caller
a way to *state* what it expects the earlier operations to have left behind — JSON Patch's
`test` and the remove-then-add definition of `replace`, a diff hunk's context lines. The probe
in §5 is the proof that this is the real axis rather than a taxonomy: git accepts the
sequential patch and refuses the contradictory one using nothing but the context lines.

xlsplice is squarely in the second family today, by construction and by ADR-0002, and its `set`
carries no assertion about prior state. That is why it should refuse. If a later ticket makes
operations genuinely depend on one another — a `clear` followed by a `set` on one cell, or an
insertion whose effect changes what a later target resolves to — the premise changes and so
does the answer, and at that point xlsplice would need something in the operation shape that
lets a caller say what it expected to find. An ADR recording "the batch is a set of edits over
one snapshot, so a target appears at most once" would make that a conscious revision rather
than a drift.

---

## 12. What it would cost to implement

Small. The information the check needs is already computed.

**Library** (`src/batch.rs`), roughly ten lines plus the message. After the resolve loop, fold
the `ResolvedOperation`s into a `BTreeMap<(String, Cell), usize>`; on a second claim, return
`Error::usage` naming both indices, the canonical address and, where either went through a
defined name, the name. `Resolution` already carries `part`, `address` and `name`,
`Address` already renders in the package's own spelling, and `Cell` already derives
`Eq` and `Ord` (`src/reference.rs:18`), so it is usable as a key with no change.

The check sits between the resolve loop and the `parts_of` loop, so it runs before any part is
read and long before `land`. Nothing about the atomicity or the "nothing was written"
guarantee needs touching.

**Doc comments**: the note in `src/splice.rs`'s module header that overlapping splices now mean
a bug in xlsplice rather than a caller asking twice, plus the line about coincident empty
ranges.

**Tests.** Five, all in the existing shapes:

- Library: two `set` operations on one cell in one batch → `usage`; the input package is
  byte-identical and no output exists.
- Library: an address and a defined name that resolve to the same cell → `usage`. This is the
  one that earns its keep; it is the wrapper's realistic bug and the string-comparison
  implementation would miss it.
- Library: two `set` operations on a cell written `<c r="A1"></c>` → `usage`. This is §1.2's
  hole, and it is the test that proves the check is not just a nicer spelling of the overlap
  guard. It needs a hand-built worksheet part rather than a fixture, since Excel does not write
  that shape; the library takes a parsed part, so no fixture has to be invented.
- Library: two operations on *different* cells in one part still apply, and the part carries
  both splices. Guards against an over-broad key.
- Binary contract: an `apply` batch with a duplicate target exits 2 and, under `--json`, writes
  the envelope with `error.code` `usage` and a message naming both indices.

**Documentation to follow.** The `apply` schema section of the skill file gains one line — a
batch names each cell at most once — and the exit-codes help topic gains the batch document to
its description of exit 2. Both are one sentence.

**Not required, and cheaper to skip**: any change to `splice::apply`, to the report shape, to
the envelope, or to the exit-code table. The table stays frozen; this uses a code that is
already in it.

---

## Sources

RFCs (rfc-editor.org, fetched 2026-09-12, raw `.txt`):
- RFC 6902, *JavaScript Object Notation (JSON) Patch*, April 2013 — §3 Document Structure,
  §4 Operations, §4.1 add, §4.3 replace, §4.6 test, §5 Error Handling.
  <https://www.rfc-editor.org/rfc/rfc6902.txt>
- RFC 7396, *JSON Merge Patch*, October 2014, **Obsoletes RFC 7386** — §1 Introduction,
  §2 Processing Merge Patch Documents. <https://www.rfc-editor.org/rfc/rfc7396.txt>;
  the obsoleted RFC 7386 <https://www.rfc-editor.org/rfc/rfc7386.txt>
- RFC 5789, *PATCH Method for HTTP*, March 2010 — §2 The PATCH Method, §2.2 Error Handling.
  <https://www.rfc-editor.org/rfc/rfc5789.txt>
- RFC 8259, *The JavaScript Object Notation (JSON) Data Interchange Format*, December 2017 —
  §4 Objects. <https://www.rfc-editor.org/rfc/rfc8259.txt>

git (git.kernel.org plain text, fetched 2026-09-12; rendered man page at
<https://git-scm.com/docs/git-apply>):
- `Documentation/git-apply.adoc` — `--reject`, `--unidiff-zero`.
  <https://git.kernel.org/pub/scm/git/git.git/plain/Documentation/git-apply.adoc>
- `apply.c` — `in_fn_table` / `add_to_fn_table` / `prepare_fn_table` and the
  `PATH_TO_BE_DELETED` comment; `previous_patch`; `load_preimage` ("We have a patched copy in
  memory; use that."); `check_patch`'s `"%s: patch does not apply"`; `apply_one_fragment`'s
  `"patch failed: %s:%ld"`; `apply_patch`'s `check_patch_list` → `write_out_results`.
  <https://git.kernel.org/pub/scm/git/git.git/plain/apply.c>

Kubernetes:
- Server-Side Apply reference — Conflicts, Field management, Managers.
  <https://kubernetes.io/docs/reference/using-api/server-side-apply/>
- `kubernetes/apimachinery` (master, fetched 2026-09-12): `pkg/api/errors/errors.go`
  (`NewConflict`, `NewApplyConflict` → `http.StatusConflict`; `NewInvalid` →
  `http.StatusUnprocessableEntity`);
  `pkg/apis/meta/v1/types.go` (`StatusReasonConflict`, "Status code 409");
  `pkg/util/managedfields/internal/conflict.go` (`NewConflictError`).
  <https://github.com/kubernetes/apimachinery>
- `kubernetes-sigs/structured-merge-diff` (master): `typed/validate.go`, `visitListItems` —
  `"duplicate entries for key %v"`, `allowDuplicates` default false.
  <https://github.com/kubernetes-sigs/structured-merge-diff>
- `kubernetes/community`, `contributors/devel/sig-architecture/api-conventions.md` — the HTTP
  status-code list (409, 422, 500) and the `Conflict` / `Invalid` / `InternalError` reasons.
  <https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md>

AWS DynamoDB API reference:
- `BatchWriteItem` — "If one or more of the following is true, DynamoDB rejects the entire
  batch write operation".
  <https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_BatchWriteItem.html>
- `TransactWriteItems` — "no two actions can target the same item";
  `TransactionCanceledException`, HTTP 400.
  <https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_TransactWriteItems.html>
- Common Error Types — `ValidationError` (400), `InternalFailure` (500).
  <https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/CommonErrors.html>

PostgreSQL (current, = 18):
- `INSERT`, Parameters → `ON CONFLICT` Clause.
  <https://www.postgresql.org/docs/current/sql-insert.html>
- `MERGE`, Description and Notes. <https://www.postgresql.org/docs/current/sql-merge.html>
- Appendix A, PostgreSQL Error Codes — Class 21 `cardinality_violation` `21000`; Class XX
  `internal_error` `XX000`. <https://www.postgresql.org/docs/current/errcodes-appendix.html>

POSIX:
- IEEE Std 1003.1-2024, `patch` — EXIT STATUS; EXTENDED DESCRIPTION (hunk line numbers "shall
  occur in ascending order").
  <https://pubs.opengroup.org/onlinepubs/9799919799/utilities/patch.html>

This repository (read, not modified): `CONTEXT.md`; `docs/adr/0001-splice-parts-never-reserialise.md`,
`0002-own-parser-over-roxmltree-not-calamine.md`, `0003-a-batch-that-changes-nothing-writes-nothing.md`;
`src/batch.rs`, `src/splice.rs`, `src/worksheet.rs`, `src/error.rs`, `src/lib.rs`; issues #2 and #8
via `gh issue view`.

Probes (session scratchpad
`/private/tmp/claude-501/-Users-niko86-sources-rust-xlsplice/db03e715-5f07-493c-a105-01ca2c186aa8/scratchpad/`):
`/tmp/gitprobe` (git 2.50.1, three patches `p.diff`, `q.diff`, `r.diff` over a five-line file,
run with and without `--unidiff-zero`); `probe/` (a scratch crate with a path dependency on this
repository, driving `batch::run` over a copy of `plain.xlsx` and `worksheet::value_splices` +
`splice::apply` over six hand-built cell shapes). No file in this repository was written and no
fixture was touched.
