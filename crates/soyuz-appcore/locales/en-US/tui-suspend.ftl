### Soyuz UI strings (en-US source): tui-suspend
### The terminal UI: suspend to a shell or a text editor (Phase T5 / T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-suspend.ftl.

# Printed in the terminal when a shell starts in place of the terminal UI.
tui-suspend-banner = Soyuz: type `exit` to return to the file list.
# Printed before a command line runs in place of the terminal UI ($command: the line).
tui-suspend-running = Soyuz: running { $command }
# Printed after that command ended.
tui-suspend-press-key = Press Enter to return to Soyuz…
# No shell configured ([terminal] shell) and none found.
tui-suspend-no-shell = No shell found: set [terminal] shell in the settings
# No console text editor found ($names: the commands tried, comma-separated).
tui-suspend-no-editor = No text editor found
tui-suspend-no-editor-tried = Tried { $names }. Set [tui] editor, or the VISUAL or EDITOR environment variable.
# A program could not be started ($program: its file name; the body is the system error).
tui-suspend-failed = Could not start { $program }
# A command or an editor ended with an error ($program: its file name, $code: the exit code).
tui-suspend-exit = { $program } ended with exit code { $code }
