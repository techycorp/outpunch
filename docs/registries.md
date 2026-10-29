# Package Registries

## Published

Every package is published at the same version on every release (see [releasing.md](releasing.md)).

| Registry | Packages | Owner account | CI auth |
|----------|----------|---------------|---------|
| crates.io | `outpunch`, `outpunch-axum`, `outpunch-client` | TechyCorp (GitHub user TechyCorp-bot) | Trusted publishing (`release.yml`) |
| PyPI | `outpunch` | `techycorp` | Trusted publishing (`release.yml`) |
| RubyGems | `outpunch-ruby`, `outpunch-ruby-rack`, `outpunch-ruby-rails` | `techycorps` | Trusted publishing (`release.yml`) |
| npm | `outpunch`, `outpunch-darwin-arm64`, `outpunch-darwin-x64`, `outpunch-linux-arm64-gnu`, `outpunch-linux-x64-gnu` | `techycorp` | Trusted publishing (`release.yml`) |

The RubyGems names `outpunch`, `outpunch-rack` and `outpunch-rails` were ours but are orphaned at
0.1.x under a RubyGems account we lost access to. If it is recovered, we move back to those names.

## Unclaimed

| Registry | Language | URL | Name | Status |
|----------|----------|-----|------|--------|
| NuGet | C# / .NET | nuget.org | outpunch | unclaimed |
| Maven Central | Java / Kotlin | central.sonatype.com | io.github.techycorp.outpunch | unclaimed |
| Go modules | Go | pkg.go.dev | github.com/techycorp/outpunch | auto-claimed |
| Hex | Elixir / Erlang | hex.pm | outpunch | unclaimed |
| Packagist | PHP | packagist.org | outpunch/outpunch | unclaimed |
| Swift Package Registry | Swift | swiftpackageindex.com | (auto-claimed by repo) | auto-claimed |
| CocoaPods | iOS/macOS | cocoapods.org | outpunch | unclaimed |
| Pub | Dart / Flutter | pub.dev | outpunch | unclaimed |
| Hackage | Haskell | hackage.haskell.org | outpunch | unclaimed |
| CPAN | Perl | metacpan.org | outpunch | unclaimed |
| LuaRocks | Lua | luarocks.org | outpunch | unclaimed |

## Rust FFI Integration Quality

### Tier 1: Close to PyO3/Napi-RS quality

Dedicated Rust bridge with proc macros, rich type mapping, and solved packaging.

| Language | Tool | Stars | Async | Packaging | Notes |
|----------|------|-------|-------|-----------|-------|
| Ruby | Magnus + rb-sys | 863 | Manual (no bridge) | Gem via rake-compiler + precompiled binaries | Official Bundler `--ext=rust`. Single maintainer risk. |
| Elixir | Rustler | 4,764 | Manual (dirty NIFs + tokio) | Hex via rustler_precompiled (221 dependents) | Only option. Production-proven. |
| Dart/Flutter | flutter_rust_bridge | 5,100 | Full (async fn, streams, StreamSink) | pub.dev via Cargokit | Flutter Favorite. Closest to PyO3 quality. Used by RustDesk (52k stars). |
| Swift | UniFFI | 4,453 | Partial (works, no cancellation) | SPM via XCFramework | Production-proven in Firefox iOS. Swift 6 partial. |

### Tier 2: Viable but significantly more manual

Fragmented tooling, pre-1.0 libraries, or painful packaging.

| Language | Tool | Stars | Async | Packaging | Notes |
|----------|------|-------|-------|-----------|-------|
| Java/Kotlin | UniFFI (Kotlin) or jni-rs (Java) | 4,453 / 100M dl | UniFFI: suspend fun (stability issues). jni-rs: manual CompletableFuture | No maturin-equivalent. QuestDB has a Maven plugin. | Fragmented. Android good (Mozilla ships UniFFI). Server Java = raw jni-rs. |
| PHP | ext-php-rs | 774 | php-tokio bridges Revolt/Fibers | PIE (PECL replacement) + Packagist | Pre-1.0. Small community. Complex distribution matrix. |
| C# / .NET | csbindgen | ~700 | None — manual callback-to-TaskCompletionSource | NuGet (standard) | Raw P/Invoke only. No high-level wrapper. Primarily Unity. |

### Tier 3: No good tools

| Language | Situation |
|----------|-----------|
| Go | No mature bridge. cgo + raw C ABI (manual), uniffi-bindgen-go (v0.6, young), or WASM via wazero. Worst Rust interop of any major language. |
