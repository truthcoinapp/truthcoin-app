#!/bin/sh
# Make a Truthcoin App release's tag, signed with the release key (git's SSH signing), and check the signature against
# the public key pinned here (truthcoinapp-release.pub). For the operator, on the machine that holds the key, after a
# green dry run. Usage: sign-tag.sh v0.1.0
#
# It tags only the commit that release-app.sh dry built and checked (release-<tag>/dry.commit in release-app.sh's
# cache), so the source people review is the source the packages came from. Nothing is pushed: release-app.sh
# <version> release pushes the tag, and refuses one that isn't signed by the release key.
set -eu
V=${1:?version tag, e.g. v0.1.0}
ID=truthcoinapp-release
K=${TRUTHCOINAPP_RELEASE_KEY:-$HOME/.ssh/truthcoinapp-release}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
PUB=$HERE/truthcoinapp-release.pub
CACHE=${TRUTHCOINAPP_RELEASE_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/truthcoin-app-release}
DRY=$CACHE/release-$V/dry.commit
stop() { echo "STOP: $*. Not tagged." >&2; exit 1; }

# The key: its public half pinned here (and committed), its private half at $K.
[ -f "$PUB" ] || stop "no release key yet: $PUB is missing. Make it once (README.md here, step 1)"
[ -f "$K" ] || stop "no private key at $K (set TRUTHCOINAPP_RELEASE_KEY to where it is)"
if [ -f "$K.pub" ]; then
    [ "$(cut -d' ' -f1,2 "$K.pub")" = "$(cut -d' ' -f1,2 "$PUB")" ] || stop "$K isn't the release key pinned in $PUB"
fi

[ -f "$DRY" ] || stop "no green dry run here: run $HERE/release-app.sh ${V#v} dry first"
SHA=$(cat "$DRY")
git -C "$ROOT" cat-file -e "$SHA^{commit}" 2>/dev/null || stop "the dry run's commit $SHA isn't in $ROOT"
git -C "$ROOT" rev-parse -q --verify "refs/tags/$V" >/dev/null && stop "tag $V exists already here"
git -C "$ROOT" show "$SHA:release/truthcoinapp-release.pub" 2>/dev/null | cmp -s - "$PUB" \
    || stop "$SHA doesn't hold the release key pinned in $PUB"

echo "Tagging $V at the dry run's commit:"
git -C "$ROOT" log -1 --format='  %H%n  %s' "$SHA"
git -C "$ROOT" -c gpg.format=ssh -c user.signingKey="$K" tag -s "$V" -m "Truthcoin App $V" "$SHA"

T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
awk -v id="$ID" 'NR == 1 {print id, $1, $2}' "$PUB" > "$T/allowed_signers"
if ! git -C "$ROOT" -c gpg.ssh.allowedSignersFile="$T/allowed_signers" -c gpg.minTrustLevel=fully tag -v "$V" >/dev/null; then
    git -C "$ROOT" tag -d "$V" >/dev/null
    stop "the tag's signature doesn't check against $PUB (the tag was removed)"
fi
echo "Signed tag $V at $SHA. Next: $HERE/release-app.sh ${V#v} release"
