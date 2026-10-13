### Soyuz UI strings (en-US source): tui-ai
### The terminal UI: the AI assistant panel and MCP approval cards (Phase T5).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-ai.ftl.

## Key hints (bottom row of the panel while it has the keyboard)

tui-ai-hints-composer = Enter send · Alt+Enter / Ctrl+J new line · / commands · @ mention · Tab cards · Alt+M model · Alt+F access · Alt+O tools · Alt+S smart search · Esc back
tui-ai-hints-cards = A approve · R reject · K all of kind · E edit · Shift+E edit in { "$" }EDITOR · Enter details · U undo · S save · Tab compose · Esc back
tui-ai-hints-recipes = Enter run here · Del delete · F1-F3 tabs · Esc back
tui-ai-hints-audit = / filter · R reload · F1-F3 tabs · Esc back
tui-ai-hints-filter = Enter / Esc done
tui-ai-hints-smart = Enter search · Alt+Enter ask the assistant · Tab / Up / Down fields · Backspace removes a filter · Esc close
tui-ai-delete-again = Press Del again to delete this recipe

## Plan cards

# A button after "… 12 more": show every item of the plan.
tui-ai-show-all = Show all
# The card button that opens the name table of a rename plan.
tui-ai-edit-names = Edit names
# The card button that opens the JSON editor of the plan's arguments.
tui-ai-edit-json = Edit

## The name table of a rename plan (dialog)

tui-ai-names-title = Edit the new names
tui-ai-names-hints = Up/Down item · Enter use these names · Esc cancel

## Save as recipe (dialog)

tui-ai-recipe-name-empty = Enter a name for the recipe.
tui-ai-recipe-hints = Enter save · Esc cancel

## The JSON arguments of a plan (dialog)

# $tool: the tool's name (move, copy, …).
tui-ai-json-title = Edit the arguments · { $tool }
tui-ai-json-hints = Ctrl+S / Alt+S use these changes · Ctrl+E edit in { "$" }EDITOR · Tab buttons · Esc cancel
tui-ai-json-external = Edit in { "$" }EDITOR
# A toast: the arguments could not be written to (or read from) the scratch file.
tui-ai-editor-failed = Could not hand the arguments to the editor
