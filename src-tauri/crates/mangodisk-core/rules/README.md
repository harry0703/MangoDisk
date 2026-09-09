# Contributing declarative cleanup rules

MangoDisk keeps ordinary cleanup behavior in validated TOML resources. Use this guide to add filesystem cleanup rules for application and system caches, or project artifact rules for rebuildable output inside source projects. If cleanup requires a system command, an application API, shared-resource coordination, or special high-impact confirmation, implement a dedicated cleaner instead of weakening the declarative safety model.

## Choose the correct rule type

- Use a **filesystem rule** for a known cache, log, crash report, temporary directory, or other disposable location owned by the operating system, an installed application, or a developer tool.
- Use a **project artifact rule** for rebuildable directories discovered inside source projects, such as `target`, `node_modules`, or framework build caches.
- Use a **dedicated cleaner** when deletion requires application-specific logic, system commands, elevated privileges, shared blob awareness, or validation immediately before execution.

## Directory layout

```text
rules/
├── filesystem/
│   ├── macos/
│   │   ├── system/
│   │   ├── browser/
│   │   ├── application/
│   │   ├── development/
│   │   ├── ai/
│   │   └── container/
│   └── windows/
│       └── ...
└── project-artifacts/
    ├── rust.toml
    ├── node.toml
    └── ...
```

Filesystem paths are enforced as `filesystem/<platform>/<category>/<rule-id>.toml`. Project artifact files conventionally use `project-artifacts/<ecosystem>.toml`. Empty categories do not need placeholder directories.

Before adding a rule, find the nearest existing example and verify the cleanup boundary against authoritative documentation or the application's current on-disk behavior. A rule must select only disposable or reproducible data. Do not target settings, credentials, databases, project source, personal files, user-managed models, or data whose ownership is uncertain.

## Filesystem rules

Filesystem rules use schema version 3. The build validates and embeds every TOML file, then the runtime parses the embedded source again with the same schema.

Start with this minimal example and replace every example value:

```toml
id = "dev.example-cache"
schema_version = 3
rule_version = 1
platform = "macos"
category = "development"
risk = "recoverable"
default_selected = false
recommended_selected = false
required_stopped_processes = []

[[applicability]]
kind = "anyOf"
items = [
  { kind = "executableAvailable", names = ["example"] },
  { kind = "anyRootExists" },
]

[[roots]]
template = "${user_library}/Caches/Example"
verified_rebuildable = true

[matcher]
kind = "all"

[execution]
kind = "deleteMatchingContents"
requires_app_close = false

[verification]
lifecycle = "verified"
evidence = "Example stores reproducible cache data in this directory; settings, credentials, projects, and user-created content remain outside the selected root"
verified_at = "2026-08-05"
verified_platform = "macos"
references = ["https://example.com/official-cache-documentation"]
```

See [`filesystem/macos/development/dev.pnpm-cache.toml`](filesystem/macos/development/dev.pnpm-cache.toml) for a real rule.

### Identity, selection, and risk

- `id` must use lowercase ASCII letters, digits, `.`, `-`, or `_`, and the file name must be `<id>.toml`.
- `rule_version` must be positive. Increment it when a change alters roots, matching, execution, risk, applicability, or verification semantics.
- `platform` is `macos` or `windows`.
- `category` is `system`, `browser`, `application`, `development`, `ai`, or `container`.
- `risk` is `safe`, `recoverable`, or `highImpact`.
- `default_selected = true` is allowed only for `safe` rules. `recommended_selected` controls the shared recommendation used by the desktop app and the CLI `recommended` selection. A `recoverable` rule may be recommended only when every root has `verified_rebuildable = true`.
- When `requires_app_close = true`, `required_stopped_processes` must contain the individual executable names that preflight must stop or reject. When application closure is not required, the list must be empty.

### Applicability

Every filesystem rule must contain at least one `[[applicability]]` probe. Applicability avoids unnecessary traversal; it never relaxes root or matcher safety. A known-inapplicable rule is skipped, while missing or incomplete inventory data keeps the rule eligible for scanning.

Supported probes are:

- `anyRootExists`
- `pathExists`
- `applicationInstalled` and `applicationVersion`
- `executableAvailable`
- `systemVersion`, `fileSystemIn`, and `capabilityAvailable`
- `processRunning`
- `anyOf`, `allOf`, and `not`

