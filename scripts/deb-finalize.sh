#!/usr/bin/env bash
# deb-finalize.sh <tauri .deb> <out dir>
#
# Tauri names the .deb after productName, with a space ("Truthcoin App_<version>_amd64.deb"), and its desktop file
# "Truthcoin App.desktop". This repacks the .deb so that:
#   - the file is truthcoin-app_<version>_<arch>.deb, after the package it holds, with no space (a GitHub release
#     turns a space into a dot, and SHA256SUMS must list the published name);
#   - the desktop file is named after the app id, dev.truthcoinapp.desktop.desktop, which is what GNOME matches a
#     running window to (so the dock shows the app's icon, not a generic one);
#   - the package has a real long description, not Tauri's "(none)";
#   - the same commit always gives the same bytes: dpkg-deb stamps SOURCE_DATE_EPOCH, and every file belongs to root.
# Writes <out dir>/truthcoin-app_<version>_<arch>.deb and prints its path.
set -euo pipefail

in=${1:?usage: deb-finalize.sh <tauri .deb> <out dir>}
out=${2:?usage: deb-finalize.sh <tauri .deb> <out dir>}
package=truthcoin-app
app_id=dev.truthcoinapp.desktop

work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT

dpkg-deb -R "$in" "$work/pkg"
ctl="$work/pkg/DEBIAN/control"

version=$(awk -F': ' '$1 == "Version" {print $2}' "$ctl")
arch=$(awk -F': ' '$1 == "Architecture" {print $2}' "$ctl")

# Tauri already calls the package truthcoin-app (productName in kebab case); this keeps it so if productName changes.
sed -i -e "s/^Package: .*/Package: $package/" "$ctl"
grep -q '^Homepage:' "$ctl" || printf 'Homepage: https://github.com/mblowes/truthcoin-app\n' >> "$ctl"
if grep -q '^ (none)$' "$ctl"; then
  sed -i -e 's/^ (none)$/ Runs a Truthcoin node and trades its prediction markets: see the markets, your balance\n and positions, buy and sell shares, and create markets, from the desktop or a paired phone./' "$ctl"
fi

# Desktop file named after the app id; the category makes it show under Office/Finance.
apps="$work/pkg/usr/share/applications"
if [ -f "$apps/Truthcoin App.desktop" ]; then
  mv "$apps/Truthcoin App.desktop" "$apps/$app_id.desktop"
fi
[ -f "$apps/$app_id.desktop" ] || { echo "no desktop file in $in" >&2; exit 1; }
sed -i -e 's/^Categories=$/Categories=Office;Finance;/' "$apps/$app_id.desktop"

# dpkg checks installed files against md5sums; rebuild it after the changes above.
(cd "$work/pkg" && find . -path ./DEBIAN -prune -o -type f -printf '%P\0' | LC_ALL=C sort -z | xargs -0 md5sum) \
  > "$work/pkg/DEBIAN/md5sums"

mkdir -p "$out"
dest="$out/${package}_${version}_${arch}.deb"
dpkg-deb --root-owner-group -Zxz -b "$work/pkg" "$dest" >/dev/null
echo "$dest"
