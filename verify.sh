#!/usr/bin/env bash
#
# Apiary — the authoritative verification gate.
#
# This is the one that counts. It runs on the pinned toolchain and runs Verus
# over the verified core. The gates run in the cloud during the build are a
# weaker pre-check on a different machine with no Verus at all; a green cloud
# build says nothing about the proofs in verus/.
#
# There is no service to start. The control database, the fleet of hive files
# and the Harvest runtime are all SQLite files, so this script makes a scratch
# directory, boots the app in it, drives it over HTTP with a cookie jar exactly
# as a browser would, and deletes it on the way out. Safe to run twice.
#
#   ./verify.sh          run everything
#   ./verify.sh --quick  skip the boot and the end-to-end flow
#
set -euo pipefail

cd "$(dirname "$0")"
ROOT=$(pwd)

GATES=()
STATUSES=()
FAILED=0
QUICK=0
APP_PID=""
SCRATCH=""
# A free port, so a `cargo run` you already have going on 3000 is no obstacle.
port_in_use() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
PORT=3100
while port_in_use "$PORT"; do PORT=$((PORT + 1)); done
BASE="http://127.0.0.1:$PORT"

if [ "${1:-}" = "--quick" ]; then QUICK=1; fi

if [ -t 1 ]; then
    BOLD=$(printf '\033[1m'); RED=$(printf '\033[31m')
    GREEN=$(printf '\033[32m'); YELLOW=$(printf '\033[33m'); OFF=$(printf '\033[0m')
else
    BOLD=""; RED=""; GREEN=""; YELLOW=""; OFF=""
fi

record() {
    GATES+=("$1"); STATUSES+=("$2")
    if [ "$2" = "FAIL" ]; then FAILED=1; fi
}
step() { printf '\n%s── %s %s\n' "$BOLD" "$1" "$OFF"; }
run_gate() {
    local name="$1"; shift
    step "$name"
    if "$@"; then record "$name" "PASS"; else record "$name" "FAIL"; fi
}
cleanup() {
    if [ -n "${APP_PID:-}" ]; then
        kill "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [ -n "${SCRATCH:-}" ] && [ -d "$SCRATCH" ]; then rm -rf "$SCRATCH"; fi
}
trap cleanup EXIT INT TERM

# ── Gate 0: the toolchain must match rust-toolchain.toml ────────────────────

step "toolchain"
EXPECTED=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' rust-toolchain.toml | sed -n 1p)
if ! command -v cargo >/dev/null 2>&1; then
    printf '%sFAIL%s cargo is not on PATH\n' "$RED" "$OFF"; exit 1
fi
ACTUAL=$(cargo --version | awk '{print $2}')
printf 'expected %s, found cargo %s\n' "$EXPECTED" "$ACTUAL"
if [ "$ACTUAL" != "$EXPECTED" ]; then
    printf '%sFAIL%s toolchain mismatch. Install it with: rustup toolchain install %s\n' "$RED" "$OFF" "$EXPECTED"
    printf '  Refusing to run: a green result on the wrong compiler is worse than no result.\n'
    exit 1
fi
record "toolchain" "PASS"

# ── Gates 1-3: format, lint, every test ─────────────────────────────────────
#
# The test storm spans 40 hives against a fleet of 3, small enough for a
# default macOS `ulimit -n` of 256.

run_gate "cargo fmt"    cargo fmt --all -- --check
run_gate "cargo clippy" cargo clippy --all-targets -- -D warnings
run_gate "cargo test"   cargo test --workspace

# ── Gate 4: boot, then the whole flow over HTTP ─────────────────────────────

