#!/usr/bin/env bash
#
# Refresh the vendored Scalar API reference bundle.
#
# The reference is served by the binary itself, so the bundle is committed and
# embedded with `include_str!`. Nothing is fetched at run time: a self-hosted
# instance has to work on a network with no outbound access, and the page must
# not leak a request to a third party.
#
# The version and the digest are pinned together so the asset is reproducible.
# To upgrade, change SCALAR_VERSION, run the script, and copy the digest it
# prints back into SCALAR_SHA256.
#
# Usage:
#   scripts/update-scalar.sh            # verify and write the pinned bundle
#   scripts/update-scalar.sh --print   # show the digest without writing

set -euo pipefail

SCALAR_VERSION="1.76.0"
SCALAR_SHA256="dcb027152bf20bf2f6dcd66e8f9fb073c1a9e274cf0b94b939a7d99ab8947060"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${REPO_ROOT}/src/assets/scalar.js"
URL="https://cdn.jsdelivr.net/npm/@scalar/api-reference@${SCALAR_VERSION}/dist/browser/standalone.js"

print_only=false
if [[ "${1:-}" == "--print" ]]; then
    print_only=true
fi

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT

echo "fetching @scalar/api-reference ${SCALAR_VERSION}"
curl --fail --silent --show-error --location --max-time 300 -o "${tmp}" "${URL}"

digest="$(sha256sum "${tmp}" | cut -d' ' -f1)"
size="$(wc -c < "${tmp}")"
echo "size:   ${size} bytes"
echo "sha256: ${digest}"

if [[ "${print_only}" == true ]]; then
    exit 0
fi

if [[ "${SCALAR_SHA256}" != "${digest}" ]]; then
    echo "refusing to write: the pinned digest does not match" >&2
    echo "  pinned:  ${SCALAR_SHA256}" >&2
    echo "  fetched: ${digest}" >&2
    echo "If the version was just bumped on purpose, put the fetched digest" >&2
    echo "into SCALAR_SHA256 in this script and run it again." >&2
    exit 1
fi

install -m 0644 "${tmp}" "${TARGET}"
echo "wrote src/assets/scalar.js"
