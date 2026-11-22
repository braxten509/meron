# Meron project structure

Read this guide before changing a shared feature. Keep its behavior in the lowest layer that owns it, then expose it through the existing desktop or mobile interface.

| Location | Responsibility |
| --- | --- |
| `meron-core/src/imap/` | IMAP sessions, folder operations, message data, flags and transfers. The parent module exposes the public API. |
| `meron-core/src/engine/` | Coordinate accounts, connections, synchronization and retries. |
| `meron-core/src/store/` | SQLite queries, migrations and transactions. Keep feature-specific storage tests in `store/tests/`. |
| `meron-core/src/protocol/` | Request and response shapes shared by the hosts. Keep protocol behavior tests in `protocol/tests/`. |
| `meron-core/src/sidecar/` | Desktop process dispatch. Mobile uses the Rust FFI entry point. |
| `desktop/` | Go host, Wails command routing, native windows, OS integration and release resources. |
| `desktop/frontend/src/` | React components grouped by feature, observable state in `states/`, shared UI helpers in `lib/`. |
| `mobile/shared/` | Kotlin protocol bindings and models used on both platforms. |
| `mobile/ui/` | Shared Compose screens and state. Platform source sets implement the Android and iOS adapters. |
| `mobile/android/`, `mobile/ios/` | Platform entry points, OS services, notifications and packaging. |
| `locales/` | Canonical translations for English, Brazilian Portuguese and Russian. Run `bun run i18n:generate` after changing them. |
| `scripts/`, `.githooks/`, `.github/` | Build, localization, structure checks, commit checks and CI. |
| `docs/`, `screenshots/`, `metadata/`, `snap/`, `mobile/fastlane*/` | Documentation, store listings, packaging and release automation. |
| `vendor/` | Upstream dependencies and their local patches. Preserve their upstream layout. |

## Working on a feature

A change to which messages belong in a thread starts in Rust. How a thread looks belongs to the desktop or mobile UI. A background notification depends on platform APIs and belongs to its native host. Keep protocol keys, database columns, preference keys and OS entry points compatible across releases.

Prefer feature names that explain where to look. Preserve Go package and build-tag conventions, Rust snake case, Kotlin and React component names, the existing `MeronMobileState+Feature.kt` naming scheme for Kotlin extensions, and established protocol terms such as IMAP, UID and TLS. Keep imports, related types, the public entry point and its helpers together.

Aim for at most 600 code lines per file. Split by responsibility with its tests. Document any cohesive exception in the table below. Open each source file with three lines that identify its job and its callers. Keep documentation paragraphs to four lines.

Run `python3 scripts/structure/check_structure.py` from the repository. The existing commit hook runs gofmt, rustfmt, Prettier, ktlint and SwiftFormat when available, and validates the staged translations. CI runs the structure check on clean checkouts. Existing violations are tracked in `docs/structure_old_files.txt`; remove paths as they are brought into compliance. A new file must pass.

## Long-file exceptions

| File | Reason |
| --- | --- |

```settings
conventional_paths: src res
max_code_lines: 600
min_note_lines: 3
paragraph_lines: 4
old_list: docs/structure_old_files.txt
any_name: .* .github/** .githooks/** desktop/build/** **/assets/** mobile/gradle/** mobile/**/res/** mobile/ios/**/Assets.xcassets/** metadata/** screenshots/** vendor/** mobile/fastlane/** mobile/fastlane-config/** locales/*
dash_names: meron-core/** scripts/** mobile/** desktop/** docs/** **/*.md
capital_names: desktop/** mobile/** **/*.md **/Cargo.* LICENSE* **/Makefile
skip_code_rules: vendor/** mobile/gradlew mobile/gradle/** desktop/build/** **/assets/**
short_words: i j x y z w h r v t _ a b c n f
```

```allowed
.editorconfig
.gitattributes
.githooks/pre-commit
.github/**
.gitignore
.prettierrc
AGENTS.md
CLAUDE.md
CONTRIBUTING.md
LICENSE
README.md
STRUCTURE.md
package.json
shell.nix
desktop/**
docs/**
locales/en.json
locales/pt_BR.json
locales/ru.json
meron-core/Cargo.*
meron-core/src/**
meron-core/.cargo/**
metadata/**
mobile/**
screenshots/**
scripts/**
snap/**
vendor/**
```
