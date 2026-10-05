#!/usr/bin/env bash
# release-app.sh <version> <step>: the Truthcoin App's release, start to finish, each step waiting for GitHub and
# checking what it built. The operator only signs (sign-sums.sh). The order is in README.md here.
#
#   scan     nothing leaves this machine: the commits a push would publish (all of them, the first time) hold no
#            private details, in their files or their messages. dry runs it too.
#   all      dry, then release: the whole build in one command, up to the signature, when the operator's signed tag
#            exists already (otherwise it stops after dry, saying so). It stops at the first failure.
#   dry      the commit (REF, default HEAD) has the version everywhere, its release notes and the release key, and
#            passes scan; it goes to branch release-v<version> on GitHub, where release.yml builds every package and
#            publishes nothing (no tag, no release); watch the run; rebuild the Linux .deb here from the same commit in
#            the pinned image (build/linux/rebuild.sh) and compare it byte for byte with GitHub's.
#   release  after a green dry run of that commit, and once the operator has tagged it (sign-tag.sh v<version>: the tag
#            is signed with the release key, and checked here against the key pinned in release/): push the tag (and
#            main, when main doesn't hold the commit yet); release.yml builds again and leaves a draft release;
#            pages.yml deploys the phone page from the tag; watch the release run; then check.
#   check    the draft release: exactly the four packages and SHA256SUMS; every package matches SHA256SUMS and has a
#            GitHub build attestation from release.yml at the tag; the .deb equals the dry run's rebuild; the Mac app
#            (.app.tar.gz) holds Truthcoin App.app at this version and nothing else.
#            It keeps the SHA256SUMS it checked ($WORK/checked.SHA256SUMS): sign-sums.sh signs only that very file.
#   after    once the operator has signed (sign-sums.sh v<version>): SHA256SUMS.sig checks against the release key
#            pinned here (truthcoinapp-release.pub, the same file in the tag), over the very SHA256SUMS check verified;
#            the phone page's deploy from the tag is green; then the draft is published as the latest release, and
#            GitHub serves the signed SHA256SUMS.
#
# Nothing public happens until the release key exists: every step except scan stops without truthcoinapp-release.pub.
# Its work files are kept outside the repository, in $CACHE.
set -euo pipefail
V=${1:?version, e.g. 0.1.0}
STEP=${2:?scan, all, dry, release, check or after}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)                                    # this repository
REPO=truthcoinapp/truthcoin-app
REMOTE=${TRUTHCOINAPP_REMOTE:-origin}                           # the git remote that is github.com/$REPO
REF=${REF:-HEAD}                                                # the commit to release
TAG=v$V
BR=release-v$V
CACHE=${TRUTHCOINAPP_RELEASE_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/truthcoin-app-release}
WORK=${WORK:-$CACHE/release-$TAG}
KEY=$HERE/truthcoinapp-release.pub                              # the release key's public half
SIGNER=truthcoinapp-release
PAGE=https://truthcoinapp.github.io/truthcoin-app/
# The packages, as release.yml names them. Tauri's names have a space, which a GitHub release turns into a dot.
DEB=truthcoin-app_${V}_amd64.deb
APPIMAGE=Truthcoin-App_${V}_amd64.AppImage
DMG=Truthcoin-App_${V}_universal.dmg
MACAPP=Truthcoin-App_${V}_universal.app.tar.gz
# GitHub CLI with `gh attestation verify` (2.49 or later), pinned by its published hash and fetched into $CACHE when
# missing.
GH_ATTEST=${GH_ATTEST:-$CACHE/tools/gh/bin/gh}
GH_ATTEST_VERSION=2.102.0
GH_ATTEST_SHA256=bb766f710eef8ede859c18578c72c327597cd4c8a85b06001b1f3843c6019386  # gh_2.102.0_linux_amd64.tar.gz
# Private details that must never be published: tailnet addresses, local paths, private keys. Allowed exception:
# 100.64.0.x (a neutral test address). The bracketed letters keep this line from matching itself, since this file is
# published too.
LEAKS='100\.(6[5-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3}|100\.64\.[1-9][0-9]{0,2}\.[0-9]{1,3}|/mnt/[a-z]|/home/[a-z]|/tmp/[c]laude|[s]vr:|[s]cratchpad|BEGIN [A-Z ]*PRIVATE KE[Y]'

