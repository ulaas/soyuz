### Soyuz UI strings (en-US source): tui-settings-tables
### The terminal UI: editing lists of tables and secrets in the settings editor (Phase T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-settings-tables.ftl.

## The list of elements
tui-tables-title = Edit { $table }
tui-tables-empty = No entries yet: Insert or Alt+A adds one
tui-tables-entry = Entry { $n }
tui-tables-duplicate = Duplicate
tui-tables-hint = Enter edit · Insert add · Alt+D duplicate · Delete remove · Alt+↑/↓ move · Esc close
tui-tables-hint-map = Enter edit · Insert add · Alt+D duplicate · Delete remove · Esc close
tui-tables-remove-confirm = Remove “{ $name }”? Its settings are deleted from config.toml.
tui-tables-key = Name
tui-tables-key-body = Each entry of this table has a unique name.
tui-tables-key-hint = Enter add · Esc cancel
tui-tables-key-needed = Type a name
tui-tables-name-taken = “{ $name }” is already used in this list
tui-tables-editor-after = The changes to this list are applied now; press Ctrl+E again to edit config.toml

## The form of one element
tui-tables-row-title = { $table } · { $name }
tui-tables-row-hint = ↑/↓ field · Enter apply or edit · ←/→ change · Alt+R reset · Esc back
tui-tables-opaque = Edit in { "$" }EDITOR (Ctrl+E)
tui-tables-chord = Enter records the shortcut; Alt+R removes it
tui-tables-secret-ref = A reference only: credential:<name> or env:<VAR>. Alt+S stores the secret in the OS credential store.
tui-tables-set-secret = Set secret…

## Set secret…
tui-tables-secret-title = Secret for { $name }
tui-tables-secret-field = Secret
tui-tables-secret-body = Stored in the OS credential store as “{ $name }”. config.toml keeps only the reference credential:{ $name }, never the secret.
tui-tables-secret-hint = Enter store · Alt+R remove the stored secret · Esc cancel
tui-tables-secret-remove = Remove
tui-tables-secret-empty = Type the secret first
tui-tables-secret-stored = Secret stored in the credential store
tui-tables-secret-removed = Secret removed from the credential store
tui-tables-secret-failed = Credential store: { $error }
tui-tables-secret-hint-mcp = Enter store · Alt+G generate a token · Alt+R remove the stored token · Esc cancel
tui-tables-token-hint = Alt+T copy the token · Alt+C copy the client config · Esc close