flow() {
    local jar="$SCRATCH/cookies.txt" token body phase i
    token=$(curl -fsS -c "$jar" -b "$jar" "$BASE/" \
        | sed -n 's/.*name="csrf-token" content="\([^"]*\)".*/\1/p' | sed -n 1p)
    [ -n "$token" ] || { echo "no csrf token on the comb"; return 1; }
    post() { curl -fsS -c "$jar" -b "$jar" -H "HX-Request: true" -H "X-CSRF-Token: $token" -X POST "$@"; }
    get() { curl -fsS -c "$jar" -b "$jar" "$@"; }
    # `expect PATTERN cmd...`: run cmd, then look for PATTERN in all of its
    # output. Never `cmd | grep -q`: grep exits on the first match, curl dies
    # of SIGPIPE mid-write, and under pipefail the step fails at random.
    expect() {
        local pattern="$1" out; shift
        out=$("$@") || return 1
        grep -qi -- "$pattern" <<<"$out"
    }

    echo "found a hive";      expect '^hx-redirect: /hive' post -D - -d name=verify "$BASE/hives" || return 1
    echo "walk into it";      expect 'The <code>verify</code> hive' get "$BASE/hive" || return 1
    echo "write a note";      expect 'written by verify.sh' post -d 'body=written+by+verify.sh' "$BASE/hive/notes" || return 1
    echo "an unfounded hive"; [ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/enter/nobody-here")" = 404 ] || return 1
    echo "a hostile name";    [ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/enter/Not.A.Hive")" = 400 ] || return 1
    echo "export the file";   get -o "$SCRATCH/export.db" "$BASE/hive/export" || return 1
    [ "$(head -c 15 "$SCRATCH/export.db")" = "SQLite format 3" ] || return 1

    echo "raise a storm (1,000 hives)"
    post "$BASE/storm" >/dev/null || return 1
    for i in $(seq 1 600); do
        body=$(get "$BASE/_map")
        if ! grep -q 'Storming' <<<"$body"; then break; fi
        sleep 1
    done
    grep -q 'Storming' <<<"$body" && { echo "the storm never finished"; return 1; }
    grep -q '<b>0</b><span>failed ops</span>' <<<"$body" || { echo "the storm had failed ops"; return 1; }
    printf '  peak open / cap: %s\n' "$(sed -n 's/.*<b>\([0-9]* \/ [0-9]*\)<\/b><span>peak open \/ cap.*/\1/p' <<<"$body")"

    echo "search every hive"; expect 'hives matched' get "$BASE/search?q=storm+note+3" || return 1

    echo "offboard (grace ${APIARY_GRACE_SECS}s)"
    get "$BASE/enter/verify" >/dev/null
    post "$BASE/hive/offboard" >/dev/null || return 1
    phase=""
    for i in $(seq 1 60); do
        phase=$(get "$BASE/_hive/offboarding" | sed -n 's/.*class="phase \([a-z]*\)".*/\1/p' | sed -n 1p)
        [ "$phase" = deleted ] || [ "$phase" = aborted ] && break
        sleep 1
    done
    [ "$phase" = deleted ] || { echo "offboarding ended in '$phase'"; return 1; }
    expect 'Restore from the backup' get "$BASE/hive" || return 1
    echo "restore it";        expect '^hx-redirect: /hive' post -D - "$BASE/hive/restore" || return 1
    expect 'written by verify.sh' get "$BASE/hive/notes" || { echo "the note did not come back"; return 1; }
    echo "the note came back from the backup"
}

if [ "$QUICK" -eq 1 ]; then
    step "boot + flow"; printf '%sskipped%s (--quick)\n' "$YELLOW" "$OFF"
    record "app boots" "SKIP"; record "end-to-end flow" "SKIP"
elif curl -fsS --max-time 2 "$BASE/health" >/dev/null 2>&1; then
    step "app boots"
    printf '%sFAIL%s something is already serving on port %s. Stop it first.\n' "$RED" "$OFF" "$PORT"
    record "app boots" "FAIL"; record "end-to-end flow" "SKIP"