die() { echo "STOP: $*" >&2; exit 1; }
sha256() { sha256sum "$1" | cut -d' ' -f1; }
now() { date -u +%Y-%m-%dT%H:%M:%SZ; }
git() { command git -C "$ROOT" "$@"; }

# The release key's public half must be here, and a real one, before anything public happens.
need_key() {
    [ -f "$KEY" ] || die "no release key yet: $KEY is missing. Make it once (README.md here, step 1: ssh-keygen, copy the .pub here, commit), then run this again."
    grep -qE '^ssh-ed25519 [A-Za-z0-9+/]+=* ?' "$KEY" || die "$KEY isn't an ed25519 public key"
}

# allowed_signers for ssh-keygen -Y verify: the release key under its identity.
allowed_signers() { awk -v id="$SIGNER" 'NR == 1 {print id, $1, $2}' "$KEY"; }

# Tag $TAG, here or on GitHub (ref $1), is an annotated tag at commit $2 signed by the release key (git's SSH signing,
# namespace "git"). A tag signed by any other key, or not signed, fails: minTrustLevel=fully also refuses a good
# signature by a key that isn't the release key, on any git that would otherwise let it pass.
signed_tag() {
    local ref=$1 sha=$2 at
    at=$(git rev-parse -q --verify "$ref^{commit}") || return 1
    [ "$at" = "$sha" ] || die "tag $TAG is at ${at:0:12}, not ${sha:0:12}"
    [ "$(git cat-file -t "$ref")" = tag ] || die "tag $TAG isn't an annotated tag, so it can't be signed"
    mkdir -p "$WORK"
    allowed_signers >"$WORK/allowed_signers"
    git -c gpg.ssh.allowedSignersFile="$WORK/allowed_signers" -c gpg.minTrustLevel=fully verify-tag "$ref" >/dev/null 2>&1 \
        || die "tag $TAG isn't signed by the release key ($KEY)"
}

# The remote is github.com/$REPO, and is fetched.
remote_ok() {
    local url
    url=$(git remote get-url "$REMOTE" 2>/dev/null) || die "no git remote '$REMOTE' here (README.md here, step 2)"
    [[ $url =~ github\.com[:/]$REPO(\.git)?/?$ ]] || die "remote '$REMOTE' is $url, not github.com/$REPO"
    git fetch -q --tags "$REMOTE" || die "can't fetch $REMOTE"
}