Multiple identifiers or names within one probe are aliases for the same fact and match when any alias succeeds. Use `allOf` when independent facts must all be true. Portable applications may not appear in the installation inventory, so combine identity evidence with a controlled path when appropriate:

```toml
[[applicability]]
kind = "anyOf"
items = [
  { kind = "applicationInstalled", identifiers = ["com.example.browser"] },
  { kind = "pathExists", template = "${user_library}/Caches/ExampleBrowser" },
]
```

Arbitrary absolute paths, environment variables, and command execution are not valid applicability inputs.

### Roots

Every root template must begin with one controlled, lowercase variable and use `/` separators. Supported variables are:

- All platforms: `${home}`, `${temp}`, `${system_root}`
- macOS: `${user_library}`, `${application_support}`, `${darwin_user_cache}`
- Windows: `${local_app_data}`, `${roaming_app_data}`, `${program_files}`, `${program_data}`

A static root needs only `template`. Use `kind = "childDirectories"` only when the rule must expand direct child directories through `child_names`, `child_prefixes`, `include_all_children`, or fixed `suffixes`. The validator rejects parent traversal, uncontrolled variables, duplicate roots, protected locations, unsafe expansion, and broad matching outside recognized cache or verified rebuildable boundaries.

### Matchers and execution

Supported matcher kinds are `all`, `nameEquals`, `nameGlob`, `extensionIn`, `pathSegmentIn`, `olderThan`, `largerThan`, `smallerThan`, `maxDepth`, `allOf`, `anyOf`, and `not`. Name matchers accept names, not paths; `nameGlob` does not support `**`. Numeric age, size, and depth values must be greater than zero.

Declarative filesystem rules support `deleteMatchingContents` and the opt-in `deleteWholeRoot` strategy. A broad `all` matcher is appropriate only when the root itself is a narrowly defined cache or a verified rebuildable location. Prefer a narrower matcher whenever the root contains mixed-purpose data.

`deleteWholeRoot` avoids one protected deletion transaction per file by atomically moving the verified root into a private same-volume staging directory and removing that tree. It is accepted only for static roots with `verified_rebuildable = true`, an exact `all` matcher, and `default_selected = false`. Runtime execution also requires the rule to own the complete root and a native aggregate to read every entry without links or permission skips. Source-scoped cleanup, nested rule ownership, unsupported native traversal, and any skipped entry automatically fall back to `deleteMatchingContents` before mutation.

### Verification

Production resources accept the `verified`, `stable`, and `deprecated` lifecycles. `candidate` and `disabled` rules are rejected from the production catalog. `evidence` must explain why the selected data is disposable and what important data remains outside the boundary. `verified_at` uses `YYYY-MM-DD`, and `verified_platform` must match the rule platform. Add authoritative HTTPS `references` whenever they are available.

Rule comments and verification evidence are developer-facing text and must use English so the automated source check can validate a consistent public rule catalog. Do not add UI names, descriptions, impact text, localization keys, or locale fields to TOML.

### Protected personal and high-value data

The home root, Downloads, Documents, Desktop, project directories, repositories, credentials, and cloud-synchronized folders are not ordinary cache roots. Do not combine them with `all`, broad name patterns, or composite matchers that can degrade into whole-root selection.

`system.stale-partial-downloads` is the only Downloads exception. It is accepted only when all of these conditions remain true:

- the rule ID is exactly `system.stale-partial-downloads`;
- the normalized root is exactly the current user's Downloads directory;
- `risk` is `recoverable` and `default_selected` is `false`;
- the matcher is `allOf`;
- `olderThan` is at least seven days;
- `extensionIn` contains only `crdownload`, `download`, `partial`, and `part`;
- `maxDepth` is no greater than three.

AI models are high-value downloadable data. Ordinary filesystem rules must not target project directories, Ollama model stores, LM Studio data, or user-managed model directories. The dedicated AI model cleaner validates official store layouts, shared blobs, links, and each model immediately before execution.

## Project artifact rules

Project artifact rules use schema version 1 and describe rebuildable output inside a recognized source project. They are always recoverable and opt-in.

Start with this minimal example:

```toml
id = "project.example-build-artifacts"
schema_version = 1
rule_version = 1
platforms = ["macos", "windows"]
category = "development"
risk = "recoverable"
default_selected = false

[match]
file_names_any = ["example-project.toml"]

[[artifacts]]
kind = "relativeDirectory"
path = "build"

[verification]
lifecycle = "verified"
evidence = ["https://example.com/official-build-directory-documentation"]
verified_at = "2026-08-05"
```

