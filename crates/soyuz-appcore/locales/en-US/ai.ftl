### Soyuz UI strings (en-US source): ai
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/ai.ftl.
### Only text the user sees lives here: prompts, tool descriptions and everything
### else sent to the model stay English in the code.

## Folder access (chips, menus, properties dialog)

ai-access-short-none = No access
ai-access-short-metadata = Metadata
ai-access-short-contents = Contents
# $access: one of the ai-access-short-* labels.
ai-access-inherited = { $access } (inherited)
ai-access-menu-caption = Assistant access for this folder
ai-access-menu-none = No access (hidden from the assistant)
ai-access-menu-metadata = Metadata only (names, sizes, dates)
ai-access-menu-contents = Contents allowed (may be sent to the model)
ai-access-inherit = Inherit from parent folders
# $name: folder name.
ai-access-tip = What the assistant may see in { $name }
ai-access-tip-inherited = What the assistant may see in { $name } (inherited)
ai-properties-access = AI access

## Context menu

ai-menu-assistant = Assistant
ai-menu-suggest-names = Suggest names
ai-menu-summarize = Summarize
ai-menu-find-related = Find related files
ai-menu-explain-folder = Explain this folder
ai-menu-organize = Organize…
ai-menu-smart-search = Smart search…

## Panel header and chips

ai-panel-title = Assistant
ai-status-recipe = Running a recipe…
ai-status-waiting = Waiting for your approval
# $secs: seconds since the task started.
ai-status-working = Working · { $secs }s
ai-status-idle = Plans every change; nothing happens without you
ai-status-external = { $count ->
    [one] { $count } external request waits for you
   *[other] { $count } external requests wait for you
}
ai-close = Close the assistant
# $key: keyboard shortcut, e.g. Ctrl+I.
ai-close-key = Close the assistant  ({ $key })
ai-settings = AI settings
ai-new-chat = New chat
ai-no-model = No model configured
ai-model-caption = Model for the next message
ai-model-auto = Automatic (fast / strong routing)
ai-manage-providers = Manage providers…
ai-route-fast = Fast
ai-route-strong = Strong
ai-route-tip = Model routing for typed messages: fast (naming, classifying) or strong (planning). Slash commands pick their own.
# $tokens: compact token count such as 12.4k.
ai-meter-tokens = { $tokens } tok
# $cost: US dollars, already formatted.
ai-meter-tokens-cost = { $tokens } tok · ${ $cost }
# $spent, $cap: US dollars, already formatted; $percent: share of the cap.
ai-meter-tip-capped =
    Tokens and cost of this task / session
    This month: ${ $spent } of ${ $cap } ({ $percent } %)
ai-meter-tip-month = This month: ${ $spent } (no cap)
ai-meter-tip = Tokens and cost (unpriced models show tokens only)
ai-tab-chat = Chat
ai-tab-recipes = Recipes
ai-tab-audit = Audit log
ai-stop-esc = Stop  Esc

## Chat

ai-empty-title = Ask about this folder
ai-empty-body = The assistant looks around with read tools and proposes every change as a plan you approve. Approved plans run as normal, undoable jobs.
ai-idea-organize = Organize this folder
ai-idea-explain = What is in here?
ai-idea-dedupe = Find duplicates
ai-idea-rename = Suggest better names for the selection
ai-thinking = Thinking
# $cost: US dollars, already formatted.
ai-confirm-cost = This task may cost about ${ $cost }
# $model: model id.
ai-confirm-body = Estimated for { $model } over a full run. You can stop it at any time with Esc.
ai-start = Start
ai-end-changes = { $count ->
    [one] { $count } change
   *[other] { $count } changes
}
# $tokens: compact token count such as 12.4k.
ai-end-tokens = { $tokens } tokens
# $cost: US dollars, already formatted.
ai-end-cost = ${ $cost }
ai-open-settings = Open AI settings
ai-undo-all = Undo everything the assistant did
ai-undo-all-tip = Undo every job of this task, newest first (history undo).
ai-undoing = Undoing…
ai-save-recipe = Save as recipe
ai-save-recipe-tip = Save these steps as a TOML recipe you can re-run later without a model (same preview and approval).
ai-recipe-name-hint = Recipe name
ai-saving = Saving…
ai-pending-plans = { $count ->
    [one] { $count } plan waits for your approval
   *[other] { $count } plans wait for your approval
}
ai-approve-all = Approve all
ai-mention-selection = { $count ->
    [one] selection ({ $count } item)
   *[other] selection ({ $count } items)
}
# $name: folder name.
ai-mention-folder = this folder: { $name }
ai-mention-tab = tab: { $name }
ai-composer-busy = The assistant is working…  (Esc stops it)
ai-composer-hint = Ask or give a task…   / commands · @ mention
ai-composer-keys = Enter to send · Shift+Enter new line
ai-stop = Stop
ai-send = Send
ai-smart-chip = Smart search
ai-drop-mention = { $count ->
    [one] Mention { $count } item
   *[other] Mention { $count } items
}
# Inserted into the composer; the user finishes the sentence.
ai-use-tool = Use the `{ $name }` tool to