else
    step "app boots"
    SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/apiary-verify.XXXXXX")
    cp autumn.toml autumn-dev.toml "$SCRATCH/"
    cargo build --bin apiary-app
    # A storm holds hundreds of hive files open at once (see docs/FINDINGS.md).
    ulimit -n 8192 2>/dev/null || ulimit -n 4096 2>/dev/null || true
    printf 'open-file limit: %s\n' "$(ulimit -n)"
    export AUTUMN_PROFILE=dev APIARY_GRACE_SECS=3 AUTUMN_SERVER__PORT="$PORT"
    : "${AUTUMN_SECURITY__SIGNING_SECRET:=verify-sh-development-secret-not-for-production-use}"
    export AUTUMN_SECURITY__SIGNING_SECRET
    (cd "$SCRATCH" && exec "$ROOT/target/debug/apiary-app") > "$SCRATCH/boot.log" 2>&1 &
    APP_PID=$!
    BOOTED=0
    for _ in $(seq 1 90); do
        kill -0 "$APP_PID" 2>/dev/null || break
        if curl -fsS "$BASE/health" >/dev/null 2>&1; then BOOTED=1; break; fi
        sleep 1
    done
    if [ "$BOOTED" -eq 1 ] && kill -0 "$APP_PID" 2>/dev/null; then
        record "app boots" "PASS"
        step "end-to-end flow"
        if flow; then record "end-to-end flow" "PASS"; else
            record "end-to-end flow" "FAIL"; tail -40 "$SCRATCH/boot.log" || true
        fi
    else
        printf '%sFAIL%s the app did not boot. Last 40 lines:\n' "$RED" "$OFF"
        tail -40 "$SCRATCH/boot.log" || true
        record "app boots" "FAIL"; record "end-to-end flow" "SKIP"
    fi
    kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; APP_PID=""
fi

# ── Gate 5: every route is classified ───────────────────────────────────────

step "autumn routes audit"
if command -v autumn >/dev/null 2>&1; then
    if (cd crates/apiary-app && autumn routes audit); then record "routes audit" "PASS"
    else record "routes audit" "FAIL"; fi
else
    printf '%sskipped%s autumn CLI not on PATH (cargo install autumn-cli --version 0.9.0)\n' "$YELLOW" "$OFF"
    record "routes audit" "SKIP"
fi

# ── Gate 6: Verus over the verified core ────────────────────────────────────

step "verus"
VERUS=""
for candidate in "$(command -v verus 2>/dev/null || true)" "${VERUS_BIN:-}" \
    "$HOME/verus/verus" "$HOME/verus/source/target-verus/release/verus"; do
    [ -n "$candidate" ] && [ -x "$candidate" ] && { VERUS="$candidate"; break; }
done
if [ -z "$VERUS" ]; then
    printf '%sFAIL%s no Verus binary found (PATH, $VERUS_BIN, ~/verus/verus, ~/verus/source/target-verus/release/verus)\n' "$RED" "$OFF"
    record "verus" "FAIL"
else
    printf 'using %s\n' "$VERUS"
    # offboard.rs first: it guards the one irreversible thing Apiary does.
    # docs/SPEC.md section 7 ranks both by how likely they are to need a pass.
    for source in verus/src/offboard.rs verus/src/storm.rs; do
        name="verus $(basename "$source" .rs)"
        printf '\n  %s\n' "$source"
        if "$VERUS" "$source"; then printf '  %sverified%s\n' "$GREEN" "$OFF"; record "$name" "PASS"
        else printf '  %snot verified%s — see docs/SPEC.md section 7\n' "$RED" "$OFF"; record "$name" "FAIL"; fi
    done
fi

# ── Summary ─────────────────────────────────────────────────────────────────

printf '\n%s─────────── apiary verify ───────────%s\n' "$BOLD" "$OFF"
i=0
while [ "$i" -lt "${#GATES[@]}" ]; do
    case "${STATUSES[$i]}" in PASS) c="$GREEN" ;; FAIL) c="$RED" ;; *) c="$YELLOW" ;; esac
    printf '  %-22s %s%s%s\n' "${GATES[$i]}" "$c" "${STATUSES[$i]}" "$OFF"
    i=$((i + 1))
done
printf '%s─────────────────────────────────────%s\n' "$BOLD" "$OFF"
if [ "$FAILED" -ne 0 ]; then printf '%sverification failed%s\n' "$RED" "$OFF"; exit 1; fi
printf '%sall gates passed%s\n' "$GREEN" "$OFF"
