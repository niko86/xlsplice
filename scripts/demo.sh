#!/bin/sh
# A hydration, start to finish, with every command shown.
#
#   scripts/demo.sh                 over a template, or the feature fixture
#   scripts/demo.sh PACKAGE         over the package you name
#
# It fills two input cells by name, dates the run, stamps who did it, asks
# Excel to recalculate on the way in, and then shows what moved and what did
# not. Nothing is written beside the package it reads: the output goes to a
# temporary directory, whose path is printed at the end for opening in Excel.
#
# Run it with a terminal on stdout, where the tool draws aligned tables; piped,
# the same commands answer tab-separated, which is the other half of the
# contract and is what `cut` wants.

set -eu

BIN=${XLSPLICE_BIN-}
if [ -z "$BIN" ]; then
	cargo build --quiet
	BIN=target/debug/xlsplice
fi

# The package to hydrate: the one named, else the corpus template where there
# is a corpus, else the fixture committed here, which runs anywhere.
TEMPLATE="PSD ISO Input [v000012].xlsm"
PACKAGE=${1-}
if [ -z "$PACKAGE" ]; then
	if [ -n "${XLSPLICE_CORPUS-}" ] && [ -f "${XLSPLICE_CORPUS}/${TEMPLATE}" ]; then
		PACKAGE="${XLSPLICE_CORPUS}/${TEMPLATE}"
	else
		PACKAGE=tests/fixtures/feature.xlsx
	fi
fi
[ -f "$PACKAGE" ] || {
	echo "no package at $PACKAGE" >&2
	exit 1
}

# What a hydration fills, per package. The corpus names carry `?TC`, which is
# why every target here comes after `--`: a shell is not the only thing that
# would rather read a `?` as something else.
case "$PACKAGE" in
*feature.xlsx)
	CONTAINER="MergedInput"     # a name on a merged range
	SAMPLE="Inputs!A2"          # a plain number cell
	RUN_DATE="Inputs!B3"        # a cell the sheet does not hold
	FORMULA="Inputs!D1"         # SUM(A1:A5), which a write must not touch
	;;
*)
	# A corpus package is vendor material and so are its defined names, which
	# is why none are written down here. Name the four in the environment.
	CONTAINER="${XLSPLICE_DEMO_CONTAINER:?set it to a name on a merged range}"
	SAMPLE="${XLSPLICE_DEMO_SAMPLE:?set it to a plain number cell}"
	RUN_DATE="${XLSPLICE_DEMO_RUN_DATE:?set it to a cell the sheet does not hold}"
	FORMULA="${XLSPLICE_DEMO_FORMULA:?set it to a cell holding a formula}"
	;;
esac

WORK=$(mktemp -d)
OUT="$WORK/hydrated.${PACKAGE##*.}"
trap 'echo; echo "The hydrated package is at $OUT"; echo "Remove it with: rm -rf $WORK"' EXIT

step() {
	echo
	echo "── $1"
	echo
}

# Print the command, then run it. What is printed says PACKAGE and OUT where
# the paths are, because an absolute path to a template three directories deep
# is not what a reader of a demo is here to look at.
show() {
	printf '$ xlsplice'
	for word in "$@"; do
		case "$word" in
		"$PACKAGE") printf ' PACKAGE' ;;
		"$OUT") printf ' OUT' ;;
		"$WORK"/*) printf ' %s' "${word##*/}" ;;
		*) printf ' %s' "$word" ;;
		esac
	done
	echo
	"$BIN" "$@" || echo "[exit $?]"
	echo
}

echo "Hydrating $PACKAGE"
echo "Binary:   $BIN"
echo "PACKAGE is that file; OUT is the one written, in $WORK."

step "What the tool is"
show version

step "What the package holds. A template keeps its workings on sheets nobody
   is meant to see, and they are listed like any other."
show sheets "$PACKAGE"

step "The input cells a template exposes are its defined names. There are
   rather a lot of them, so here are the ones this demo fills."
echo "\$ xlsplice names PACKAGE | wc -l"
"$BIN" names "$PACKAGE" | wc -l
echo
show get "$PACKAGE" -- "$CONTAINER" "$SAMPLE" "$RUN_DATE" "$FORMULA"

step "Two of those are empty and one holds a formula over them. Writing to a
   formula cell is refused unless you say so, so a mis-addressed write cannot
   quietly destroy a template. The exit code says which kind of refusal it is."
show set --type number "$PACKAGE" -- "$FORMULA" 1

step "Nothing was written. Every non-zero exit means that, so a caller can
   retry without inspecting the file — and here is the package held against
   itself to say so."
echo "\$ xlsplice diff PACKAGE PACKAGE --json | jq .identical"
"$BIN" diff "$PACKAGE" "$PACKAGE" --json | sed 's/.*"identical":\([a-z]*\).*/\1/'
echo

step "The hydration itself: two masses, the date it was run, who ran it, and
   the flag that tells Excel to work the formulas out again on the way in.
   One batch, one process, one rewrite — and \`--out\` leaves the template
   alone, so copy-then-fill is one pass rather than two."
cat >"$WORK/hydration.json" <<JSON
[
  {"op": "set", "target": "$CONTAINER", "type": "number", "value": "152.4"},
  {"op": "set", "target": "$SAMPLE", "type": "number", "value": "1187.6"},
  {"op": "set", "target": "$RUN_DATE", "type": "date", "value": "2026-09-13"},
  {"op": "props.set", "name": "Run.By", "type": "text", "value": "xlsplice demo"},
  {"op": "props.set", "name": "Run.At", "type": "date", "value": "2026-09-13"},
  {"op": "calc", "full_calc_on_load": true}
]
JSON
echo "\$ cat hydration.json"
cat "$WORK/hydration.json"
echo
show apply "$PACKAGE" "$WORK/hydration.json" --out "$OUT" --json

step "What landed, read back out of the package that was written."
show get "$OUT" -- "$CONTAINER" "$SAMPLE" "$RUN_DATE" "$FORMULA"

step "The formula is the formula the template author wrote. Its cached value
   is the one Excel last worked out, because nothing here computes anything —
   which is what the recalculate flag is for."
show props get "$OUT"
show calc "$OUT"

step "And what it cost: the parts that differ, against everything else, byte
   for byte. Charts, drawings, media and a VBA project are in the second list."
show diff "$PACKAGE" "$OUT"

step "Run the same hydration again and it changes nothing, because the values
   are already there. A re-run is safe."
show apply "$OUT" "$WORK/hydration.json" --json
