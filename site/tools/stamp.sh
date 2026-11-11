#!/bin/sh
# Stamps a COPY of the site for publishing (never the source folder):
# the release version and the site's public address.
#   sh site/tools/stamp.sh _site 1.0.0 [https://samer2373.github.io/one-mk/] [sizes.tsv]
# sizes.tsv (optional): "<release file name><TAB><bytes>" lines; the download
# sizes shown on the page are rewritten from it (the source carries 1.0.0's).
# The source files carry the version 1.0.0 and the address below, so the page
# reads sensibly when opened from source. To move to a custom domain, pass
# its address here (in .github/workflows/pages.yml: SITE_URL); nothing else.
set -eu
DEFAULT_URL=https://samer2373.github.io/one-mk/
dir=$1; version=$2; url=${3:-$DEFAULT_URL}; sizes=${4:-}
case $version in [0-9]*.[0-9]*.[0-9]*) ;; *) echo "stamp.sh: '$version' is not a version" >&2; exit 1 ;; esac
case $url in https://*/) ;; https://*) url="$url/" ;; *) echo "stamp.sh: '$url' must start with https://" >&2; exit 1 ;; esac
[ -f "$dir/index.html" ] || { echo "stamp.sh: no index.html in $dir" >&2; exit 1; }
[ "$(cd "$dir" && pwd)" != "$(cd "$(dirname "$0")/.." && pwd)" ] || { echo "stamp.sh: stamp a copy, not the source" >&2; exit 1; }
for f in "$dir/index.html" "$dir/404.html"; do
  sed -i -e "s|data-version=\"[^\"]*\"|data-version=\"$version\"|g" \
         -e "s|<span data-version>[^<]*</span>|<span data-version>$version</span>|g" \
         -e "s|\"softwareVersion\":\"[^\"]*\"|\"softwareVersion\":\"$version\"|g" "$f"
done
for f in index.html 404.html robots.txt sitemap.xml; do sed -i "s|$DEFAULT_URL|$url|g" "$dir/$f"; done
sed -i "s|<lastmod>[^<]*</lastmod>|<lastmod>$(date -u +%Y-%m-%d)</lastmod>|" "$dir/sitemap.xml"
n=0
if [ -n "$sizes" ] && [ -s "$sizes" ]; then
  tab=$(printf '\t')
  while IFS="$tab" read -r name bytes; do
    case $bytes in ''|*[!0-9]*) continue ;; esac
    grep -q "data-size=\"$name\"" "$dir/index.html" || continue
    mb=$(awk -v b="$bytes" 'BEGIN { printf "%.1f MB", b / 1000000 }')
    re=$(printf '%s' "$name" | sed 's/[.]/\\./g')
    sed -i "s|\(data-size=\"$re\">\)[^<]*<|\1$mb<|g" "$dir/index.html"
    n=$((n + 1))
  done < "$sizes"
fi
echo "stamped $dir: version $version, address $url, $n file sizes"
