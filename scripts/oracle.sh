#!/bin/sh
# Run the oracle suite: a real Excel, asked whether what xlsplice wrote opens
# clean.
#
#   scripts/oracle.sh                 every case, over the fixtures
#   scripts/oracle.sh CORPUS_DIR      and over the packages under CORPUS_DIR
#   scripts/oracle.sh -- NAME         one case, by name
#
# Everything here is one thing: exporting what the suite reads, so that a
# variable typed on a line of its own — which sets a shell variable the child
# process never sees — cannot quietly turn `require` off and make a suite of
# skips look like a suite of verdicts.
#
# It takes the screen for as long as it runs: Excel opens and quits once per
# package, and the oracle refuses to start if Excel has a window that is not
# its own. See ADR-0006.

set -eu

# A skip is a failure here. This is the machine the oracle is meant to run on,
# so an oracle that cannot answer means the harness is broken rather than the
# machine being the wrong one.
XLSPLICE_ORACLE=require
export XLSPLICE_ORACLE

case "${1-}" in
-- | "") ;;
*)
	XLSPLICE_CORPUS=$1
	export XLSPLICE_CORPUS
	shift
	;;
esac
[ "${1-}" = "--" ] && shift

# The verdict is read off the screen, so the terminal this runs in needs
# Accessibility permission. Without it System Events answers nothing about any
# application and every package looks like a timeout, so it is asked about
# first, plainly, rather than left to be discovered case by case.
# Asked about one named process, which is the shape the suite itself uses.
# Asking about every visible process is a stricter question than the suite
# ever asks — one process in the enumeration refusing fails the whole thing —
# and on 2026-09-14 that is exactly what it did, on a machine where the suite
# would have run.
if ! /usr/bin/osascript -e 'tell application "System Events"
	if not (exists process "Finder") then return "no Finder to ask about"
	return (count of windows of process "Finder") as text
end tell' >/dev/null 2>&1; then
	echo "This terminal cannot read the screen, so the oracle has no verdict to give." >&2
	echo >&2
	echo "What is refused is narrow: Apple Events work, and System Events will say" >&2
	echo "how many processes there are and refuse to say how many windows they have." >&2
	echo "That is the Accessibility permission, and it belongs to this terminal." >&2
	echo >&2
	echo "Grant it in System Settings, Privacy & Security, Accessibility. If it is" >&2
	echo "listed and enabled already the entry has gone stale, and the checkbox does" >&2
	echo "not clear a stale entry — it edits it. Remove it instead, quit the terminal," >&2
	echo "and start it again, which makes the next call ask:" >&2
	echo >&2
	echo "  tccutil reset Accessibility <this terminal's bundle id>" >&2
	echo >&2
	echo "Note that \`UI elements enabled\` answers true while the calls deny, so it is" >&2
	echo "no use as a check. The probe above is the one that tells the truth." >&2
	exit 1
fi

# `--nocapture` because the counts the suite prints — how many packages Excel
# actually saw — are the difference between a case that passed and a case that
# ran, and libtest hides them otherwise.
exec cargo test --test oracle -- --ignored --test-threads=1 --nocapture "$@"
