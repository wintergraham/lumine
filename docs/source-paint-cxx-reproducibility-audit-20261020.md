# Source paint C++ reproducibility audit

This receipt is always unqualified. `executionClosureComplete`,
`originalProductionCommandsObserved`, `acceptance`, and `activationQualified`
remain false. It grants no source paint capability or full-page acceptance.

The normal `cc::Build::compile` execution and link metadata remain unchanged.
After the original v1 recipe has been written, a separate bounded audit uses
fresh actual `cc::Tool::to_command` and `Build::get_archiver` commands. The old v1
recipe/API remains unchanged, including its unreviewed `LC_ALL` observation.
The new audit records explicit `LC_ALL` only for the reviewed literal `C`,
`C.UTF-8`, or `POSIX`, and `ZERO_AR_DATE` only as `1`. Other explicit environment
values are neither logged nor executed by the audit; normal compilation remains
available. The inherited environment is preserved and never dumped.

The audit executes a GCC `-M` preflight, hashes every reported regular source and
system header, then actually compiles with `-MD -MF` and `-v`. It requires exact
canonical dependency-set equality and unchanged pre/post header bytes. The
compiler's reported assembler discovery is labelled discovery; only actual
verbose compile invocations of the same prehashed programs establish
compiler-reported use. This is not kernel exec or file-read tracing.

The audit object keeps the production object basename and must have exactly the
same bytes as the actual production object. Actual configured archiver commands
create and index a separate deterministic archive using the audit object. The
archive must be byte-identical to the actual production archive. A mismatch
leaves the audit incomplete; flags and profiles are never changed to force a
match. Configured compiler/archiver identities, production object/archive,
production recipe and complete recorded runtime input inventory are checked
again after the audit. Runtime inventory must also match the v1 recipe's known
canonical serialization before the audit begins.

Each actual audit command records its real program/arguments/cwd, reviewed
explicit environment, pre/post executable hashes, status, duration and bounded
raw stdout/stderr files. Empty process streams are genuine regular zero-byte
files and have SHA256 digests. Compiler/helper executables are bounded to
128 MiB each; headers to 1,024 files and 128 MiB total; each captured stream to
2 MiB. The audit has a 120-second total execution budget and a 60-second command
budget. Excess output/deadline kills only the audit's own process group.

The runtime inspection APIs are separate from the v1 recipe:

- `source_paint_execution_audit_json() -> &'static str`
- `source_paint_execution_audit_sha256() -> Option<&'static str>`

Independent production verification must recompute the raw JSON digest, inspect
all regular bounded referenced files, and verify every byte before and after
use. The current absolute output/header/tool paths are live build-cache proof,
not portable artifact references. Retiring or rebuilding that cache invalidates
those references unless a separately authenticated preservation mapping exists.

The remaining gaps are explicit: original cc command execution, compiler shared
libraries, archiver plugins, final native linker and loaded C++ standard library
closure are not established. GCC `-v`, `-M`, `-MD`, program-name discovery and
byte-identical recompilation do not close these gaps. Audit reproducibility does
not upgrade any source-rendering qualification or acceptance.

Validation: direct metadata compilation against the actual Host-selected cached
cc 1.5.1; eight std-only unit tests (including real process output/deadline
bounds); one separately guarded tiny genuine GNU compile/archive test that also
rejects a changed header and unreviewed secret environment value. No Cargo/GPU or
source browser was run by the worker. The actual production audit and renderer
regressions remain the parent's fresh native-build responsibility.