## Slash commands (descriptions in the composer popup)

ai-slash-organize = Plan a tidy folder structure and move files into it
ai-slash-dedupe = Find duplicates and propose trashing the extra copies
ai-slash-rename = Suggest consistent names (one batch rename)
ai-slash-explain = Explain what this folder contains
ai-slash-find = Find files matching a description

## Tools menu

ai-tools = Tools
ai-tools-connected = Tools · { $count } connected
ai-tools-tip = Tools the assistant can use: Soyuz's own and those of connected MCP servers
# $risk: "read" | "write" | "destructive" (the assistant's risk ceiling).
ai-tools-builtin = { $count } built-in tools (files, search, media, jobs, UI) · up to { $risk ->
    [read] Read
    [write] Write
   *[destructive] Destructive
} risk
ai-tools-caption-connected = Connected tools (MCP servers)
ai-tools-none = No external MCP servers. Add one in Settings → AI → Connections.
ai-tools-count = { $count ->
    [one] { $count } tool
   *[other] { $count } tools
}
ai-tools-disabled = disabled
ai-tools-connecting = connecting…
ai-tools-not-connected = not connected
ai-risk-read = read
ai-risk-write = write
ai-risk-destructive = destructive
ai-tools-gate-ceiling = above the assistant's risk ceiling
ai-tools-gate-free = runs without asking
ai-tools-gate-ask = asks for approval
# $risk: one of ai-risk-*; $gate: one of ai-tools-gate-*.
ai-tools-risk-line = Risk: { $risk } · { $gate }
ai-manage-connections = Manage connections…

## Plan cards

ai-card-items = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
ai-card-entries = { $count ->
    [one] { $count } entry
   *[other] { $count } entries
}
ai-card-conflicts = { $count ->
    [one] { $count } conflict
   *[other] { $count } conflicts
}
ai-card-state-pending = Waiting for you
ai-card-state-auto = Auto-approved
ai-card-state-approved = Approved
ai-card-state-running = Running
ai-card-state-done = Done
ai-card-state-failed = Failed
ai-card-state-rejected = Rejected
ai-card-state-edited = Replaced by your edit
ai-card-state-preview = Preview only
ai-card-names-empty = names cannot be empty
ai-card-form-none = Keep at least one item
ai-card-form-dest-empty = Enter where the items go
ai-card-form-items = Items (unchecked ones are left out)
ai-card-form-dest = Destination
ai-card-form-json = Edit as JSON
ai-card-invalid-json = not valid JSON: { $error }
ai-card-external = External client
ai-card-client-tip = The MCP client that asked for this change. Its request is untrusted data.
# $transport: pipe, HTTP, stdio, …
ai-card-via = via { $transport }
# $time: minutes:seconds, e.g. 1:05.
ai-card-left = { $time } left
ai-card-timeout-tip = The request is refused when nobody answers in time (Settings → AI → Connections).
ai-card-destructive = Destructive: items leave this folder (trash is restorable; permanent delete is not).
ai-card-external-warning = Requested by an AI client outside Soyuz. Check the items: neither the request nor file names can approve anything by themselves.
ai-card-tainted = Proposed right after the assistant read file contents. Text inside files can try to steer it — check this plan before approving.
ai-card-blocked = Blocked by conflicts (see the items marked below). Edit or reject it.
ai-card-use-edit = Use these changes
ai-card-cancel-edit = Cancel edit
ai-card-approve = Approve
ai-card-blocked-tip = This plan has blocking conflicts
ai-card-edit = Edit
ai-card-reject = Reject
# $tool: tool name such as move.
ai-card-approve-kind = Approve all “{ $tool }”
ai-card-approve-kind-tip = Approve this plan and every later plan of the same kind in this chat (never for plans proposed right after reading file contents).
# $client: MCP client name.
ai-card-approve-kind-client = Approve all “{ $tool }” from { $client }
ai-card-approve-kind-client-tip = Approve this plan and every later “{ $tool }” request from { $client } until Soyuz restarts. Each still runs as an undoable job you can see here.
ai-card-edit-names-hint = Edit the new names, then use them to rebuild the plan.
ai-card-edit-json-hint = Edit the tool arguments (JSON). The plan is rebuilt with a fresh dry run.

