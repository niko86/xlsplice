# Machine-first CLI conventions, as practised by the tools that do it well

Research note, 2026-09-11. Primary sources only: official documentation, the
tools' own source code, and the authors' published design guidelines. Every
claim carries a URL; source-code claims name the file and the commit or version
read. Anything that could not be confirmed against such a source is marked
**unverified**. Quotations were checked against raw copies of the pages fetched
on 2026-09-11, not against search-engine summaries.

Versions read: `gh version 2.100.0 (2026-09-03)` installed locally (`gh help
formatting`, `gh help exit-codes`, `gh help environment` were run on this Mac);
cli/cli source at `trunk` commit `7b2de63c49c0d6717930415d2100a56103360509`
(2026-09-10); clap 4.6.x docs on docs.rs; cargo book current; git-scm.com
current; FreeBSD 15.1 `sysexits(3)` and the local macOS `sysexits(3)`.

The xlsplice design being tested (newer than the 2026-09-11 handoff): verbs
`sheets`, `names`, `get`, `set`, `clear`, `apply` (JSON batch on stdin or
file), `props get/set/unset`, `calc`, `diff`; package path first; `--json` for
structured output; writes accept `--out`, otherwise in place via temp file and
rename; exit codes 0 ok, 1 unexpected failure, 2 usage, 3 not found, 4 refused
(a guard said no), 5 package unreadable; any non-zero code guarantees the file
was not written; deterministic output, no interactivity, no resident process.

---

## Recommendations for xlsplice

Each line maps a practice to a decision. Citations point at the evidence
sections below; "(synthesis)" marks a decision the sources do not settle.

**Streams.** Data to stdout, diagnostics to stderr, never mixed. clig: "Send
output to stdout... Anything that is machine readable should also go to
stdout" and "Send messaging to stderr" (§2). gh's contributor rule: "Keep data
on the command's established stdout path and diagnostics on its stderr path;
do not merge streams or leak interactive decoration into pipes" (§1.8). Cargo
warns consumers to "only interpret a line as JSON if it starts with `{`"
because it cannot keep other tools off stdout (§4); xlsplice can, so in
`--json` mode stdout is exactly one JSON document and nothing else.