# The commit to release: REF, with nothing uncommitted when it is HEAD (what is released is the commit, not the files).
commit() {
    if [ "$REF" = HEAD ] && [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        die "$ROOT has uncommitted changes: commit them first (or set REF to the commit to release)"
    fi
    git rev-parse --verify "$REF^{commit}" || die "no such commit: $REF"
}

# Every commit up to $1 that GitHub doesn't have yet: no private details in its files or its message.
scan_commits() {
    local sha=$1 c n=0
    for c in $(git rev-list "$sha" --not --remotes="$REMOTE"); do
        if git grep -nIE "$LEAKS" "$c" >&2; then die "private details above, in commit ${c:0:12}: nothing was pushed"; fi
        if git log -1 --format=%B "$c" | grep -nE "$LEAKS" >&2; then
            die "private details above, in the message of commit ${c:0:12}: nothing was pushed"
        fi
        n=$((n + 1))
    done
    echo "No private details in the $n commit(s) up to ${sha:0:12} that GitHub doesn't have yet."
}

# The version everywhere, this release's notes, and the release key, in the commit itself.
vet() {
    local sha=$1 f got
    for f in package.json src-tauri/tauri.conf.json; do
        got=$(git show "$sha:$f" | python3 -c 'import json, sys; print(json.load(sys.stdin)["version"])')
        [ "$got" = "$V" ] || die "$f says $got, not $V"
    done
    git show "$sha:src-tauri/Cargo.toml" | grep -m1 '^version' | grep -qF "\"$V\"" || die "src-tauri/Cargo.toml isn't $V"
    git cat-file -e "$sha:release-notes/$TAG.md" 2>/dev/null || die "no release-notes/$TAG.md in ${sha:0:12}"
    git show "$sha:release/truthcoinapp-release.pub" 2>/dev/null | cmp -s - "$KEY" \
        || die "${sha:0:12} doesn't hold this release key (release/truthcoinapp-release.pub): commit it first"
}

# The run of workflow $3 (default release.yml) for a branch or tag that started after $2 (it takes a few seconds to
# appear).
new_run() {
    local ref=$1 since=$2 wf=${3:-release.yml} id=""
    for _ in $(seq 1 30); do
        id=$(gh run list --repo "$REPO" --workflow "$wf" --branch "$ref" --limit 5 --json databaseId,createdAt \
            -q "[.[] | select(.createdAt >= \"$since\")][0].databaseId // empty")
        if [ -n "$id" ]; then echo "$id"; return; fi
        sleep 4
    done
    die "no $wf run for $ref started after $since"
}

watch_run() {
    local id=$1
    echo "Run $id: https://github.com/$REPO/actions/runs/$id"
    local ok=0
    gh run watch "$id" --repo "$REPO" --exit-status --interval 30 >/dev/null 2>&1 || ok=1
    gh run view "$id" --repo "$REPO" --json jobs -q '.jobs[] | "  \(.name): \(.conclusion)"'
    [ $ok = 0 ] || die "run $id failed: gh run view $id --repo $REPO --log-failed"
}

scan() {
    local sha
    remote_ok
    sha=$(commit)
    scan_commits "$sha"
}

dry() {
    local sha since id gh_deb mine
    need_key
    remote_ok
    sha=$(commit)
    vet "$sha"
    scan_commits "$sha"
    # workflow_dispatch needs release.yml on the default branch, and the first branch pushed would become the default.
    git rev-parse -q --verify "refs/remotes/$REMOTE/main" >/dev/null \
        || die "GitHub has no main yet: push it first (README.md here, step 2)"
    git diff --quiet "$sha" -- build/linux scripts/deb-finalize.sh \
        || die "build/linux or scripts/deb-finalize.sh here differs from ${sha:0:12}'s: check out ${sha:0:12} to rebuild it"
    since=$(now)
    git push -q --force "$REMOTE" "$sha:refs/heads/$BR"
    gh workflow run release.yml --repo "$REPO" --ref "$BR"
    id=$(new_run "$BR" "$since")
    watch_run "$id"
    rm -rf "$WORK/dry" "$WORK/rebuild"
    mkdir -p "$WORK/dry"
    gh run download "$id" --repo "$REPO" -n linux -D "$WORK/dry"
    gh_deb=$WORK/dry/$DEB
    [ -f "$gh_deb" ] || die "no $DEB in the run's Linux files"
    echo "Rebuilding ${sha:0:12} here in the pinned image (log: $WORK/rebuild.log)…"
    OUT=$WORK/rebuild DOCKER_BUILDKIT=${DOCKER_BUILDKIT:-0} JOBS=${JOBS:-2} "$ROOT/build/linux/rebuild.sh" "$sha" \
        >"$WORK/rebuild.log" 2>&1 || die "the rebuild failed: $WORK/rebuild.log"
    mine=$WORK/rebuild/$DEB
    cmp -s "$gh_deb" "$mine" || die "GitHub's .deb ($(sha256 "$gh_deb")) differs from the rebuild ($(sha256 "$mine"))"
    sha256 "$mine" >"$WORK/deb.sha256"
    echo "$sha" >"$WORK/dry.commit"
    echo "Dry run green, and GitHub's .deb equals the rebuild: $(cat "$WORK/deb.sha256")"
}

release() {
    local sha head ok since id
    local refs=()
    need_key
    remote_ok
    sha=$(commit)
    [ -f "$WORK/dry.commit" ] && [ "$(cat "$WORK/dry.commit")" = "$sha" ] \
        || die "no dry run of ${sha:0:12} here: run $0 $V dry first"
    git ls-remote --exit-code --tags "$REMOTE" "refs/tags/$TAG" >/dev/null && die "tag $TAG exists already on GitHub"
    head=$(git rev-parse -q --verify "refs/remotes/$REMOTE/$BR") || die "no $BR on GitHub: run dry first"
    [ "$head" = "$sha" ] || die "$BR on GitHub is at ${head:0:12}, not ${sha:0:12}: run dry again"
    ok=$(gh run list --repo "$REPO" --workflow release.yml --branch "$BR" --limit 1 --json headSha,conclusion \
        -q ".[0] | select(.headSha == \"$sha\") | .conclusion")
    [ "$ok" = success ] || die "the last dry run of $BR at ${sha:0:12} isn't green (got: ${ok:-none})"
    # The operator's tag, signed with the release key: this script never makes the tag itself.
    signed_tag "refs/tags/$TAG" "$sha" || die "no tag $TAG here yet: the operator signs it ($HERE/sign-tag.sh $TAG), then: $0 $V release"
    refs+=("refs/tags/$TAG")
    # main gets the commit unless it holds it already; a main that has moved elsewhere refuses the push.
    git merge-base --is-ancestor "$sha" "refs/remotes/$REMOTE/main" || refs+=("$sha:refs/heads/main")
    since=$(now)
    git push -q --atomic "$REMOTE" "${refs[@]}" \
        || die "the push failed, nothing was pushed (has main moved? then make the release commit on top of it and run dry again)"
    echo "Tagged $TAG at ${sha:0:12} and pushed ${refs[*]}."
    id=$(new_run "$TAG" "$since")
    watch_run "$id"
    check
}

# The pinned GitHub CLI for attestations (above), fetched and checked against its pinned hash when it isn't there yet.
attest_gh() {
    [ -x "$GH_ATTEST" ] && "$GH_ATTEST" attestation --help >/dev/null 2>&1 && return
    [ "$GH_ATTEST" = "$CACHE/tools/gh/bin/gh" ] || die "$GH_ATTEST has no gh attestation (GitHub CLI 2.49 or later)"
    local t tgz=gh_${GH_ATTEST_VERSION}_linux_amd64.tar.gz
    t=$(mktemp -d)
    gh release download "v$GH_ATTEST_VERSION" --repo cli/cli --pattern "$tgz" --dir "$t" \
        || die "couldn't fetch GitHub CLI $GH_ATTEST_VERSION"
    echo "$GH_ATTEST_SHA256  $t/$tgz" | sha256sum -c --quiet - || die "GitHub CLI $GH_ATTEST_VERSION doesn't match its pinned hash"
    tar -xzf "$t/$tgz" -C "$t"
    rm -rf "$CACHE/tools/gh"
    mkdir -p "$CACHE/tools"
    mv "$t/gh_${GH_ATTEST_VERSION}_linux_amd64" "$CACHE/tools/gh"
    rm -rf "$t"
}

# A GitHub build attestation for this file, checked by Sigstore (gh attestation verify): signed for this repository's
# release.yml, run from tag $TAG. No fallback: reading the attestations API without checking its signatures proves
# nothing.
attested() {
    "$GH_ATTEST" attestation verify "$1" --repo "$REPO" \
        --signer-workflow "$REPO/.github/workflows/release.yml" --source-ref "refs/tags/$TAG" >/dev/null 2>&1
}

check() {
    local f got want pkgs=()
    need_key
    rm -rf "$WORK/rel"
    mkdir -p "$WORK/rel"
    gh release download "$TAG" --repo "$REPO" -D "$WORK/rel"
    [ -f "$WORK/rel/SHA256SUMS" ] || die "no SHA256SUMS on $TAG"
    for f in "$WORK"/rel/*; do
        case ${f##*/} in SHA256SUMS | SHA256SUMS.sig) ;; *) pkgs+=("${f##*/}") ;; esac
    done
    got=$(printf '%s\n' "${pkgs[@]}" | sort | tr '\n' ' ')
    want=$(printf '%s\n' "$DEB" "$APPIMAGE" "$DMG" "$MACAPP" | sort | tr '\n' ' ')
    [ "$got" = "$want" ] || die "expected exactly $want; got: ${got:-nothing}"
    # SHA256SUMS lists exactly these packages, and each matches it.
    got=$(awk '{sub(/^\*/, "", $2); print $2}' "$WORK/rel/SHA256SUMS" | sort | tr '\n' ' ')
    [ "$got" = "$want" ] || die "SHA256SUMS lists $got, not $want"
    (cd "$WORK/rel" && sha256sum --quiet -c SHA256SUMS) || die "a package doesn't match SHA256SUMS"
    if [ "$(gh repo view "$REPO" --json isPrivate -q .isPrivate)" = true ]; then
        [ "${ATTEST:-}" = skip ] || die "$REPO is private, so release.yml recorded no build attestations: make it public before tagging (or ATTEST=skip $0 $V check, for a rehearsal)"
        echo "ATTEST=skip: build attestations not checked ($REPO is private)."
    else
        attest_gh
        for f in "${pkgs[@]}"; do
            attested "$WORK/rel/$f" || die "no build attestation from release.yml at $TAG for $f (checked by Sigstore)"
        done
        echo "Every package matches SHA256SUMS and has a build attestation: ${pkgs[*]}"
    fi
    mac_app "$WORK/rel/$MACAPP"
    if [ -f "$WORK/deb.sha256" ]; then
        [ "$(sha256 "$WORK/rel/$DEB")" = "$(cat "$WORK/deb.sha256")" ] || die "the released .deb differs from the dry run's rebuild"
        echo "The released .deb equals the rebuild: $(cat "$WORK/deb.sha256")"
    else
        echo "No dry run here, so the .deb wasn't compared with a rebuild."
    fi
    # What the operator signs must be what was checked here.
    cp "$WORK/rel/SHA256SUMS" "$WORK/checked.SHA256SUMS"
    echo "The checked SHA256SUMS ($(sha256 "$WORK/checked.SHA256SUMS")):"
    sed 's/^/  /' "$WORK/checked.SHA256SUMS"
    echo "$TAG is checked, and still a draft: $(gh release view "$TAG" --repo "$REPO" --json isDraft -q .isDraft)."
    echo "The operator signs: $HERE/sign-sums.sh $TAG $(sha256 "$WORK/checked.SHA256SUMS")"
    echo "Then: $0 $V after"
}

