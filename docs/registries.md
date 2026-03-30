# Package Registries

## Published

| Registry | Language | URL | Name |
|----------|----------|-----|------|
| npm | JS/TS | npmjs.com | outpunch |
| PyPI | Python | pypi.org | outpunch |
| crates.io | Rust | crates.io | outpunch |

## Unclaimed

| Registry | Language | URL | Name | Status |
|----------|----------|-----|------|--------|
| NuGet | C# / .NET | nuget.org | outpunch | unclaimed |
| Maven Central | Java / Kotlin | central.sonatype.com | io.github.joenap.outpunch | unclaimed |
| RubyGems | Ruby | rubygems.org | outpunch | unclaimed |
| Go modules | Go | pkg.go.dev | github.com/joenap/outpunch | auto-claimed |
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
