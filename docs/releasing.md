# Releasing

Every outpunch package, in every language, is published at the same version: the release tag's.
A release publishes every package, even ones that did not change. When a package misses a
release, it skips forward to the next shared version; versions are never back-filled.

## Cutting a release

1. Bump every package:

   ```sh
   python scripts/version.py bump 0.6.0
   ```

   This rewrites each version field and refreshes the lockfiles (Cargo, Bundler, npm, uv), then
   runs the check below. It needs `cargo`, `bundle`, `npm` and `uv` on the PATH.

2. Add a `## v0.6.0` section to `CHANGELOG.md`.

3. Confirm, then commit and push:

   ```sh
   python scripts/version.py check 0.6.0
   ```

4. Wait for CI to pass on that commit, then run a dry run of the release on it. Every build runs
   (all wheels and native binaries); nothing is published:

   ```sh
   gh workflow run release.yml --ref master
   ```

5. When the dry run passes, tag and push the tag:

   ```sh
   git tag v0.6.0 && git push origin v0.6.0
   ```

## Where the version lives

`scripts/version.py` owns the list. Most packages inherit the version rather than repeating it:

| Package | Source of its version |
|---|---|
| Rust crates, Python binding, Node native crate | `[workspace.package] version` in the root `Cargo.toml` |
| Python package (PyPI) | `dynamic = ["version"]`: maturin reads it from Cargo |
| `outpunch-axum`, `outpunch-client` dependency on `outpunch` | `[workspace.dependencies]` in the root `Cargo.toml` |
| Native Ruby gem's crate dependencies | `bindings/ruby/ext/outpunch/Cargo.toml` (outside the workspace) |
| npm main and platform packages | `bindings/node/package.json`, `bindings/node/npm/*/package.json`, `bindings/node/index.js` |
| npm optional dependencies on platform packages | Added by the release at publish time, not checked in |
| Ruby gems | `outpunch-ruby.gemspec`, `rack/version.rb`, `rails/version.rb`; Rack and Rails pin their dependency to their own version |

`python scripts/version.py check` (run in CI) fails if any of these disagree or cannot be found.

## What the release workflow does

On a `v*` tag, `.github/workflows/release.yml`:

1. **verify-version:** fails unless every package and the changelog match the tag.
2. **Builds** every artifact. Nothing publishes until all builds succeed, so a failed build can't
   leave a release half-published.
3. **Publishes** to crates.io, PyPI, npm and RubyGems. crates.io, PyPI and RubyGems use trusted
   publishing (short-lived OIDC tokens; no stored secrets). npm uses `NPM_REGISTRY_TOKEN` until
   its trusted publishing is configured.
4. **verify-published:** waits (up to 20 minutes) until every package is served at the new version
   by its registry. A publish command can succeed before the registry serves it.
5. **Creates the GitHub release** only if every package is live.

### Failures are never silent

- Each publish runs through `.github/scripts/publish-if-missing.sh`. It asks the registry's API
  whether that exact version exists: if it does, the publish is skipped with a warning (so a failed
  release can be re-run); if not, the publish command runs and its exit status is the step's. Any
  unexpected registry response fails the step.
- PyPI is per file, so it uses the publish action's `skip-existing` instead.
- CI fails if `release.yml` or `.github/scripts/` contain `continue-on-error`, `|| true`,
  `|| echo`, `|| :`, `|| exit 0` or `set +e`.
- `.github/workflows/release-failure-check.yml` runs every publish command against the real
  registries with credentials that must be rejected, and fails unless each one fails. It runs
  whenever the release workflow or its scripts change, and on demand.

### Re-running a failed release

If the failure is transient (a registry outage), use **Re-run failed jobs**: packages already
published are skipped, the rest are published.

If the fix needs a code or workflow change, a re-run can't pick it up (it uses the tagged commit).
Don't move the tag; bump to the next patch version and release that. Packages that missed the
failed version skip it.