**`--json` is a global boolean flag** (`#[arg(global = true)]`, §9.1). gh's
`--json` takes a field list because the fields drive its GraphQL query (PR
#3414, §1.3); xlsplice has no per-field cost, so no field list. kubectl and
docker use `-o`/`--format` enums (§5); keep `--format` as an additive future
flag if YAML or templates are ever wanted (clig: "Keep changes additive", §2).
Do not embed jq or Go templates: the callers are a Python program and an agent
with jq available.

**One JSON document per invocation, not JSON Lines.** gh emits a single
array/object; kubectl wraps lists in `items`; cargo, rustc, Codex and Claude
Code stream one object per line only because their runs are long and produce
events over time, and cargo needs a `build-finished` sentinel so tools "know
when to stop reading" (§4, §6). xlsplice commands are short and atomic. Batch
results from `apply` go in a `results` array inside the one object. If a
streaming verb ever appears, add `--jsonl` additively; JSONL remains trivially
consumable (jsonlines.org, nushell `from json --objects`, §6).

**Errors under `--json` are JSON on stdout, same exit code.** Practice splits:
gh prints errors as text on stderr even with `--json` and never emits a JSON
error (§1.4, §8.1); npm merges an `error` object into the stdout JSON and its
maintainers regret not having keyed all output under `result`/`error` from
the start (§8.3); Claude Code `-p` prints in-run failures "as the result on
stdout" (§10.2); Codex `--json` emits `error` events on the same stdout
stream (§10.5). Follow npm's stated regret rather than its current shape:
every `--json` response is an envelope `{"ok": true|false, "schema_version":
1, ...}` with `"error": {"code": "<stable_snake_case>", "message": "<human
text naming the fix>"}` when `ok` is false, and nothing else on stdout. The
exit code is unchanged by `--json`. Usage errors from clap must be caught
(`Command::try_get_matches`) and rendered in the same envelope when `--json`
is present in argv, otherwise clap's default text on stderr with exit 2 (§9.2).
(synthesis on the envelope keys; the evidence is npm's comment and cargo's
`success` field.)

**Error text must say what to do next.** Anthropic's tool-design guidance:
"prompt-engineer your error responses to clearly communicate specific and
actionable improvements, rather than opaque error codes or tracebacks"
(§10.4). gh, when it detects an invoking agent, prints the full help on stderr
after a usage error "giving AI agents the examples, JSON fields and
environment variables they need to correct themselves without a second round
trip" (§1.4). So: a refusal (exit 4) names the flag that overrides it; a "not
found" (exit 3) lists what was searched (sheet names, defined names); usage
errors append the usage line.

**Exit codes: keep the small-integer table, publish it, freeze it.** Nobody
well-regarded uses `sysexits.h` (FreeBSD 15.1 now marks it "deprecated...
Its use is discouraged", §7.1): gh uses 0/1/2/4 (and an undocumented 8), cargo
0/101, jj 0/1/2/3/255, clap 2 for usage (§7). 2 for usage matches clap, bash
builtins and grep (§7.2, §9.2). The 0/1/2/3/4/5 table is fine; document it in a
help topic (`xlsplice help exit-codes`, gh's pattern, §1.2) and in `--help`,
and treat changes as breaking. gh's own caveat is worth copying verbatim into
the help topic: commands may add codes, so callers should check the docs.

**`diff` must exit 0 whether or not the packages differ** and report the
result in its output; add `--exit-code` for `diff(1)` semantics on request,
as git does. Claude Code's Bash tool: "Every other command that exits 1 counts
as a failure, even when exit 1 is a benign informational outcome" and a
skill's `!` injected command aborts the whole skill on any non-zero exit
(§10.3). This is the one place the current design (1 = failure) collides with
a plausible verb semantics.

**`apply` reads stdin only when told `-`.** POSIX Guideline 13 and clig: "If
input or output is a file, support `-` to read from stdin" (§2, §7.2); clap's
lexer recognises `-` as "a stdio argument" and passes it through as a value
(§9.3). Do not fall back to stdin when the file operand is absent: an agent
that forgot the pipe would hang on a TTY. If `-` is given and stdin is a TTY,
fail with exit 2 and say so (synthesis; clig's TTY-on-stdin rule for prompts
is the analogue).

**`--` is accepted and documented.** POSIX Guideline 10; clap handles the
escape natively (§9.3). Defined names contain `?` and dots, and a package
path can start with `-`, so the skill file should show `--` before operands.

**No colour in `--json` mode; otherwise `--color auto|always|never` plus
`NO_COLOR`.** clig lists the four disable conditions (not a TTY, `NO_COLOR`
non-empty, `TERM=dumb`, `--no-color`), no-color.org says a command-line flag
overrides the variable (§2, §9.4). Use `anstream` (clap's `color` feature
already pulls it in): its `Auto` order is `NO_COLOR` → never, `CLICOLOR_FORCE`
→ always, `CLICOLOR=0` → never, else colour only if `is_terminal()` and
`TERM` is not `dumb` (§9.4). Note gh resolves the same variables in the
opposite precedence (`CLICOLOR_FORCE` beats `NO_COLOR`, §1.5); follow
anstream, the Rust default, and let `--color` be the override the spec
requires.

**TTY detection changes only decoration.** gh's primer for piped output: "No
color or styling; State is explicitly written, not implied from color; Tabs
between columns...; No truncation; Exact date format; No header" (§1.6).
Apply that to `sheets`/`names`/`diff` human output: aligned table with header
on a TTY, TSV without header otherwise. No pager (clig: only if interactive;
output is small), no spinner, no prompt, no update check, no telemetry: gh
gates every one of these on a TTY anyway (§1.5, §1.7), so a tool without them
loses nothing. Do not add a `GH_FORCE_TTY` analogue until a human asks.

**Stability contract: `--json` is the stable interface; human output is not.**
git: porcelain "will remain stable across Git versions and regardless of user
configuration" while `--short` is free to change (§3.1); clig: "Encourage
your users to use `--plain` or `--json` in scripts to keep output stable"
(§2); gh: "Preserve JSON field names, types, and empty-result behavior
independently of terminal rendering" (§1.8). Skip `--plain`: the TSV non-TTY
form covers `cut`/`awk` and the stable path is JSON.

**Schema versioning: `schema_version: 1` in every envelope, additive changes
allowed, no version flag until a v2 exists.** cargo metadata: "The format is
stable and versioned... pass `--format-version` flag explicitly to avoid
forward incompatibility hazard", with adding fields and adding enum values
declared compatible (§4.2); rustc: "New fields may be added. Enumerated fields
... may add new values" (§4.3); git added `--porcelain=v2` seven years after
v1, kept v1 as the default, and told parsers to "ignore headers they don't
recognize" (§3.2). Document that consumers must ignore unknown fields and
unknown `error.code` values.

**Bound output size.** Claude Code returns Bash output inline "up to roughly
30,000 characters"; Anthropic restricts tool responses "to 25,000 tokens by
default" and recommends "pagination, range selection, filtering, and/or
truncation with sensible default parameter values" (§10.3, §10.4). `get` on a
range and `diff` need limits: `diff` reports per-part summary by default and
per-node detail on request; compact (not pretty-printed) JSON when stdout is
not a TTY, as gh does (§1.4).

**`--dry-run` on every writing verb; `--quiet`/`--verbose` touch stderr
only.** clig's standard flag list has `-n, --dry-run` and `-q, --quiet` (§2).
`apply --dry-run` validates the whole batch and returns the same envelope
with `"dry_run": true` (synthesis). gh's `GH_DEBUG` sends verbose output to
stderr (§1.7); do the same with `--verbose`.

**Atomic writes are the crash-only design clig asks for**, and the guarantee
"non-zero means not written" is exactly what OfficeCLI broke (#390, handoff).
Claude Code kills the Bash process tree on SIGTERM (§10.2); temp-file-and-
rename means a killed xlsplice leaves the package intact. Use unique temp
names and either delete stale ones on the next run or leave them (clig: defer
cleanup to the next run, §2).

**Idempotence and determinism.** clig: "operations should be idempotent where
possible" (§2). `set` with the value already present should produce
byte-identical output and report `"changed": false` (synthesis). Zip entry
timestamps must be copied, not regenerated, or repeated runs differ.

**`--help` with examples; `help` topics for `json` and `exit-codes`.** clig's
help rules and gh's `gh help formatting`/`exit-codes`/`environment` topics
(§1.1, §1.2, §2). Anthropic's skill guidance shows scripts documented with
their exact invocation and their output shape (§10.1), so the skill file
should carry one example per verb plus the envelope.

**`--version --json`: no source practises it.** gh, cargo, git print text
(unverified for Codex and Claude Code). Recommend `xlsplice version` as a verb
that honours the global `--json`, and leave clap's `--version` as text.

**Shell completions** via `clap_complete` static generation
(`xlsplice completions <shell>`, §9.5); low priority, the human is third.

**Python caller.** `subprocess.run(..., check=True)` raises
`CalledProcessError` carrying "the exit code, and stdout and stderr if they
were captured" (§10.6), so the wrapper maps exit codes to exception classes
and reads the JSON envelope from `e.stdout` on failure.

### Where the sources argue against the current design

1. `diff` cannot signal "differences found" with exit 1 without being counted
   as a failure by Claude Code's Bash tool and skill injection (§10.3). Exit 0
   by default, `--exit-code` opt-in (git's precedent, §3.3).
2. The design does not say how errors look under `--json`. Every JSON-mode tool
   examined either leaves errors as stderr text (gh, cargo) or puts them in the
   stdout document (npm, Claude Code, Codex). Choose the latter with fixed
   envelope keys; npm's source comment is the cautionary tale (§8.3).
3. "`apply` on stdin or file" needs the explicit `-` operand; implicit stdin is
   a hang risk for an agent (§2, §7.2, §9.3).
4. Exit code 1 as "unexpected failure" is conventional (coreutils: "typically
   1", §7.3) but every panic must still produce the envelope and a message:
   install a panic hook that emits `error.code = "internal"` with exit 1
   (jj reserves 255 and cargo 101 for this; a separate code is optional).
5. The design has no schema version and no stated additive-change policy;
   cargo, git and rustc all state one (§3.2, §4.2, §4.3).

---

## 1. GitHub CLI (`gh`)

Local: `gh version 2.100.0 (2026-09-03)`. Source: cli/cli `trunk`
`7b2de63c` (2026-09-10). Manual pages: <https://cli.github.com/manual/>.

### 1.1 `gh help formatting`

Verbatim from the local binary (also
<https://cli.github.com/manual/gh_help_formatting>): "By default, the result
of `gh` commands are output in line-based plain text format. Some commands
support passing the `--json` flag, which converts the output to JSON format.
Once in JSON, the output can be further formatted according to a required
formatting string by adding either the `--jq` or `--template` flag." "The
`--json` flag requires a comma separated list of fields to fetch. To view the
possible JSON field names for a command omit the string argument to the
`--json` flag when you run the command. Note that you must pass the `--json`
flag and field names to use the `--jq` or `--template` flags." "The `jq`
utility does not need to be installed on the system to use this formatting
directive. When connected to a terminal, the output is automatically
pretty-printed." The `--template` section lists Go template helpers
(`autocolor`, `color`, `join`, `pluck`, `tablerow`, `tablerender`, `timeago`,
`timefmt`, `truncate`, `hyperlink`) plus four Sprig functions. Example output
shows `gh pr list --json number,title,author` returning a JSON array of
objects with only the requested keys, sorted alphabetically.

### 1.2 `gh help exit-codes`

Verbatim (<https://cli.github.com/manual/gh_help_exit-codes>): "gh follows
normal conventions regarding exit codes. If a command completes successfully,
the exit code will be 0. If a command fails for any reason, the exit code will
be 1. If a command is running but gets cancelled, the exit code will be 2. If
a command requires authentication, the exit code will be 4. NOTE: It is
possible that a particular command may have more exit codes, so it is a good
practice to check documentation for the command if you are relying on exit
codes to control some behavior."

Source (`internal/ghcmd/cmd.go`,
<https://github.com/cli/cli/blob/7b2de63c49c0d6717930415d2100a56103360509/internal/ghcmd/cmd.go>):
`exitOK = 0, exitError = 1, exitCancel = 2, exitAuth = 4, exitPending = 8`.
`exitPending` is not in the help text; `pkg/cmdutil/errors.go` defines
`PendingError` as "signals nothing failed but something is pending" and
`SilentError` as "an error that triggers exit code 1 without any error
messaging". A `NoResultsError` returns exit 0 ("no results is not a command
failure") and its message is printed to stderr only when stdout is a TTY.
Extension and alias exit codes are passed through unchanged. A `FlagError`
"indicates an error processing command-line flags or other arguments. Such
errors cause the application to display the usage message" and exits 1 (gh
does not use 2 for usage; 2 is cancellation).

### 1.3 Why `--json` takes a field list

PR #3414 "Add `--json` export flag for issues and pull requests" (merged
2021-04-14, <https://github.com/cli/cli/pull/3414>): "The `--json` flag
accepts a list of GraphQL fields to query for and output in JSON format. To
get the list of available flags, run the command with a blank value for
`--json`. Additional `--jq` and `--template` flags are available for
post-processing JSON just like in `gh api`." Limitations stated: "Only a
preset of known fields are available for fetching" and nested collections
are capped at 100 records. The field list therefore shapes the API request,
not just the output.

Implementation (`pkg/cmdutil/json_flags.go`, same commit): `--json` is a
`StringSlice` flag "Output JSON with the specified `fields`"; an empty value
produces the error "Specify one or more comma-separated fields for `--json`:"
followed by the sorted field list; an unknown field produces "Unknown JSON
field: %q\nAvailable fields:"; `--jq` and `--template` without `--json` are
errors ("cannot use `--jq` without specifying `--json`"); `--web` conflicts
with `--json`. Field names get shell completion. A second helper
`AddFormatFlags` adds `--format json` (an enum with the single value `json`)
for commands that export whole objects.

### 1.4 What `--json` writes, and how errors look when `--json` is on

`jsonExporter.Write` (same file): encodes with `SetEscapeHTML(false)`; with
`--jq`, indents only `if ios.IsStdoutTTY()`; with `--template`, uses terminal
width and colour; otherwise `else if ios.ColorEnabled() { return
jsoncolor.Write(w, &buf, "  ") }` and finally a raw `io.Copy` of the compact
`encoding/json` output. So piped `--json` output is compact single-line JSON;
pretty-printing and colour exist only when colour is enabled (a TTY).

Errors are never JSON. `cmd.go`'s `printError` writes `err` to stderr,
appends the usage string for flag errors, and returns exit 1; the exporter is
only reached from a command's success path (`docs/command-development.md`:
"return `opts.Exporter.Write(opts.IO, data)` when the exporter is set, before
human-readable output"). `printError`'s comment: "When fullHelp is set the
complete help text is written instead of the terse usage string, giving AI
agents the examples, JSON fields and environment variables they need to
correct themselves without a second round trip", and "Render into out rather
than calling cmd.Help(), which would send the help text to stdout and split a
single failure across two streams." `fullHelp` is `invokingAgent != ""`, from
`internal/agents/detect.go`, which reads `AI_AGENT`, `CLAUDECODE`/
`CLAUDE_CODE`, `CLAUDE_CODE_IS_COWORK`, `CODEX_SANDBOX`/`CODEX_CI`/
`CODEX_THREAD_ID`, `GEMINI_CLI`, `COPILOT_CLI` and others. Agent detection
also disables the spinner (`newIOStreams`).

### 1.5 TTY detection and colour

`pkg/iostreams/iostreams.go`: `IsStdoutTTY()` returns true if
`s.term.IsTerminalOutput()` (comment: "support GH_FORCE_TTY") or the fd is a
Cygwin terminal; `IsStdinTTY`/`IsStderrTTY` test the real descriptors.
`CanPrompt()` is `!neverPrompt && IsStdinTTY() && IsStdoutTTY()`.
`StartPager()` is a no-op when the pager is empty, `cat`, or stdout is not a
TTY; pager precedence is `GH_PAGER`, then config, then `PAGER`; it sets
`LESS=FRX` when unset. Progress indicators need stdout and stderr both TTYs.

go-gh `pkg/term/env.go` (<https://github.com/cli/go-gh/blob/trunk/pkg/term/env.go>):
`FromEnv` reads `GH_FORCE_TTY`, `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`,
`TERM`, `COLORTERM`. With `GH_FORCE_TTY` set: `stdoutIsTTY = true`, colour on
unless disabled, a numeric value is the column width and `NN%` a percentage.
Otherwise `stdoutIsTTY = IsTerminal(os.Stdout)` and `isColorEnabled =
IsColorForced() || (!IsColorDisabled() && stdoutIsTTY)`, where
`IsColorDisabled` is `NO_COLOR != "" || CLICOLOR == "0"` and `IsColorForced`
is `CLICOLOR_FORCE` set and not `"0"`. Hence in gh `CLICOLOR_FORCE` beats
`NO_COLOR`.

### 1.6 What changes when stdout is not a TTY

`pkg/cmd/pr/list/list.go`: the "Showing N of M pull requests" header is
printed only `if opts.IO.IsStdoutTTY()`, and a `STATE` column is added when
not a TTY (state is otherwise implied by colour). `internal/tableprinter/
table_printer.go`: headers are added only `if isTTY`; `AddTimeField` "in TTY
mode displays the fuzzy time difference... In non-TTY mode it just displays t
with the time.RFC3339 format"; the underlying go-gh table printer is created
with `isTTY` and the terminal width. The design primer
(`docs/primer/foundations/README.md#scriptability`): "Create flags for
anything interactive; Ensure flags have clear language and defaults; Consider
what should be different for terminal vs machine output", and the machine
output differences: "No color or styling; State is explicitly written, not
implied from color; Tabs between columns instead of table layout, since `cut`
uses tabs as a delimiter; No truncation; Exact date format; No header".

### 1.7 Environment (`gh help environment`)

Verbatim: "`GH_DEBUG`: set to a truthy value to enable verbose output on
standard error." "`GH_PAGER`, `PAGER` (in order of precedence): a terminal
paging program to send standard output to." "`NO_COLOR`: set to any value to
avoid printing ANSI escape sequences for color output. `CLICOLOR`: set to `0`
to disable printing ANSI colors in output. `CLICOLOR_FORCE`: set to a value
other than `0` to keep ANSI colors in output even when the output is piped."
"`GH_FORCE_TTY`: set to any value to force terminal-style output even when the
output is redirected. When the value is a number, it is interpreted as the
number of columns available in the viewport. When the value is a percentage,
it will be applied against the number of columns available in the current
viewport." "`GH_NO_UPDATE_NOTIFIER`: set to any value to disable GitHub CLI
update notifications. When any command is executed, gh checks for new
versions once every 24 hours. If a newer version was found, an upgrade notice
is displayed on standard error." "`GH_PROMPT_DISABLED`: set to any value to
disable interactive prompting in the terminal." "`GH_SPINNER_DISABLED`: set to
a truthy value to replace the spinner animation with a textual progress
indicator." Telemetry: `GH_TELEMETRY` (`log` prints to stderr, `false`/`0`
disables) and `DO_NOT_TRACK`. Source `internal/update/update.go`
`ShouldCheckForUpdate`: returns false if `GH_NO_UPDATE_NOTIFIER` or
`CODESPACES` is set, else `!ci.IsCI() && IsTerminal(os.Stdout) &&
IsTerminal(os.Stderr)`, so a piped or CI invocation never checks.

### 1.8 gh's own contract for contributors

`docs/command-development.md` (same commit): "Preserve script-facing
contracts unless the agreed change explicitly authorizes breaking them:
flags, arguments, defaults, exit behavior, error messages, JSON fields,
non-TTY output, and stdout/stderr routing. Preserve intended TTY behavior
too." "Keep data on the command's established stdout path and diagnostics on
its stderr path; do not merge streams or leak interactive decoration into
pipes. Non-TTY tables use script-friendly output, not terminal truncation,
color, or headers. Prompts need a non-interactive flag path." "Preserve JSON
field names, types, and empty-result behavior independently of terminal
rendering."

## 2. Command Line Interface Guidelines (clig.dev)

All quotes verbatim from <https://clig.dev/> as fetched 2026-09-11.

Output: "Human-readable output is paramount. Humans come first, machines
second." "The most simple and straightforward heuristic for whether a
particular output stream (stdout or stderr) is being read by a human is
whether or not it's a TTY." "Have machine-readable output where it does not
impact usability." "If human-readable output breaks machine-readable output,
use `--plain` to display output in plain, tabular text format for integration
with tools like `grep` or `awk`." "...you should provide a `--plain` flag for
scripts, which disables all such manipulation and outputs one record per
line." "Display output as formatted JSON if `--json` is passed." "Display
output on success, but keep it brief. Traditionally, when nothing is wrong,
UNIX commands display no output to the user." "Send output to stdout. The
primary output for your command should go to stdout. Anything that is
machine readable should also go to stdout—this is where piping sends things
by default." "Send messaging to stderr. Log messages, errors, and so on should
all be sent to stderr." "Disable color if your program is not in a terminal
or the user requested it. These things should disable colors: stdout or
stderr is not an interactive terminal (a TTY). It's best to individually
check—if you're piping stdout to another program, it's still useful to get
colors on stderr. The `NO_COLOR` environment variable is set and it is not
empty (regardless of its value). The `TERM` environment variable has the
value `dumb`. The user passes the option `--no-color`." "If stdout is not an
interactive terminal, don't display any animations." "Use a pager only if
stdin or stdout is an interactive terminal." "Don't print log level labels
(ERR, WARN, etc.) or extraneous contextual information, unless in verbose
mode."

Errors: "Catch errors and rewrite them for humans." "Signal-to-noise ratio is
crucial."

Arguments and flags: "Return zero exit code on success, non-zero on failure.
Exit codes are how scripts determine whether a program succeeded or failed,
so you should report this correctly. Map the non-zero exit codes to the most
important failure modes." "If input or output is a file, support `-` to read
from stdin or write to stdout." "Prefer flags to args." "Have full-length
versions of all flags." "Use standard names for flags, if there is a
standard." The standard list includes "`-f`, `--force`", "`--json`: Display
JSON output", "`-n`, `--dry-run`: Dry run. Do not run the command, but
describe the changes that would occur if the command were run", "`--no-input`:
See the interactivity section", "`-q`, `--quiet`: Quiet. Display less output".
"If possible, make arguments, flags and subcommands order-independent."

Interactivity: "Only use prompts or interactive elements if stdin is an
interactive terminal (a TTY)." "Never require a prompt. Always provide a way
of passing input with flags or arguments. If stdin is not an interactive
terminal, skip prompting and just require those flags/args." "If `--no-input`
is passed, don't prompt or do anything interactive... If the command requires
input, fail and tell the user how to pass the information as a flag."
"Confirm before doing anything dangerous. A common convention is to prompt
for the user to type y or yes if running interactively, or requiring them to
pass `-f` or `--force` otherwise."

Robustness: "unexpected input should be handled gracefully, operations should
be idempotent where possible". "Responsive is more important than fast. Print
something to the user in <100ms." "Make it recoverable." "Make it crash-only.
This is the next step up from idempotence. If you can avoid needing to do
any cleanup after operations, or you can defer that cleanup to the next run,
your program can exit immediately on failure or interruption." "Check early
and bail out before anything bad happens, and make the errors
understandable."

Future-proofing: "Subcommands, arguments, flags, configuration files,
environment variables: these are all interfaces, and you're committing to
keeping them working." "Keep changes additive where you can." "Warn before
you make a non-additive change." "Changing output for humans is usually OK...
Encourage your users to use `--plain` or `--json` in scripts to keep output
stable." "Don't have a catch-all subcommand." "Don't allow arbitrary
abbreviations of subcommands."

Help: "Display concise help text by default" when run with no arguments;
"Show full help when `-h` and `--help` are passed"; "you should be able to add
`-h` to the end of anything and it should show help."

Environment: "Check general-purpose environment variables for configuration
values when possible: `NO_COLOR`, to disable color (see Output) or
`FORCE_COLOR` to enable it and ignore the detection logic; `DEBUG`, to enable
more verbose output".

## 3. git's plumbing conventions

### 3.1 `--porcelain` and `-z` (git-status)

<https://git-scm.com/docs/git-status>: "`--porcelain[=<version>]` Give the
output in an easy-to-parse format for scripts. This is similar to the short
output, but will remain stable across Git versions and regardless of user
configuration. See below for details. The `<version>` parameter is used to
specify the format version. This is optional and defaults to the original
version v1 format." "`-z` Terminate entries with NUL, instead of LF. This
implies the `--porcelain=v1` output format if no other format is given."
"Version 1 porcelain format is similar to the short format, but is guaranteed
not to change in a backwards-incompatible way between Git versions or based
on user configuration. This makes it ideal for parsing by scripts." Its two
exceptions: "The user's `color.status` configuration is not respected; color
will always be off" and "The user's `status.relativePaths` configuration is
not respected; paths shown will always be relative to the repository root."
The `-z` variant: "a NUL (ASCII 0) follows each filename, replacing space as
a field separator and the terminating newline... filenames containing special
characters are not specially formatted; no quoting or backslash-escaping is
performed." git-ls-files (<https://git-scm.com/docs/git-ls-files>): "`-z` \0
line termination on output and do not quote filenames"; "Without the `-z`
option, pathnames with 'unusual' characters are quoted as explained for the
configuration variable `core.quotePath`."

The original rationale, commit 6f15787 "status: add --porcelain output
format" (Jeff King, 2009-09-05,
<https://github.com/git/git/commit/6f15787181a163e158c6fee1d79085b97692ac2f>):
"Scripts which want to parse the information and need a stable, easy-to-parse
interface... as time goes on, users of (1) may want additional format tweaks,
or for 'git status' to change its behavior based on configuration variables.
Those wishes will be at odds with (2), which wants to stability for scripts.
This patch introduces a separate --porcelain option early to avoid problems
later on... we will have the freedom to customize --short for human
consumption while keeping --porcelain stable." Commit 4a7cc2f: "The porcelain
format is identical to the shortstatus format, except that it should not
respect any user configuration, including color."

### 3.2 Why format versions exist (`--porcelain=v2`)

git-status: "Version 2 format adds more detailed information about the state
of the worktree and changed items. Version 2 also defines an extensible set
of easy to parse optional headers. Header lines start with `#` and are added
in response to specific command line arguments. Parsers should ignore headers
they don't recognize." Commit 1ecdecc "status: collect per-file data for
--porcelain=v2" (Jeff Hostetler, 2016-08-11,
<https://github.com/git/git/commit/1ecdecce621009a4d039d061d514056501d0ed8f>):
"The output of `git status --porcelain` leaves out many details about the
current status that clients might like to have. This can force them to be
less efficient as they may need to launch secondary commands (and try to
match the logic within git) to accumulate this extra information." Merge
commit 00d2793 (2016-09-09): "Enhance 'git status --porcelain' output by
collecting more data on the state of the index and the working tree files".
So v1 was frozen by promise, v2 was added rather than v1 changed, and v1
stayed the default.

The general rule, git(1) <https://git-scm.com/docs/git#_low_level_commands_plumbing>:
"The interface (input, output, set of options and the semantics) to these
low-level commands are meant to be a lot more stable than Porcelain level
commands, because these commands are primarily for scripted use. The
interface to Porcelain commands on the other hand are subject to change in
order to improve the end user experience."

### 3.3 `git diff --exit-code`

<https://git-scm.com/docs/git-diff>: "`--exit-code` Make the program exit with
codes similar to diff(1). That is, it exits with 1 if there were differences
and 0 means no differences." Without the flag `git diff` exits 0 in both
cases; the diff(1) convention is opt-in.

## 4. cargo

### 4.1 `--message-format` and exit status

<https://doc.rust-lang.org/cargo/commands/cargo-build.html>: "`--message-format`
fmt: The output format for diagnostic messages. Can be specified multiple
times and consists of comma-separated values. Valid values: `human`
(default)... `short`... `json`: Emit JSON messages to stdout... `json-
diagnostic-short`... `json-diagnostic-rendered-ansi`... `json-render-
diagnostics`: Instruct Cargo to not include rustc diagnostics in JSON
messages printed, but instead Cargo itself should render the JSON diagnostics
coming from rustc." "`--color` when: `auto` (default): Automatically detect if
color support is available on the terminal. `always`... `never`." "EXIT
STATUS: `0`: Cargo succeeded. `101`: Cargo failed to complete."

<https://doc.rust-lang.org/cargo/reference/external-tools.html>: "The output
goes to stdout in the JSON object per line format. The `reason` field
distinguishes different kinds of messages." "Note: `--message-format=json`
only controls Cargo and Rustc's output. This cannot control the output of
other tools... A possible workaround in these situations is to only interpret
a line as JSON if it starts with `{`." The `build-finished` message carries
`"success": true|false` and "can be helpful for tools to know when to stop
reading JSON messages... This message lets a tool know that Cargo will not
produce additional JSON messages, but there may be additional output that may
be generated afterwards".

Errors in JSON mode: rustc diagnostics arrive as `compiler-message` objects
on stdout; cargo's own errors do not. Source `src/cargo/lib.rs`
(<https://github.com/rust-lang/cargo/blob/master/src/cargo/lib.rs>)
`exit_with_error`: a `clap::Error` is printed by clap and exits `1` if
`use_stderr()` else `0` (cargo does not use clap's 2); otherwise
`display_error` ("Displays an error, and all its causes, to stderr") and
`std::process::exit(exit_code)`. `src/cargo/util/errors.rs`: `impl
From<anyhow::Error> for CliError { ... CliError::new(err, 101) }`, and
`CliError.error` "can be `None` in rare cases to exit with a code without
displaying a message".

### 4.2 Versioning and stability (`cargo metadata`)

external-tools: "The format is stable and versioned. When calling `cargo
metadata`, you should pass `--format-version` flag explicitly to avoid
forward incompatibility hazard." <https://doc.rust-lang.org/cargo/commands/cargo-metadata.html>:
"The output format is subject to change in future versions of Cargo. It is
recommended to include the `--format-version` flag to future-proof your code
and ensure the output is in the format you are expecting." "`--format-version`
version: Specify the version of the output format to use. Currently `1` is
the only possible value." Compatibility: "Within the same output format
version, the compatibility is maintained, except some scenarios. The
following is a non-exhaustive list of changes that are not considered as
incompatible: Adding new fields — New fields will be added when needed...
Adding new values for enum-like fields... Changing opaque representations".
The `--message-format json` page states no equivalent policy (checked; the
only "stable" language on the external-tools page is the metadata sentence
above).

### 4.3 rustc's JSON contract (the shape cargo forwards)

<https://doc.rust-lang.org/rustc/json.html>: "JSON messages are emitted one
per line to stderr." "Each type of message has a `$message_type` field which
can be used to distinguish the different formats. When parsing, care should
be taken to be forwards-compatible with future changes to the format.
Optional values may be `null`. New fields may be added. Enumerated fields
like 'level' or 'suggestion_applicability' may add new values." Diagnostic
fields: `$message_type` (`"diagnostic"`), `message`, `code`, `level`, `spans`,
`children`, `rendered`; `level` is one of `error`, `warning`, `note`, `help`,
`failure-note`, `error: internal compiler error`.

## 5. kubectl and docker

kubectl (<https://kubernetes.io/docs/reference/kubectl/>): syntax `kubectl
[command] [TYPE] [NAME] -o <output_format>`; formats `custom-columns`,
`custom-columns-file`, `json` ("Output a JSON formatted API object"),
`jsonpath`, `jsonpath-file`, `name` ("Print only the resource name and nothing
else"), `wide`, `yaml`, `go-template`, `go-template-file`. `kubectl get`
(<https://kubernetes.io/docs/reference/kubectl/generated/kubectl_get/>):
"`-o, --output` string: Output format. One of: (json, yaml, kyaml, name,
go-template, go-template-file, template, templatefile, jsonpath,
jsonpath-as-json, jsonpath-file, custom-columns, custom-columns-file,
wide)". Lists come back as one document with an `items` array; the
quick-reference (<https://kubernetes.io/docs/reference/kubectl/quick-reference/>)
consumes it as `kubectl get pods -o json | jq '.items[].spec...'`. Scripting
conventions (<https://kubernetes.io/docs/reference/kubectl/conventions/>):
"For a stable output in a script: Request one of the machine-oriented output
forms, such as `-o name`, `-o json`, `-o yaml`, `-o go-template`, or `-o
jsonpath`. Fully-qualify the version... Don't rely on context, preferences,
or other implicit states."

docker (<https://docs.docker.com/reference/cli/docker/container/ls/>):
`--format` accepts "'table': Print output in table format with column headers
(default); 'table TEMPLATE': Print output in table format using the given Go
template; 'json': Print in JSON format; 'TEMPLATE': Print output using the
given Go template", with the example `docker ps --format json` printing one
object per container. Formatting guide (<https://docs.docker.com/engine/cli/formatting/>):
"Docker supports Go templates which you can use to manipulate the output
format of certain commands and log drivers", functions `join`, `json`,
`lower`, `split`, `title`, `truncate`, `upper`, `println`, `pad`, `table`; tip:
`docker container ls --format='{{json .}}'`. Source
(`cli/command/formatter/formatter.go`,
<https://github.com/docker/cli/blob/master/cli/command/formatter/formatter.go>):
`--format json` maps to the template `{{json .}}` and `contextFormat` writes
`"\n"` after each element, so list commands emit JSON Lines, one object per
row, while `docker inspect` emits a JSON array (unverified beyond the docs
examples).

## 6. jq-friendliness and streaming

JSON Lines (<https://jsonlines.org/>, page generated 2026-09-01): "This page
describes the JSON Lines text format, also called newline-delimited JSON. JSON
Lines is a convenient format for storing structured data that may be
processed one record at a time. It works well with unix-style text processing
tools and shell pipelines. It's a great format for log files. It's also a
flexible format for passing messages between cooperating processes." Three
requirements: "1. UTF-8 Encoding... a byte order mark (U+FEFF) must NOT be
included. 2. Each Line is a Valid JSON Value: The most common values will be
objects or arrays, but any JSON value is permitted. e.g. `null` is a valid
value but a blank line is not. 3. Line Terminator is `'\n'`: This means
`'\r\n'` is also supported". Conventions: `.jsonl`, "MIME type may be
`application/jsonl`, but this is not yet standardized".

Who streams and why: cargo (§4.1, long build, `build-finished` sentinel);
rustc (§4.3, one per line to stderr); Codex `exec --json` ("stdout becomes a
JSON Lines (JSONL) stream so you can capture every event Codex emits while
it's running", §10.5); Claude Code `--output-format stream-json`
("newline-delimited JSON for real-time streaming", §10.2); docker list
commands (§5). Who emits one document: gh (§1.1, arrays), kubectl (`items`,
§5), Claude Code `--output-format json` (§10.2). Consumers handle both:
nushell `from json --objects`: "Treat each line as a separate value"
(<https://www.nushell.sh/commands/docs/from_json.html>); jq reads a stream of
values natively (unverified here; not fetched).

## 7. Exit code conventions

### 7.1 `sysexits.h`

FreeBSD 15.1 `sysexits(3)`
(<https://man.freebsd.org/cgi/man.cgi?query=sysexits&sektion=3>): "Some
commands attempt to describe the nature of a failure condition by using these
pre-defined exit codes. This interface has been deprecated and is retained
only for compatibility. Its use is discouraged." "Error numbers begin at
EX__BASE to reduce the possibility of clashing with other exit statuses that
random programs may already return." Codes: EX_OK 0; EX_USAGE 64 "The command
was used incorrectly, e.g., with the wrong number of arguments, a bad flag, a
bad syntax in a parameter, or whatever"; EX_DATAERR 65 "The input data was
incorrect in some way"; EX_NOINPUT 66 "An input file (not a system file) did
not exist or was not readable"; EX_NOUSER 67; EX_NOHOST 68; EX_UNAVAILABLE 69;
EX_SOFTWARE 70 "An internal software error has been detected"; EX_OSERR 71;
EX_OSFILE 72; EX_CANTCREAT 73 "A (user specified) output file cannot be
created"; EX_IOERR 74; EX_TEMPFAIL 75; EX_PROTOCOL 76; EX_NOPERM 77; EX_CONFIG
78. The macOS page on this Mac (`man 3 sysexits`) still calls them
"preferable exit codes for programs" with the same table and no deprecation.

### 7.2 POSIX and GNU practice

POSIX.1-2017 Utility Syntax Guidelines
(<https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap12.html>):
Guideline 10: "The first `--` argument that is not an option-argument should
be accepted as a delimiter indicating the end of options. Any following
arguments should be treated as operands, even if they begin with the '-'
character." Guideline 13: "For utilities that use operands to represent files
to be opened for either reading or writing, the '-' operand should be used to
mean only standard input (or standard output when it is clear from context
that an output file is being specified) or a file named `-`." Guideline 11:
"The order of different options relative to one another should not matter".

Bash manual, Exit Status
(<https://www.gnu.org/software/bash/manual/html_node/Exit-Status.html>):
"while an exit status of zero indicates success, a non-zero exit status
indicates failure." "All builtins return an exit status of 2 to indicate
incorrect usage, generally invalid options or missing arguments." "When a
command terminates on a fatal signal whose number is N, Bash uses the value
128+N as the exit status. If a command is not found... 127. If a command is
found but is not executable... 126."

GNU grep (<https://www.gnu.org/software/grep/manual/html_node/Exit-Status.html>):
"Normally the exit status is 0 if a line is selected, 1 if no lines were
selected, and 2 if an error occurred."

### 7.3 coreutils

<https://www.gnu.org/software/coreutils/manual/html_node/Exit-status.html>
(as returned by the fetcher; page could not be re-fetched raw due to rate
limiting, so treat the wording as close paraphrase): for the vast majority of
commands an exit status of zero indicates success, "Failure is indicated by a
nonzero value – typically '1'", with listed exceptions (`chroot`, `env`,
`expr`, `ls`, `nice`, `nohup`, `numfmt`, `printenv`, `runcon`, `sort`,
`stdbuf`, `test`, `timeout`, `tty`).

### 7.4 How gh, cargo, clap and jj choose codes

gh: 0 ok, 1 any failure, 2 cancelled, 4 auth, 8 pending (source), extensions
pass through (§1.2). cargo: 0 and 101; clap usage errors mapped to 1 (§4.1).
clap (<https://docs.rs/clap/latest/clap/error/struct.Error.html>): "`exit`:
Prints the error and exits. Depending on the error kind, this either prints
to stderr and exits with a status of 2 or prints to stdout and exits with a
status of 0." "`use_stderr`: Should the message be written to stdout or not?"
jj (`cli/src/command_error.rs`,
<https://github.com/jj-vcs/jj/blob/main/cli/src/command_error.rs>,
`handle_command_result`): User error 1, Config error 1, Cli error 2 (clap
errors rendered with clap's own stream/code rule: help/version to stdout with
0, else stderr with 2), BrokenPipe 3 ("A broken pipe is not an error, but a
signal to exit gracefully"), Internal 255. jj's user docs do not publish this
table (searched `docs/` for "exit code"; only merge-tool settings mention
exit codes).

Stability guarantees: gh documents its codes and warns commands may add more
(§1.2); cli/cli's contributor doc lists "exit behavior" among contracts that
may not be broken without explicit agreement (§1.8); cargo documents 0/101
per command; git promises stability for `--porcelain` output (§3.1) and
plumbing interfaces (§3.2); cargo metadata promises additive-only changes
within a format version (§4.2). No tool examined publishes a semver-style
promise for JSON fields beyond those statements.

## 8. Error objects

### 8.1 gh

No JSON error object exists. Errors are text on stderr with exit 1 regardless
of `--json`; usage errors append the usage string, or the full help when an
agent is detected (§1.4).

### 8.2 cargo / rustc

Cargo's own errors: text to stderr, exit 101 (§4.1). rustc diagnostics: JSON
objects with `message`, `code`, `level`, `spans`, `children`, `rendered`,
one per line on rustc's stderr, forwarded by cargo as `compiler-message`
objects on stdout (§4.1, §4.3).

### 8.3 npm

Config (<https://docs.npmjs.com/cli/v11/using-npm/config>): "`json` Default:
false. Type: Boolean. Whether or not to output JSON data, rather than the
normal output... Not supported by all npm commands." "`color` Default: true
unless the NO_COLOR environ is set to something other than '0'... If true,
then only prints color codes for tty file descriptors." Errors with `--json`
are not documented; source `lib/utils/output-error.js`
(<https://github.com/npm/cli/blob/latest/lib/utils/output-error.js>):
`jsonError` returns `{ code: error.code, summary, detail, ...error.json }`
when `--json` is set; `lib/utils/display.js` merges it into the final stdout
JSON under the key `error` (`ERROR_KEY = 'error'`), with this comment: "JSON
output has always been keyed at the root with an `error` key, so we cant
change that without it being a breaking change. At the same time some
commands output arbitrary keys at the top level of the output, such as
package names. So the output could already have the same key... XXX
(BREAKING_CHANGE): all json output should be keyed under well known keys, eg
`result` and `error`". npm audit
(<https://docs.npmjs.com/cli/v11/commands/npm-audit>): "will exit with a 0
exit code if no vulnerabilities were found", non-zero otherwise, tunable with
`--audit-level`.

### 8.4 jj and nushell

jj: text on stderr with `Error: `, `Config error: `, `Internal error: `
headings, `Caused by:` chains and `Hint:` lines (§7.4); no JSON mode.
nushell: no documented error-object policy found; skipped.

## 9. Rust ecosystem specifics

### 9.1 clap global flags

`Arg::global` (<https://docs.rs/clap/latest/clap/struct.Arg.html#method.global>):
"Specifies that an argument can be matched to all child Subcommands. NOTE:
Global arguments only propagate down, not up (to parent commands), however
their values once a user uses them will be propagated back up to parents. In
effect, this means one should define all global arguments at the top level,
however it doesn't matter where the user uses the global argument." In derive
form this is `#[arg(global = true)]` on the top-level struct field (derive
reference: raw attributes forward any `Arg` method,
<https://docs.rs/clap/latest/clap/_derive/index.html>). `Command::
propagate_version` propagates `--version` to subcommands.

### 9.2 clap exit codes and streams

§7.4: usage errors print to stderr and exit 2; `--help`/`--version` print to
stdout and exit 0. `Error::print`: "Prints formatted and colored error to
stdout or stderr according to its error kind". To emit JSON usage errors,
parse with `try_get_matches`/`try_parse`, inspect `kind()` and `use_stderr()`,
and render yourself (jj does exactly this in `handle_clap_error`, §7.4).

### 9.3 `--` and `-`

clap_lex `ParsedArg` (<https://docs.rs/clap_lex/latest/clap_lex/struct.ParsedArg.html>):
`is_escape`: "Does the argument look like an argument escape (`--`)";
`is_stdio`: "Does the argument look like a stdio argument (`-`)"; `is_long`,
`is_short`, `is_negative_number`. The parser
(`clap_builder/src/parser/parser.rs`) sets `TrailingVals=true` on
`is_escape()` unless the current arg allows hyphen values, and a bare `-` is
never treated as a flag, so it reaches the positional as the string `"-"`.
clap does not open stdin itself; the program checks for `"-"`.
`Arg::allow_hyphen_values`: "Allows values which start with a leading hyphen
(`-`)"; `Arg::last`: "only able to be accessed via the `--` syntax";
`Arg::trailing_var_arg`: "everything that follows should be captured by it,
as if the user had used a `--`".

### 9.4 anstream / anstyle / NO_COLOR

clap's `color` feature is `color = ["dep:anstream"]` and its `Colorizer::
print` wraps stdout/stderr in `anstream::AutoStream::new(..., color_when)`
(`clap_builder/Cargo.toml`, `clap_builder/src/output/fmt.rs`). clap
`ColorChoice` (<https://docs.rs/clap/latest/clap/enum.ColorChoice.html>):
"`Auto`: Enables colored output only when the output is going to a terminal
or TTY. NOTE: This is the default behavior of `clap`." anstream
(<https://docs.rs/anstream/latest/anstream/>, source
`crates/anstream/src/auto.rs` in rust-cli/anstyle): "Auto-adapting stdout /
stderr streams... AutoStream always accepts ANSI escape codes, adapting to
the user's terminal's capabilities." `choice()` for `ColorChoice::Auto`, in
order: `anstyle_query::no_color()` → `Never`; `clicolor_force()` → `Always`;
`CLICOLOR` present and `== "0"` → `Never`; `raw.is_terminal() &&
(term_supports_color() || CLICOLOR non-zero || is_ci())` → `Always`; else
`Never`. anstyle-query (<https://docs.rs/anstyle-query/latest/anstyle_query/>,
`crates/anstyle-query/src/lib.rs`): `no_color()` "Check NO_COLOR status. When
`true`, should prevent the addition of ANSI color. User-level configuration
files and per-instance command-line arguments should override NO_COLOR";
`clicolor_force()` non-empty `CLICOLOR_FORCE`; `term_supports_color()` false
if `TERM` unset (non-Windows) or `TERM=dumb`; `is_ci()` the `CI` variable;
`truecolor()` `COLORTERM` of `truecolor`/`24bit`. `colorchoice` provides the
"Global override of color control" (`ColorChoice::write_global`,
<https://docs.rs/colorchoice/latest/colorchoice/>). no-color.org
(<https://no-color.org/>): "Command-line software which adds ANSI color to
its output by default should check for a `NO_COLOR` environment variable
that, when present and not an empty string (regardless of its value),
prevents the addition of ANSI color... User-level configuration files and
per-instance command-line arguments should override the `NO_COLOR`
environment variable." Terminal detection in std: `std::io::IsTerminal`
(stable 1.70, <https://doc.rust-lang.org/std/io/trait.IsTerminal.html>):
"Returns `true` if the descriptor/handle refers to a terminal/tty."

### 9.5 clap_complete

<https://docs.rs/clap_complete/latest/clap_complete/>: static generation via
`generate`/`generate_to` for `Shell::{Bash, Zsh, Fish, PowerShell, Elvish}`,
typically behind a `completions <shell>` subcommand or at build time; the
dynamic `CompleteEnv` mode (`COMPLETE=$SHELL <bin>`) requires the
`unstable-dynamic` feature and is subject to change.

## 10. Agent-facing CLIs

### 10.1 Anthropic: Skill authoring best practices

<https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices>
(fetched as `.md`): "When writing scripts for Skills, handle error conditions
rather than deferring to Claude." The good example catches
`FileNotFoundError`/`PermissionError` and prints what it did; the bad example
is "Just fail and let Claude figure it out". "Configuration parameters should
also be justified and documented to avoid 'voodoo constants'". Benefits of
utility scripts: "More reliable than generated code; Save tokens (no need to
include code in context); Save time (no code generation required); Ensure
consistency across uses". "The instruction file (forms.md) references the
script, and Claude can execute it without loading its contents into
context." "Make clear in your instructions whether Claude should: Execute the
script (most common)... Read it as reference". Runtime environment: "Scripts
executed efficiently: Utility scripts can be executed through bash without
loading their full contents into context. Only the script's output consumes
tokens". The worked example documents each script by its exact invocation,
e.g. `python scripts/analyze_form.py input.pdf > fields.json` with an "Output
format" JSON block, and `validate_boxes.py` "# Returns: 'OK' or lists
conflicts". Nothing on exit codes, JSON envelopes or `--help` quality: the
page does not address them.

### 10.2 Anthropic: Claude Code's own headless contract

<https://code.claude.com/docs/en/headless.md>: "Add the `-p` (or `--print`)
flag to any `claude` command to run it non-interactively." "Claude Code exits
with code 0 on success and a non-zero code when the run fails, so your
scripts can branch on the exit status. If you pass an invalid flag, Claude
Code reports the error to stderr before the run starts. When a failure
happens inside the run, such as missing authentication, Claude Code prints
the failure as the result on stdout." `--output-format`: "`text` (default):
plain text output; `json`: structured JSON with result, session ID, and
metadata; `stream-json`: newline-delimited JSON for real-time streaming". The
JSON result has `result`, `session_id`, `total_cost_usd`, and
`structured_output` when `--json-schema` is given; stream events carry
`type`, `subtype`, `session_id`. "Piped stdin is capped at 10MB. If you exceed
the cap, Claude Code exits with a clear error and a non-zero status." "If you
stop a `claude -p` run with SIGTERM... Claude Code exits with code 143... On
SIGTERM, Claude Code terminates the process tree of any Bash command that is
still running." Environment
(<https://code.claude.com/docs/en/env-vars.md>): "`CLAUDECODE`: Set to `1` in
subprocesses Claude Code spawns (Bash and PowerShell tools, tmux sessions,
hook commands, status line commands, stdio MCP server subprocesses)... Use to
detect when a script is running inside a subprocess spawned by Claude Code";
`CLAUDE_CODE_CHILD_SESSION` for the stricter check.

### 10.3 Anthropic: how Bash output and exit codes reach the model

<https://code.claude.com/docs/en/tools-reference.md>, Bash tool: valid output
"Inline up to roughly 30,000 characters by default; past that, the path of a
file saved to the session directory and truncated past 64 MiB, plus a short
preview from the start"; failure output "Inline up to roughly 10,000
characters; past that, a head-and-tail excerpt". "A command that exits 1
counts as a valid result for the Bash tool only when Claude Code recognizes
exit code 1 as a benign outcome for that command: `grep`, `rg`, `egrep`,
`fgrep`, `find`, `diff`, `test`, and `[`, plus `git diff` and `git grep`.
Every other command that exits 1 counts as a failure, even when exit 1 is a
benign informational outcome: no matches for `pgrep` and `jq -e`, files that
differ for `cmp`." Timeouts: `BASH_DEFAULT_TIMEOUT_MS` "two minutes out of
the box", `BASH_MAX_TIMEOUT_MS` "ten minutes out of the box". Skills
(<https://code.claude.com/docs/en/skills.md>), injected `!` commands: "A
failed command aborts the entire skill invocation... With the default `bash`
shell, any non-zero exit code counts as a failure." "Exit codes of 2 or
higher fail even for those commands." "Injected commands never prompt for
permission." `allowed-tools: Bash(${CLAUDE_SKILL_DIR}/scripts/render.sh *)`
lets a skill "run a bundled script without a permission prompt". API Bash
tool (<https://platform.claude.com/docs/en/agents-and-tools/tool-use/bash-tool>):
the harness "runs the command in its bash session and returns the output in
a `tool_result` block" (delegated fetch; the stdout/stderr interleaving
detail is from its example code and is **unverified** beyond that).

### 10.4 Anthropic engineering: "Writing tools for agents" (2025-09-11)

<https://www.anthropic.com/engineering/writing-tools-for-agents> (first-party
engineering post, not reference docs): "tool implementations should take care
to return only high signal information back to agents. They should prioritize
contextual relevance over flexibility, and eschew low-level technical
identifiers (for example: `uuid`, `256px_image_url`, `mime_type`). Fields like
`name`, `image_url`, and `file_type` are much more likely to directly inform
agents' downstream actions and responses." "We suggest implementing some
combination of pagination, range selection, filtering, and/or truncation with
sensible default parameter values for any tool responses that could use up
lots of context. For Claude Code, we restrict tool responses to 25,000 tokens
by default." "if a tool call raises an error (for example, during input
validation), you can prompt-engineer your error responses to clearly
communicate specific and actionable improvements, rather than opaque error
codes or tracebacks." "You can enable both by exposing a simple
`response_format` enum parameter in your tool, allowing your agent to control
whether tools return 'concise' or 'detailed' responses." "Tools can
consolidate functionality, handling potentially multiple discrete operations
(or API calls) under the hood."

### 10.5 OpenAI: Codex CLI (its own conventions; no guidance for tools it drives)

Non-interactive mode
(<https://developers.openai.com/codex/noninteractive>, redirects to
<https://learn.chatgpt.com/docs/non-interactive-mode>): "While `codex exec`
runs, Codex streams progress to stderr and prints only the final agent message
to stdout. This makes it straightforward to redirect or pipe the final
result". "When you enable `--json`, stdout becomes a JSON Lines (JSONL) stream
so you can capture every event Codex emits while it's running. Event types
include `thread.started`, `turn.started`, `turn.completed`, `turn.failed`,
`item.*`, and `error`." `-o/--output-last-message` "writes the final message
to the file and still prints it to stdout"; `--output-schema` "to request a
final response that conforms to a JSON Schema". CLI reference
(<https://developers.openai.com/codex/cli/reference>, redirects to
<https://learn.chatgpt.com/docs/developer-commands?surface=cli>): "`--json`,
`--experimental-json`: Print newline-delimited JSON events instead of
formatted text"; "`--color` always | never | auto: Control ANSI color in
stdout". No general exit-code table is published. No OpenAI first-party
document giving guidance on how third-party CLIs should behave for agents was
found (**unverified**: none located; treat as absent).

### 10.6 Python as the second caller

<https://docs.python.org/3/library/subprocess.html>: "If `check` is true, and
the process exits with a non-zero exit code, a `CalledProcessError` exception
will be raised. Attributes of that exception hold the arguments, the exit
code, and stdout and stderr if they were captured."

---

## Sources

- GitHub CLI manual: <https://cli.github.com/manual/gh_help_formatting>,
  <https://cli.github.com/manual/gh_help_exit-codes>,
  <https://cli.github.com/manual/gh_help_environment> (also run locally, gh 2.100.0)
- cli/cli source at `7b2de63c`: `internal/ghcmd/cmd.go`, `pkg/cmdutil/json_flags.go`,
  `pkg/cmdutil/errors.go`, `pkg/iostreams/iostreams.go`, `internal/tableprinter/table_printer.go`,
  `pkg/cmd/pr/list/list.go`, `internal/update/update.go`, `internal/agents/detect.go`,
  `docs/command-development.md`, `docs/primer/foundations/README.md`
  (<https://github.com/cli/cli/tree/7b2de63c49c0d6717930415d2100a56103360509>)
- cli/cli PR #3414: <https://github.com/cli/cli/pull/3414>
- cli/go-gh `pkg/term/env.go`: <https://github.com/cli/go-gh/blob/trunk/pkg/term/env.go>
- clig.dev: <https://clig.dev/>
- git: <https://git-scm.com/docs/git-status>, <https://git-scm.com/docs/git-ls-files>,
  <https://git-scm.com/docs/git-diff>, <https://git-scm.com/docs/git#_low_level_commands_plumbing>;
  commits 6f15787, 4a7cc2f, 1ecdecc, 00d2793 in <https://github.com/git/git>
- cargo: <https://doc.rust-lang.org/cargo/commands/cargo-build.html>,
  <https://doc.rust-lang.org/cargo/reference/external-tools.html>,
  <https://doc.rust-lang.org/cargo/commands/cargo-metadata.html>;
  source `src/cargo/lib.rs`, `src/cargo/util/errors.rs` (<https://github.com/rust-lang/cargo>)
- rustc: <https://doc.rust-lang.org/rustc/json.html>
- kubectl: <https://kubernetes.io/docs/reference/kubectl/>,
  <https://kubernetes.io/docs/reference/kubectl/generated/kubectl_get/>,
  <https://kubernetes.io/docs/reference/kubectl/conventions/>,
  <https://kubernetes.io/docs/reference/kubectl/quick-reference/>
- docker: <https://docs.docker.com/reference/cli/docker/container/ls/>,
  <https://docs.docker.com/engine/cli/formatting/>,
  <https://github.com/docker/cli/blob/master/cli/command/formatter/formatter.go>
- JSON Lines: <https://jsonlines.org/>; nushell: <https://www.nushell.sh/commands/docs/from_json.html>
- sysexits: <https://man.freebsd.org/cgi/man.cgi?query=sysexits&sektion=3> (FreeBSD 15.1); local `man 3 sysexits`
- POSIX: <https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap12.html>
- GNU: <https://www.gnu.org/software/bash/manual/html_node/Exit-Status.html>,
  <https://www.gnu.org/software/grep/manual/html_node/Exit-Status.html>,
  <https://www.gnu.org/software/coreutils/manual/html_node/Exit-status.html>
- clap: <https://docs.rs/clap/latest/clap/error/struct.Error.html>,
  <https://docs.rs/clap/latest/clap/struct.Arg.html>,
  <https://docs.rs/clap/latest/clap/enum.ColorChoice.html>,
  <https://docs.rs/clap/latest/clap/_derive/index.html>,
  <https://docs.rs/clap_lex/latest/clap_lex/struct.ParsedArg.html>,
  <https://docs.rs/clap_complete/latest/clap_complete/>;
  source `clap_builder/Cargo.toml`, `clap_builder/src/output/fmt.rs`,
  `clap_builder/src/parser/parser.rs` (<https://github.com/clap-rs/clap>)
- anstream/anstyle: <https://docs.rs/anstream/latest/anstream/>,
  <https://docs.rs/anstyle/latest/anstyle/>, <https://docs.rs/anstyle-query/latest/anstyle_query/>,
  <https://docs.rs/colorchoice/latest/colorchoice/>;
  source `crates/anstream/src/auto.rs`, `crates/anstyle-query/src/lib.rs`
  (<https://github.com/rust-cli/anstyle>)
- no-color.org: <https://no-color.org/>; std: <https://doc.rust-lang.org/std/io/trait.IsTerminal.html>
- npm: <https://docs.npmjs.com/cli/v11/using-npm/config>,
  <https://docs.npmjs.com/cli/v11/commands/npm-audit>;
  source `lib/utils/output-error.js`, `lib/utils/display.js`, `lib/npm.js` (<https://github.com/npm/cli>)
- jj: `cli/src/command_error.rs` (<https://github.com/jj-vcs/jj>)
- Anthropic: <https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices>,
  <https://code.claude.com/docs/en/headless.md>, <https://code.claude.com/docs/en/tools-reference.md>,
  <https://code.claude.com/docs/en/skills.md>, <https://code.claude.com/docs/en/env-vars.md>,
  <https://platform.claude.com/docs/en/agents-and-tools/tool-use/bash-tool>,
  <https://www.anthropic.com/engineering/writing-tools-for-agents>
- OpenAI Codex: <https://developers.openai.com/codex/noninteractive>,
  <https://developers.openai.com/codex/cli/reference>
- Python: <https://docs.python.org/3/library/subprocess.html>