# The Mac app (.app.tar.gz) unpacks to Truthcoin App.app alone, at this version, with its program and no ._ files.
mac_app() {
    python3 - "$1" "$V" <<'PY' || die "${1##*/} isn't Truthcoin App.app at v$V alone"
import plistlib, sys, tarfile
path, version = sys.argv[1], sys.argv[2]
app = "Truthcoin App.app"
with tarfile.open(path, "r:gz") as t:
    names = t.getnames()
    assert names and all(n == app or n.startswith(app + "/") for n in names), "entries outside " + app
    assert not any("/._" in n or n.startswith("._") for n in names), "._ files"
    info = plistlib.load(t.extractfile(app + "/Contents/Info.plist"))
    assert info["CFBundleIdentifier"] == "dev.truthcoinapp.desktop", info["CFBundleIdentifier"]
    assert info["CFBundleShortVersionString"] == version, info["CFBundleShortVersionString"]
    exe = app + "/Contents/MacOS/" + info["CFBundleExecutable"]
    assert exe in names and t.getmember(exe).mode & 0o111, "no program"
print("The Mac app holds", app, version, "and nothing else:", len(names), "entries")
PY
}

# The phone page's deploy from the tag (pages.yml) is green and the page answers.
pages_ok() {
    local c
    if [ "${PAGES:-}" = skip ]; then echo "PAGES=skip: the phone page isn't checked."; return; fi
    c=$(gh run list --repo "$REPO" --workflow pages.yml --branch "$TAG" --limit 1 --json conclusion -q '.[0].conclusion // empty')
    [ "$c" = success ] || die "the phone page's deploy (pages.yml) from $TAG isn't green (got: ${c:-no run}): see README.md here, Pages; or PAGES=skip $0 $V after, to publish without it"
    curl -sf -o /dev/null "$PAGE" || die "$PAGE doesn't answer"
    echo "The phone page deployed from $TAG answers at $PAGE"
}

