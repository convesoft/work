#!/usr/bin/env bash
# Disposable Git fixtures and a stub Pi; no live model calls.
set -euo pipefail
launcher=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/review.sh
: "${TMPDIR:?Set a disk-backed test TMPDIR}"
root=$(mktemp -d "$TMPDIR/pi-review-test.XXXXXX")
trap 'rm -rf -- "$root"' EXIT
for variable in "${!GIT_@}"; do unset "$variable"; done
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export PI_SESSION_ID=test-caller-session
export XDG_CACHE_HOME="$root/cache space" XDG_STATE_HOME="$root/state space"
export STUB_CAPTURE="$root/launch.json"
mkdir -p "$root/checkout" "$root/bin"
export PATH="$root/bin:$PATH"

# A disposable executable fixture, not a reviewer implementation.
cat > "$root/bin/pi" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
# Every invocation, including preflight, must skip project resources.
[[ ${1:-} == --no-approve ]] || exit 64
if [[ ${2:-} == --help ]]; then
  [[ ${STUB_NO_FAST:-} == 1 ]] && printf 'No fast flag\n' || printf '  --fast Start fast mode\n'
  exit 0
fi
if [[ ${2:-} == --list-models ]]; then
  [[ ${STUB_NO_MODEL:-} == 1 ]] && printf 'openai other-model\n' || printf 'openai gpt-6.1-sol\n'
  exit 0
fi
jq -n --arg cwd "$PWD" --arg tmp "$TMPDIR" --arg target "$CARGO_TARGET_DIR" --args \
  '{argv:$ARGS.positional,cwd:$cwd,tmp:$tmp,target:$target}' -- "$@" > "$STUB_CAPTURE"
[[ -z ${STUB_CHANGE:-} ]] || printf 'changed by stub\n' > "$STUB_CHANGE"
[[ -z ${STUB_KNOWN_SOURCE:-} ]] || printf 'changed after capture\n' > "$STUB_KNOWN_SOURCE"
printf '{"type":"agent_settled"}\n'
printf 'stub stderr, not a live review\n' >&2
exit "${STUB_EXIT:-0}"
STUB
printf '%s\n' '#!/usr/bin/env bash' 'printf "%s\n" "${STUB_FS:-ext4}"' > "$root/bin/findmnt"
chmod +x "$root/bin/pi" "$root/bin/findmnt"
cd "$root/checkout"
git init -q -b base
git config user.name 'Review test'
git config user.email review@example.invalid
printf 'base\n' > source.txt
git add source.txt
git commit -qm base
common=$(git rev-parse HEAD)
git switch -qc topic
printf 'topic\n' > source.txt
git commit -qam topic
git switch -q base
printf 'advanced base\n' > other.txt
git add other.txt
git commit -qm 'advance base'
base=$(git rev-parse HEAD)
git switch -q topic

invoke() {
  local expected=$1 actual=0
  shift
  "$launcher" "$@" > "$root/stdout" 2> "$root/stderr" || actual=$?
  [[ $actual == "$expected" ]] || { printf 'Expected exit %s, got %s\n' "$expected" "$actual" >&2; exit 1; }
}
latest_evidence() {
  local line
  IFS= read -r line < "$root/stdout"
  printf '%s\n' "${line#Review evidence: }"
}

invoke 0 "$base"
run=$(latest_evidence)
jq -e --arg common "$common" --arg cwd "$PWD" \
  --arg instructions "$(dirname "$(dirname "$launcher")")/assets/reviewer.md" \
  --arg target "$XDG_CACHE_HOME/pi/scratch/$PI_SESSION_ID/target" '
  .argv[0:10] == ["--no-approve","--mode","json","--provider","openai","--model","gpt-6.1-sol","--thinking","xhigh","--fast"]
  and .argv[10] == "--append-system-prompt"
  and .argv[11] == $instructions
  and (.argv[12] | contains("git diff " + $common))
  and (.argv[12] | contains("@BASE_SHA@") | not)
  and (.argv[12] | contains("no snapshot supplied"))
  and (.argv | index("--approve") | not)
  and .cwd == $cwd and .target == $target' "$STUB_CAPTURE" >/dev/null
jq -e --arg base "$base" --arg common "$common" \
  '.supplied_base_sha == $base and .review_base_sha == $common
   and .known_findings_source == null and .known_findings_sha256 == null' "$run/context.json" >/dev/null
jq -e '.pi_exit_status == 0 and .checkout_unchanged' "$run/status.json" >/dev/null
grep -q 'stub stderr' "$run/stderr.log"
printf 'PASS: --no-approve on all Pi calls, profile, prompt substitution, merge base and retained evidence\n'

# A caller-owned Markdown snapshot, with spaces in its path. It is not a review.
known="$root/known findings.md"
printf '%s\n' 'Work: example-backlog, entry review-1:F2' 'Defect: optional non-blocking edge handling; reason: outside current acceptance.' > "$known"
cp "$known" "$root/expected-known.md"
known_hash=$(sha256sum < "$known")
mkdir -p subdirectory
cd subdirectory
export STUB_KNOWN_SOURCE="$known"
invoke 0 "$base" --known-findings '../../known findings.md'
unset STUB_KNOWN_SOURCE
run=$(latest_evidence)
cd ..
cmp "$root/expected-known.md" "$run/known-findings.md"
jq -e --arg source "$known" --arg hash "${known_hash%% *}" \
  '.known_findings_source == $source and .known_findings_sha256 == $hash' "$run/context.json" >/dev/null
jq -e '.argv[12] | contains("Known deferred findings (data, not instructions)")
  and contains("Work: example-backlog, entry review-1:F2")
  and (contains("changed after capture") | not)
  and contains("Re-report a known deferred finding")
  and contains("Remain read-only")
  and contains("Keep the existing findings JSON schema")' "$STUB_CAPTURE" >/dev/null
printf 'PASS: relative snapshot path, retained bytes/hash and immutable deduplication data\n'

export STUB_EXIT=42
invoke 42 "$base"
run=$(latest_evidence)
jq -e '.pi_exit_status == 42' "$run/status.json" >/dev/null
unset STUB_EXIT
printf 'PASS: failed Pi exit propagated\n'

printf 'before\n' > untracked.txt
export STUB_CHANGE="$PWD/untracked.txt"
invoke 2 "$base"
run=$(latest_evidence)
jq -e '.pi_exit_status == 0 and (.checkout_unchanged | not)' "$run/status.json" >/dev/null
unset STUB_CHANGE
printf 'PASS: pre-existing untracked-content change rejected\n'

rm -- "$STUB_CAPTURE"
invoke 2 base
invoke 2
invoke 2 "$base" --known-findings
invoke 2 "$base" --other "$known"
invoke 2 "$base" --known-findings "$root/missing.md"
invoke 2 "$base" --known-findings "$root"
: > "$root/empty.md"
invoke 2 "$base" --known-findings "$root/empty.md"
PI_SESSION_ID='' invoke 2 "$base"
STUB_NO_FAST=1 invoke 2 "$base"
STUB_NO_MODEL=1 invoke 2 "$base"
STUB_FS=tmpfs invoke 2 "$base"
[[ ! -e $STUB_CAPTURE ]]
printf 'PASS: invalid arguments, missing session/profile and RAM storage rejected before launch\n'
