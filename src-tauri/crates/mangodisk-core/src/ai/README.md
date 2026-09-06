# Optional AI explanations

Explicit per-item requests explain built-in cleanup rules, privacy data kinds,
startup registrations, system settings, and maintenance actions.
It does not select items, change risk classifications, execute commands, or
authorize cleanup. Existing native preflight and confirmation remain authoritative.

## Boundaries

- The page maps its result to `AiContext`, an explicit metadata allowlist.
  Never add file contents, authentication secrets, process arguments, or arbitrary scan objects. Startup locations are explicitly included for software attribution.
  Custom cleanup rules are not yet supported by this adapter.
- Core validates configuration and context, builds prompts, enforces request
  limits, and decodes streaming responses. Model output is untrusted Markdown.
- Core stores the complete provider configuration as plain JSON in
  `data/ai.json` below the adapter-supplied application data directory. A private
  temporary file is atomically persisted over the previous document. The file
  contains the API key and must not be included in logs or feedback attachments.
  Unsupported schemas and invalid documents are rejected without rewriting.
  The unreleased credential-store prototype is not migrated automatically:
  re-enter that preview's settings once; existing OS credentials are untouched.
- Tauri owns request reservation, cancellation, IPC channels, and diagnostic logs.
- The frontend service owns IPC sessions. The AI store owns transient UI state
  and a bounded, memory-only result cache. Changing configuration clears the cache.
  Shared settings UI is store-independent and retrieves the saved key only while
  editing, masked by default with an explicit visibility toggle. The page shell
  hosts a non-modal panel inside content bounds, above its action footer.
  Clicking a configured item starts its explanation.
  Each module owns one panel and IPC session. Navigation and minimize preserve
  its stream, answer, reasoning and minimized state. Close cancels only that
  module; selecting another item in the same module replaces its previous request.
  The backend permits up to six independent reservations: five module streams
  plus a connection test. Cancellation and completion release only their own IDs.
  State is memory-only and does not survive application exit. Configuration changes
  cancel active requests and clear the shared cache, but retain displayed answers;
  only the initiating panel may restart, never hidden panels automatically.
- Each page owns a pure metadata projection. Version 2 of the transient context
  uses a tagged subject with domain-specific facts; older preview contexts are
  rejected. Configuration remains schema 1 and needs no migration.
  Startup explanations include original software names, descriptions, publishers,
  versions, signature status, executable paths and registration paths without
  redaction. Cleanup includes original source paths, source-level block reasons,
  running processes and scan capabilities. Optimization includes selection kind
  and diagnostic codes; `hasRecordedOriginalValue` describes existing saved history,
  not whether future changes can capture an original value. Maintenance includes
  the stable task ID and current status.
  These fields may identify users or installation locations and are
  sent only after an explicit explanation request to the configured provider.
  Groups are not silently truncated; malformed or oversized requests are rejected.
  Execution arguments, opaque operational IDs, privacy profiles and record details
  remain excluded. Model attribution is inference, not proof of ownership or safety.
- Core combines common explanation boundaries with a subject-specific prompt.
  The model must distinguish startup configuration from service runtime, draft
  settings from applied changes, and available maintenance from diagnosed faults.
  No prompt editor or model-driven operation capabilities are exposed.
- Page deactivation or unmounting does not cancel explanations. Native state
  changes dismiss only that module's panel; they do not generate paid requests.

## Provider contract

Configure a base URL ending at the API prefix (commonly `/v1`), a model, and a key.
Requests use `POST /chat/completions` with `stream: true`. Both HTTP and HTTPS
are accepted; HTTP does not encrypt the API key or request. Local loopback services
may omit a key. Redirects and automatic retries are disabled. The editor saves
the visible configuration explicitly; the key-free update API cannot silently
carry an old key to another destination.

The supported stream contains `choices[0].delta.content`, a `stop` finish reason,
and `[DONE]`. Truncated, oversized, empty, and unsuccessful responses are errors,
not cacheable answers. The optional disabled-reasoning mode sends provider-specific
extensions; use the default mode if a provider rejects them. Cancellation aborts
the local request but cannot guarantee that a provider stops billing immediately.
New configurations default to provider-managed reasoning for compatibility.
Existing saved reasoning preferences are preserved; disabling reasoning is opt-in.
Requests omit both `max_tokens` and `max_completion_tokens` in every reasoning
mode, including connection tests. The provider chooses its default completion
budget; MangoDisk does not impose an output token cap.
Length-limited output is a distinct error, never silently accepted or retried.
Wire traffic is bounded at 4 MiB to accommodate repeated per-token gateway
metadata. Visible text remains bounded at 32 KiB and individual SSE records at
64 KiB. Readable reasoning is independently bounded at 256 KiB.
The total request timeout is 180 seconds (connection timeout: 10 seconds)
to accommodate provider-default reasoning. These defensive byte limits and the timeout protect the client
from malformed or unbounded streams; they are not generation token budgets.