## Plan previews (tree diff, rename table)

ai-diff-added = added
ai-diff-moved = moved
ai-diff-renamed = renamed
ai-diff-deleted = deleted
ai-diff-changed = changed
# $path: full path of the row.
ai-diff-reveal-tip =
    { $path }
    Click to show it in the pane
ai-diff-more = … { $count } more
ai-diff-before = Before
ai-diff-after = After
ai-diff-current-name = Current name
ai-diff-new-name = New name

## External requests (MCP approvals area)

ai-mcp-title = External requests
ai-mcp-clients = MCP clients
ai-mcp-waiting = { $count } waiting for you
ai-mcp-clear = Clear
ai-mcp-clear-tip = Hide finished requests
# $client: MCP client name.
ai-mcp-undo-client = Undo everything { $client } did
ai-mcp-undo-client-tip = Undo the { $count } operation(s) { $client } ran in this session, newest first (each as an undo job).
ai-mcp-timed-out = Timed out
ai-mcp-cancelled-by-client = Cancelled by the client
ai-mcp-job-gone = The job is gone from the activity list
ai-mcp-job-done = Done: { $ok } item(s), { $skipped } skipped
ai-mcp-job-errors = Finished with errors: { $ok } ok, { $failed } failed
ai-mcp-job-failed = Failed: { $error }
ai-mcp-job-cancelled = Cancelled
# Toast when an unknown MCP client connects. $name: client name.
ai-mcp-new-client = New MCP client “{ $name }”
# $transport: pipe, HTTP, stdio, …
ai-mcp-new-client-body = Connected over { $transport }; it is read-only until you approve it in Settings → AI assistant → Connections.
ai-mcp-pipe-unavailable = not available: { $error }
# $port: TCP port.
ai-mcp-http-failed = cannot listen on 127.0.0.1:{ $port }: { $error }

## Recipes page

ai-recipes-intro = Recipes replay saved steps in the current folder with fresh plans — no model, same approvals.
ai-recipes-empty-title = No recipes yet
ai-recipes-empty-body = After the assistant finished a task, choose “Save as recipe” to keep its steps.
ai-recipe-delete-tip = Delete this recipe
ai-recipe-run = Run here
ai-recipe-run-tip = Plan every step again in the current folder; each plan waits for your approval.
ai-recipe-unknown-tool = unknown tool “{ $tool }”
ai-recipe-read-tool = “{ $tool }” is a read tool; recipes only hold changes

## Audit log page

ai-audit-filter-hint = Filter text
ai-audit-source = Source
ai-audit-session = Session
ai-audit-model = Model
# $label: one of ai-audit-source / -session / -model.
ai-audit-chip-all = { $label }: all
ai-audit-chip = { $label }: { $value }
ai-audit-all = All
ai-audit-reload = Reload
# $path: the audit log file.
ai-audit-count = { $shown } of { $total } events · { $path }
# $id: job number.
ai-audit-job = job { $id }
ai-audit-badge-assistant = Assistant
ai-audit-badge-recipe = Recipe
ai-audit-source-assistant = Assistant
ai-audit-source-recipe = Assistant (recipe)

## Smart search

ai-smart-title = Smart search
ai-smart-hint = e.g. photos from last summer larger than 5 MB
ai-smart-empty = Filters appear here as you type.
ai-smart-query = Query
ai-smart-search = Search
ai-smart-ask = Ask the assistant
ai-smart-ask-tip = Let the model build the query and pick the best matches
ai-smart-remove-tip = Remove this filter
# Chat bubble for "Ask the assistant". $text: what the user typed.
ai-smart-ask-shown = Smart search: { $text }
ai-smart-unknown-kind = unknown kind “{ $kind }” (try image, video, audio, document, archive, code, folder)
ai-smart-bad-size = bad size “{ $value }” (e.g. >5MB)
ai-smart-bad-date = bad date “{ $value }” (e.g. 2026-06..2026-08)

## Chat bubbles, notices and toasts

