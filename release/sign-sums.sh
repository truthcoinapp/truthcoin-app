#!/bin/sh
# Sign a Truthcoin App release's SHA256SUMS with the release key, check the signature against the public key pinned
# here (truthcoinapp-release.pub), and upload it to the release, which is still a draft. For the operator, on the
# machine that holds the key. Usage: sign-sums.sh v0.1.0 [sha256 of the checked SHA256SUMS]
#
# It signs only the SHA256SUMS that release-app.sh check verified: GitHub's copy must be byte for byte the checked file
# (release-<tag>/checked.SHA256SUMS in release-app.sh's cache, or $TRUTHCOINAPP_CHECKED_SUMS), or have the sha256 that
# check printed (the second argument, for signing on another machine). With neither, it refuses. So a file swapped
# after the check is never signed. Then release-app.sh <version> after publishes the release.
set -eu
V=${1:?version tag, e.g. v0.1.0}
WANT=${2:-}
R=truthcoinapp/truthcoin-app
ID=truthcoinapp-release
K=${TRUTHCOINAPP_RELEASE_KEY:-$HOME/.ssh/truthcoinapp-release}
HERE=$(cd "$(dirname "$0")" && pwd)
PUB=$HERE/truthcoinapp-release.pub
CACHE=${TRUTHCOINAPP_RELEASE_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/truthcoin-app-release}
CHECKED=${TRUTHCOINAPP_CHECKED_SUMS:-$CACHE/release-$V/checked.SHA256SUMS}
stop() { echo "STOP: $*. Not signed." >&2; exit 1; }

# The key: its public half pinned here (and committed), its private half at $K.
[ -f "$PUB" ] || stop "no release key yet: $PUB is missing. Make it once (README.md here, step 1)"
[ -f "$K" ] || stop "no private key at $K (set TRUTHCOINAPP_RELEASE_KEY to where it is)"
if [ -f "$K.pub" ]; then
    [ "$(cut -d' ' -f1,2 "$K.pub")" = "$(cut -d' ' -f1,2 "$PUB")" ] || stop "$K isn't the release key pinned in $PUB"
fi

T=$(mktemp -d); trap 'rm -rf "$T"' EXIT; cd "$T"
gh release view "$V" --repo "$R" --json assets -q '.assets[].name' > assets
grep -qxF SHA256SUMS.sig assets && stop "$V has a SHA256SUMS.sig already: run release-app.sh ${V#v} after"
gh release download "$V" --repo "$R" --pattern SHA256SUMS
GOT=$(sha256sum SHA256SUMS | cut -d' ' -f1)
echo "SHA256SUMS of $V ($GOT):"
sed 's/^/  /' SHA256SUMS
if [ -f "$CHECKED" ]; then
    cmp -s SHA256SUMS "$CHECKED" || stop "GitHub's SHA256SUMS isn't the one release-app.sh check verified ($CHECKED)"
    echo "It is the file release-app.sh check verified."
elif [ -n "$WANT" ]; then
    [ "$GOT" = "$WANT" ] || stop "its sha256 isn't $WANT, the one release-app.sh check printed"
    echo "Its sha256 is the one release-app.sh check printed."
else
    echo "STOP: nothing to compare it with. Run release-app.sh ${V#v} check here first, or give the sha256 it printed:" >&2
    echo "  $0 $V <sha256>" >&2
    exit 1
fi
ssh-keygen -Y sign -f "$K" -n truthcoinapp-sums SHA256SUMS
awk -v id="$ID" 'NR == 1 {print id, $1, $2}' "$PUB" > allowed_signers
ssh-keygen -Y verify -f allowed_signers -I "$ID" -n truthcoinapp-sums -s SHA256SUMS.sig < SHA256SUMS \
    || stop "the signature doesn't check against $PUB"
gh release upload "$V" --repo "$R" SHA256SUMS.sig
echo "Signed and uploaded SHA256SUMS.sig for $V. Next: $HERE/release-app.sh ${V#v} after"