Streaming IPC separates `text` and `reasoning` deltas. Readable strings from
`delta.reasoning_content`, or the `delta.reasoning` alias, appear in a muted,
height-bounded, selectable section above the answer. Structured or encrypted
reasoning details are not rendered, and thinking is never inferred from answer
markup. The section collapses when the answer starts unless the user has taken
control by expanding, scrolling, or selecting text. Copy copies only the answer.
Reasoning stays in the bounded, memory-only result cache with its answer; it is
never logged, persisted, or sent back to the provider. Reasoning without a final
answer is still an empty-response error and is not cached as success.

Answers reuse the release-note Markdown renderer with compact foreground
typography and inert links. Every streaming update is sanitized through an
explicit formatting allowlist: no scripts, images, embedded documents, styles,
or event handlers. If sanitization is unsupported, Vue renders plain text and
logs a typed compatibility diagnostic. Reasoning stays plain text. Prompts ask
for a short conclusion, brief bullets, and selective bold emphasis, not headings
or tables. Unexpected code blocks and tables remain locally scrollable. Copy
preserves the answer's Markdown source; mouse selection copies visible text.
Sanitizer regression tests use jsdom because DOMPurify does not support happy-dom
as a security test environment. Production rendering still uses the native WebView.

Logs contain module tags, context schema versions, operation IDs, durations, HTTP status, typed failure reasons, and
token counts, text/reasoning byte counts, stream completion and request policy.
Stream summaries also cover cancellation and report the actual terminal error;
normal cancellation is an informational event, not a warning. HTTP error bodies
are read only for diagnostics, bounded at 8 KiB and two seconds. Known provider
codes and parameter names are mapped to internal enums, including SSE errors;
unknown values and free-form messages are discarded. Diagnostic read failures
preserve the original HTTP error. Configuration IO failures record the precise
stage, IO error kind and native error code, without raw error messages or paths.
They must not include keys, endpoints, prompts,
response bodies, or private scan data.

## Validation

`tests/fixtures/ai-context-v2.json` contains five synthetic module contexts shared
by frontend projection tests and Rust deserialization, prompt, and transport tests.
It is test-only contract evidence, not persisted settings or a production request
source. Keep the shared fixture so field/schema drift fails on both sides.

Run repository checks and Core tests on macOS and Windows. The ignored
`actual_provider_stream` test is an opt-in live request configured through
`ZENAI_AI_GATEWAY_API_KEY`, `MANGODISK_AI_TEST_ENDPOINT`, and
`MANGODISK_AI_TEST_MODEL`; it can incur provider charges. Select a synthetic
module fixture with `MANGODISK_AI_TEST_MODULE` (defaults to `cleanup`). Check the test source
for current environment variable names before running it.

Two ignored tests support reproducible multi-module evaluation:
- `capture_ai_evaluation_catalogs` reads native catalogs into an existing absolute
  `MANGODISK_AI_EVAL_DIRECTORY`, using isolated application state. It never cleans
  files or changes startup/system settings.
- `evaluate_ai_corpus` reads an explicit JSON array of `{id, context}` from
  `MANGODISK_AI_EVAL_INPUT` and writes answer/usage/timing records to a new
  `MANGODISK_AI_EVAL_OUTPUT` file. It uses the production transport and default
  reasoning, with two requests at a time and no automatic retries. The provider
  variables above are required. Validate costs and review every answer manually;
  HTTP success is not evidence of factual accuracy.
Keep catalogs, corpora and raw answers outside the repository and feedback logs:
they can contain private paths and installation details. Output files are never
overwritten, so completed evaluation evidence survives a later failure.

Configuration-file tests use isolated temporary directories and synthetic keys.
The frontend tests cover automatic generation, minimized streaming, cancellation,
rapid selection changes, retries, and cache reuse.