ai-user-summarize-selection = Summarize the selected files
ai-user-summarize-folder = Summarize this folder
ai-user-find-related = Find files related to the selection
ai-user-suggest-names = { $count ->
    [one] Suggest names for { $count } file
   *[other] Suggest names for { $count } files
}
# $name: recipe name; $folder: path.
ai-user-run-recipe = Run recipe “{ $name }” in { $folder }
ai-notice-select-files = Select some files to suggest names for.
ai-notice-busy-previous = The assistant is still working on the previous task. Stop it first (Esc).
ai-notice-disabled = AI features are turned off in Settings → AI.
ai-notice-busy = The assistant is busy; stop it first.
ai-notice-cancelled = Cancelled before starting.
ai-notice-compacted = Summarised { $count } older tool result(s) to keep the conversation small.
# $profile: provider profile name; $percent: share used; $cap: US dollars, formatted.
ai-notice-budget = “{ $profile }” has used { $percent } % of its monthly budget (${ $cap }).
ai-notice-edit-invalid = The edited plan is invalid: { $error }
ai-notice-needs-approval = This plan came right after the assistant read file contents, so it needs your explicit approval.
ai-toast-open-folder = Open a folder to run the recipe in
# $file: recipe file name.
ai-saved-as = Saved as { $file }
ai-save-failed = Could not save: { $error }
ai-undo-done = Undid { $count } operation(s).
# $failed: number of failures; $errors: their messages.
ai-undo-partial = Undid { $count } operation(s); { $failed } could not be undone: { $errors }
# $client: MCP client name.
ai-undo-client-done = Undid { $count } operation(s) of { $client }.
ai-undo-client-partial = Undid { $count } operation(s) of { $client }; { $failed } could not be undone: { $errors }
# $id: history entry number.
ai-undo-entry-unavailable = history entry { $id } cannot be undone now
# $outcome: an ai-outcome-* text.
ai-undo-entry-failed = history entry { $id }: { $outcome }
# Shown when an error mentions the settings (offers an "Open AI settings" button).
ai-settings-word = Settings

## Task end

ai-finish-done = Done
ai-finish-cancelled = Stopped
ai-finish-step-limit = Stopped: step limit reached
ai-finish-token-budget = Stopped: token budget for this task used up
ai-finish-time-limit = Stopped: time limit reached
ai-finish-monthly-cap = Stopped: monthly budget cap reached
ai-finish-error = Error: { $error }
ai-error-disabled = AI features are turned off (Settings → AI).
ai-error-no-provider = No AI provider is configured yet. Add one in Settings → AI.
# $name: provider profile name.
ai-error-no-model-id = Profile “{ $name }” has no model id.

## Tool rows and plan results

ai-tool-attach-detail = { $texts } text excerpt(s), { $pics } picture(s)
ai-tool-attach-brief = { $count } file(s) looked at
ai-tool-read-limit = read-tool limit reached
ai-brief-entries = { $count } entries
ai-brief-matches = { $count } matches
ai-brief-groups = { $count } duplicate groups
ai-brief-items = { $count } items
ai-brief-folders = { $count } folders
ai-brief-tabs = { $count } tabs
ai-brief-lines = { $count } lines read
ai-brief-checksum = checksum computed
ai-brief-hidden = { $count } hidden by folder permissions
ai-brief-ok = ok
ai-brief-args-items = { $count } items
ai-exec-blocked = Blocked by conflicts
ai-exec-done = Done
ai-exec-server-error = The server reported an error
ai-outcome-done = done: { $ok } item(s), { $skipped } skipped
ai-outcome-errors = finished with errors: { $ok } ok, { $failed } failed
# $path, $error: the first failed item.
ai-outcome-errors-first = finished with errors: { $ok } ok, { $failed } failed (first: { $path }: { $error })
ai-outcome-failed = failed: { $error }
ai-outcome-cancelled = cancelled
ai-outcome-timed-out = still running (time limit reached)
ai-note-tags-added = added tags on { $count } item(s)
ai-note-tags-removed = removed tags on { $count } item(s)
# $id: history entry number.
ai-note-undo-started = undo of history entry { $id } started
ai-note-command-started = custom command “{ $name }” started
ai-note-nothing = nothing to do
# $step, $total: step numbers; $tool: tool name; $args: short argument summary.
ai-recipe-step = Step { $step }/{ $total }: { $tool } { $args }
ai-recipe-step-rejected = Step { $step } was rejected; the recipe stopped.
ai-recipe-step-failed = Step { $step } did not complete; the recipe stopped.
ai-recipe-step-error = step { $step } ({ $tool }): { $error }

## Live highlights in the file views

ai-touch-reading = The assistant looked at this
ai-touch-planned = In a plan waiting for your approval
ai-touch-changing = Being changed by an assistant job
ai-touch-done = Changed by the assistant
