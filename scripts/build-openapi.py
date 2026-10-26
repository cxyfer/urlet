#!/usr/bin/env python3
"""Merge the shared OpenAPI components into each language file.

The contract is authored in three files under ``openapi/``:

* ``components.yaml`` -- schemas, responses and security schemes. Language
  neutral: every description here is English, matching the JSON API itself,
  which always answers in English whatever the visitor's interface language is.
* ``en.yaml`` / ``zh-TW.yaml`` -- ``info`` and ``paths`` with translated
  summaries, parameter descriptions and response descriptions. Each of these
  references the shared parts as ``components.yaml#/schemas/Name``.

Neither language file is a valid OpenAPI document on its own. This script
inlines the shared components, rewrites every reference to the document-local
form ``#/components/...`` and writes one self-contained document per language to
``openapi/dist/``.

Two placeholders are expanded while writing:

* ``__URLET_VERSION__`` becomes the version in ``Cargo.toml``. The version is a
  build input, so the generated documents carry it and CI catches a version bump
  that was not regenerated.
* ``__URLET_BASE_URL__`` is left alone. The base URL is deployment
  configuration rather than build input, so the binary substitutes it when it
  serves the document.

The generated files are committed because the binary embeds them with
``include_str!`` at compile time, and because the documentation site consumes
them. They are derived artifacts: never edit them by hand. Run this script with
``--check`` to verify that the committed files still match their sources; CI
does exactly that.

Output is deterministic: the key order follows the source files, so a diff
shows only real contract changes.

Requires PyYAML (``python3 -m pip install pyyaml``).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ModuleNotFoundError:  # pragma: no cover - depends on the environment
    sys.exit(
        "PyYAML is required: python3 -m pip install pyyaml\n"
        "The generated specs are committed, so this script only runs when the "
        "contract changes and in the CI check that guards them."
    )

ROOT = Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "openapi"
DIST = OPENAPI / "dist"
COMPONENTS = OPENAPI / "components.yaml"
MANIFEST = ROOT / "Cargo.toml"

# Language files, in the order they are reported.
LANGUAGES = ("en", "zh-TW")

# Prefixes that mark a reference to the shared components file.
SHARED_PREFIXES = ("components.yaml#/", "./components.yaml#/")

# Replaced while generating; the manifest is the single source for the version.
VERSION_TOKEN = "__URLET_VERSION__"

# Left in place for the binary to substitute at serve time.
BASE_URL_TOKEN = "__URLET_BASE_URL__"


def cargo_version() -> str:
    """The `version` of the root package in Cargo.toml."""
    try:
        text = MANIFEST.read_text(encoding="utf-8")
    except FileNotFoundError:
        sys.exit("Cargo.toml is missing")
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    if not match:
        sys.exit("Cargo.toml has no top-level version")
    return match.group(1)


def load_yaml(path: Path) -> dict[str, Any]:
    """Read one YAML document, failing loudly on an empty or non-mapping file."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        sys.exit(f"missing contract source: {path.relative_to(ROOT)}")
    try:
        document = yaml.safe_load(text)
    except yaml.YAMLError as error:
        sys.exit(f"{path.relative_to(ROOT)} is not valid YAML: {error}")
    if not isinstance(document, dict):
        sys.exit(f"{path.relative_to(ROOT)} must contain a mapping at the top level")
    return document


def rewrite_refs(node: Any, where: str) -> Any:
    """Point every shared reference at the merged document's own components.

    Walks the document in place and returns it. A reference that is neither
    shared nor already document-local is an error: it would silently produce a
    spec that only resolves on the machine that generated it.
    """
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str):
                for prefix in SHARED_PREFIXES:
                    if value.startswith(prefix):
                        node[key] = "#/components/" + value[len(prefix) :]
                        break
                else:
                    if not value.startswith("#/"):
                        sys.exit(
                            f"{where}: unsupported external $ref {value!r}; "
                            "the contract must only reference its own components"
                        )
            else:
                rewrite_refs(value, where)
    elif isinstance(node, list):
        for item in node:
            rewrite_refs(item, where)
    return node


def resolve_pointer(document: Any, pointer: str) -> bool:
    """Whether a document-local JSON pointer resolves."""
    node = document
    for token in pointer.lstrip("#/").split("/"):
        token = token.replace("~1", "/").replace("~0", "~")
        if isinstance(node, dict) and token in node:
            node = node[token]
        elif isinstance(node, list) and token.isdigit() and int(token) < len(node):
            node = node[int(token)]
        else:
            return False
    return True


def check_refs(node: Any, document: Any, where: str) -> None:
    """Fail on any document-local reference that points at nothing.

    Rewriting a reference to `#/components/...` proves it has the right shape,
    not that the entry exists. A typo in a schema name would otherwise ship a
    contract that only fails in the client, which is exactly the drift the
    generated artifacts are supposed to make impossible.
    """
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str) and value.startswith("#/"):
                if not resolve_pointer(document, value):
                    sys.exit(f"{where}: $ref {value!r} does not resolve")
            else:
                check_refs(value, document, where)
    elif isinstance(node, list):
        for item in node:
            check_refs(item, document, where)


def render(language: str, components: dict[str, Any], version: str) -> str:
    """Build the self-contained spec for one language as its final text."""
    path = OPENAPI / f"{language}.yaml"
    document = load_yaml(path)
    if "components" in document:
        sys.exit(
            f"{path.relative_to(ROOT)} must not define components; "
            f"they belong in {COMPONENTS.name}"
        )
    if "paths" not in document or not document["paths"]:
        sys.exit(f"{path.relative_to(ROOT)} must define paths")
    document["components"] = components
    where = path.relative_to(ROOT).as_posix()
    rewrite_refs(document, where)
    check_refs(document, document, where)
    # sort_keys stays off on purpose: the source order is meaningful for a
    # reader, and YAML loading preserves it, so the output is still stable.
    text = json.dumps(document, indent=2, ensure_ascii=False, sort_keys=False)
    return text.replace(VERSION_TOKEN, version) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed specs match their sources; write nothing",
    )
    args = parser.parse_args()

    version = cargo_version()
    components = load_yaml(COMPONENTS)
    missing = {"schemas", "responses", "securitySchemes"} - components.keys()
    if missing:
        sys.exit(f"{COMPONENTS.name} is missing: {', '.join(sorted(missing))}")

    expected = {DIST / f"urlet.{language}.json" for language in LANGUAGES}
    stale: list[str] = []
    for language in LANGUAGES:
        target = DIST / f"urlet.{language}.json"
        rendered = render(language, components, version)
        if VERSION_TOKEN in rendered:
            sys.exit(f"{target.name} still contains {VERSION_TOKEN}")
        if args.check:
            current = target.read_text(encoding="utf-8") if target.exists() else ""
            if current != rendered:
                stale.append(target.relative_to(ROOT).as_posix())
            continue
        DIST.mkdir(parents=True, exist_ok=True)
        target.write_text(rendered, encoding="utf-8")
        print(f"wrote {target.relative_to(ROOT)}")

    if args.check:
        # A leftover file would be published alongside the real ones and never
        # regenerated, so it counts as drift too.
        if DIST.is_dir():
            for path in sorted(DIST.iterdir()):
                if path.is_file() and path not in expected:
                    stale.append(f"{path.relative_to(ROOT).as_posix()} (unexpected)")
        if stale:
            print("generated specs are out of date; run scripts/build-openapi.py:", file=sys.stderr)
            for name in stale:
                print(f"  {name}", file=sys.stderr)
            return 1
        print("generated specs match their sources")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