See [`project-artifacts/node.toml`](project-artifacts/node.toml) for a real rule.

Project artifact constraints are intentionally narrow:

- `id` must start with `project.` and use lowercase ASCII tokens separated by dots.
- `platforms` must contain one or both of `macos` and `windows`, without duplicates.
- `category`, `risk`, and `default_selected` must remain `development`, `recoverable`, and `false`.
- `[match]` must identify the project with at least one `file_names_any` or extension-like `file_suffixes_any` value. Use `relative_paths_all` and `relative_paths_any` only for additional project evidence.
- Every rule needs at least one artifact. `relativeDirectory` selects a normalized relative path. `descendantDirectory` selects one directory name below the project and requires `max_depth` from 1 through 64.
- Artifact paths must be normalized relative paths without absolute roots, `.` segments, or `..` traversal.
- Verification lifecycle must be `verified`, evidence must contain at least one authoritative HTTPS source, and `verified_at` must use `YYYY-MM-DD`.

### Codex worktree discovery

Regular deep-cleanup scans also inspect `${CODEX_HOME}/worktrees` (or
`~/.codex/worktrees` when the environment variable is unset). Discovery is bounded
to the `worktrees/ID/checkout` layout and requires matching reciprocal Git
pointers without linked path components. Optional `codex-thread.json` metadata
is not required inside this configured location. Outside it, Codex ownership
metadata remains necessary for Codex-specific classification. Identified
checkouts use the existing opt-in project
artifact rules; there is no separate scan mode or cleanup workflow. Explicit
project scopes are not expanded. Sessions, images, credentials, plugins, source
files, and entire worktrees are never cleanup targets.

Codex artifact sources use the existing per-source close requirement while
Codex or ChatGPT is running. Global Node, Python, Cargo, and other build processes
are not associated with a checkout and are neither blockers nor close targets.
Sources remain selectable. Confirmation uses the shared optional application
close panel, unchecked by default; only selected sources contribute to it.
Core resolves trusted project rule IDs to the fixed Codex/ChatGPT application
set, never to process names supplied by the frontend.
Execution rechecks checkout provenance and running processes before deletion;
unavailable process inventory fails closed for these artifacts, not other projects.
Execution reports known writers as `blocked` / `runningProcesses` (or `partial`
when other sources were cleaned). Changed Git associations and unavailable
process inspection report `preflightFailed`, with distinct diagnostic reasons.
For disposable real Git worktree deletion and log validation, run
`cargo test -p mangodisk-core codex_real_git_worktree_deletion_and_diagnostics -- --ignored --nocapture --test-threads=1`.
With Codex or ChatGPT running, this test checks the live app guard, then injects deterministic process
states for failure and idle scenarios; Git validation and deletion remain real.
It never targets existing user worktrees or terminates running applications.

## User-facing text

Rule execution data and UI presentation remain separate. Locale entries are optional at the schema level because MangoDisk can derive a readable fallback name from the rule ID. For a complete user-facing contribution, add `name`, `description`, and `impact` under `cleanupRules.entries.<rule-id>` in every supported file under [`src/locales`](../../../../src/locales). If you cannot provide a reliable translation, mention it in the pull request instead of adding unreviewed machine-translated text.

## Validation

Run the focused source and schema checks while developing:

```sh
pnpm check:rule-sources
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core declarative_schema
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core project_artifact_schema
```

Before opening a pull request, run the repository checks:

```sh
pnpm check
cargo test --manifest-path src-tauri/Cargo.toml -p mangodisk-core
```

The build recursively validates all TOML resources, including schema fields, platform variables, risk, lifecycle, matchers, root boundaries, applicability, process policy, and catalog overlap. Validation failures include the source file and the rejected boundary. Do not bypass a failure with a broader root, weaker matcher, or dedicated Rust branch for one rule ID.

## Pull request checklist

- The rule uses the correct filesystem or project artifact schema.
- Official documentation or reproducible evidence confirms the cleanup boundary.
- The rule excludes settings, credentials, source code, personal content, databases, and other durable data.
- IDs, paths, categories, platforms, risks, and versions follow the schema.
- Applicability and matchers are as narrow as practical.
- User-facing text is kept in locale resources rather than TOML.
- Focused and complete validation commands pass on every affected platform, or the pull request clearly states which platform could not be validated.