after() {
    local f n
    need_key
    remote_ok
    rm -rf "$WORK/sig"
    mkdir -p "$WORK/sig"
    gh release download "$TAG" --repo "$REPO" -p SHA256SUMS -p SHA256SUMS.sig -D "$WORK/sig" 2>/dev/null || true
    # gh downloads what matches and says nothing about a pattern that matched nothing.
    [ -f "$WORK/sig/SHA256SUMS.sig" ] || die "no SHA256SUMS.sig on $TAG yet: the operator runs $HERE/sign-sums.sh $TAG"
    [ -f "$WORK/checked.SHA256SUMS" ] || die "no checked SHA256SUMS here: run $0 $V check first"
    cmp -s "$WORK/sig/SHA256SUMS" "$WORK/checked.SHA256SUMS" \
        || die "GitHub's SHA256SUMS isn't the one check verified: don't publish this release until that's explained"
    git show "$TAG:release/truthcoinapp-release.pub" 2>/dev/null | cmp -s - "$KEY" \
        || die "$TAG doesn't hold the release key that is here ($KEY)"
    # The tag as GitHub has it (fetched by remote_ok) is still the operator's signed one.
    signed_tag "refs/tags/$TAG" "$(git rev-parse "$TAG^{commit}")" || die "no tag $TAG here"
    git ls-remote --tags "$REMOTE" "refs/tags/$TAG" | grep -q "^$(git rev-parse "refs/tags/$TAG")" \
        || die "GitHub's tag $TAG isn't the signed tag here"
    allowed_signers >"$WORK/sig/allowed_signers"
    ssh-keygen -Y verify -f "$WORK/sig/allowed_signers" -I "$SIGNER" -n truthcoinapp-sums -s "$WORK/sig/SHA256SUMS.sig" \
        <"$WORK/sig/SHA256SUMS" || die "SHA256SUMS.sig doesn't check against the release key"
    pages_ok
    if [ "$(gh release view "$TAG" --repo "$REPO" --json isDraft -q .isDraft)" = true ]; then
        gh release edit "$TAG" --repo "$REPO" --draft=false --latest >/dev/null
        echo "Published $TAG."
    else
        echo "$TAG was published already."
    fi
    # What anyone downloads as the latest release: these very files (GitHub may take a moment to serve them).
    for f in SHA256SUMS SHA256SUMS.sig; do
        for n in $(seq 1 12); do
            if curl -sfL "https://github.com/$REPO/releases/latest/download/$f" | cmp -s - "$WORK/sig/$f"; then break; fi
            [ "$n" = 12 ] && die "GitHub's latest release doesn't serve this $f: is $TAG marked latest?"
            sleep 10
        done
    done
    echo "$TAG is out, signed, as the latest release: https://github.com/$REPO/releases/tag/$TAG"
}

case $STEP in
scan) scan ;;
all) dry && release ;;
dry) dry ;;
release) release ;;
check) check ;;
after) after ;;
*) die "step is scan, all, dry, release, check or after" ;;
esac
