"""Bump or check the single version shared by every outpunch package.

    python scripts/version.py bump 0.5.0     rewrite every version field, then refresh lockfiles
    python scripts/version.py check          fail unless every package has the same version
    python scripts/version.py check 0.5.0    fail unless every package is at 0.5.0 and CHANGELOG.md has "## v0.5.0"

Every field must be found where it is expected; a missing field is an error, never a skip.
"""

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")

NPM_PLATFORM_DIR = "bindings/node/npm"
NODE_PACKAGE = "bindings/node/package.json"

# (file, pattern capturing the version in group 1, exact number of matches; None means at least one)
FIELDS = [
    ("Cargo.toml", r'^\[workspace\.package\]\nversion = "([^"]+)"', 1),
    ("Cargo.toml", r'^outpunch(?:-client)? = \{ version = "([^"]+)"', 2),
    ("bindings/ruby/ext/outpunch/Cargo.toml", r'^(?:outpunch-client|outpunch_core) = \{ version = "([^"]+)"', 2),
    (NODE_PACKAGE, r'^  "version": "([^"]+)"', 1),
    ("bindings/node/index.js", r"bindingPackageVersion !== '([^']+)'", None),
    ("bindings/node/index.js", r"expected ([^ ]+) but got", None),
    ("bindings/ruby/outpunch-ruby.gemspec", r'^  spec\.version = "([^"]+)"', 1),
    ("adapters/ruby/rack/lib/outpunch/rack/version.rb", r'VERSION = "([^"]+)"', 1),
    ("adapters/ruby/rails/lib/outpunch/rails/version.rb", r'VERSION = "([^"]+)"', 1),
]

# Cargo packages that must inherit the workspace version.
WORKSPACE_MEMBERS = [
    "crates/outpunch",
    "crates/outpunch-axum",
    "crates/outpunch-client",
    "bindings/python",
    "bindings/node",
]

LOCK_COMMANDS = [
    (".", ["cargo", "update", "--workspace"]),
    ("bindings/ruby", ["cargo", "update", "--workspace"]),
    ("bindings/ruby", ["bundle", "lock"]),
    ("adapters/ruby/rack", ["bundle", "lock"]),
    ("adapters/ruby/rails", ["bundle", "lock"]),
    ("bindings/node", ["npm", "install", "--package-lock-only", "--ignore-scripts"]),
    ("bindings/python", ["uv", "lock"]),
]


def all_fields():
    npm_platforms = sorted((ROOT / NPM_PLATFORM_DIR).glob("*/package.json"))
    platform_fields = [(str(p.relative_to(ROOT)), r'^  "version": "([^"]+)"', 1) for p in npm_platforms]
    return FIELDS + platform_fields


def find(path, pattern, expected):
    text = (ROOT / path).read_text()
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    if expected is None and not matches:
        raise SystemExit(f"{path}: no match for {pattern!r}")
    if expected is not None and len(matches) != expected:
        raise SystemExit(f"{path}: expected {expected} match(es) for {pattern!r}, found {len(matches)}")
    return text, matches


def workspace_inheritance_errors():
    manifests = {m: tomllib.loads((ROOT / m / "Cargo.toml").read_text()) for m in WORKSPACE_MEMBERS}
    return [
        f"{m}/Cargo.toml: version must be `version.workspace = true`"
        for m, manifest in manifests.items()
        if manifest["package"].get("version") != {"workspace": True}
    ]


def python_version_errors():
    project = tomllib.loads((ROOT / "bindings/python/pyproject.toml").read_text())["project"]
    dynamic = "version" not in project and "version" in project.get("dynamic", [])
    return [] if dynamic else ['bindings/python/pyproject.toml: version must be dynamic = ["version"]']


def node_package_errors():
    package = json.loads((ROOT / NODE_PACKAGE).read_text())
    platforms = len(list((ROOT / NPM_PLATFORM_DIR).glob("*/package.json")))
    targets = len(package["napi"]["targets"])
    return [
        *([f"{NODE_PACKAGE}: optionalDependencies are added at publish time, not checked in"]
          if "optionalDependencies" in package else []),
        *([f"{NPM_PLATFORM_DIR}: {platforms} platform packages for {targets} napi targets"]
          if platforms != targets else []),
    ]


def structural_errors():
    return workspace_inheritance_errors() + python_version_errors() + node_package_errors()


def versions_found():
    return [(m.group(1), path) for path, pattern, count in all_fields() for m in find(path, pattern, count)[1]]


def agreement_errors(found):
    versions = sorted({v for v, _ in found})
    return [] if len(versions) == 1 else [
        "packages disagree: " + "; ".join(f"{v}: {sorted({p for fv, p in found if fv == v})}" for v in versions)
    ]


def expected_version_errors(found, expected):
    mismatched = sorted({p for v, p in found if v != expected})
    changelog_ok = f"## v{expected}\n" in (ROOT / "CHANGELOG.md").read_text()
    return [
        *([f"expected {expected} in {mismatched}"] if mismatched else []),
        *([] if changelog_ok else [f'CHANGELOG.md: no "## v{expected}" section']),
    ]


def check(expected=None):
    found = versions_found()
    version_errors = agreement_errors(found) if expected is None else expected_version_errors(found, expected)
    errors = structural_errors() + version_errors
    if errors:
        raise SystemExit("version check failed:\n  " + "\n  ".join(errors))
    print(f"all packages at {found[0][0]}")


def bump(version):
    for path, pattern, count in all_fields():
        text, matches = find(path, pattern, count)
        for m in reversed(matches):
            text = text[: m.start(1)] + version + text[m.end(1) :]
        (ROOT / path).write_text(text)
    for cwd, command in LOCK_COMMANDS:
        print(f"$ ({cwd}) {' '.join(command)}", flush=True)
        subprocess.run(command, cwd=ROOT / cwd, check=True)
    check(None)


def main(argv):
    if len(argv) == 2 and argv[0] == "bump" and VERSION_RE.match(argv[1]):
        bump(argv[1])
    elif len(argv) == 1 and argv[0] == "check":
        check()
    elif len(argv) == 2 and argv[0] == "check" and VERSION_RE.match(argv[1]):
        check(argv[1])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
