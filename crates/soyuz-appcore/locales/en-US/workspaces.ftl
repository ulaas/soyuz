### Soyuz UI strings (en-US source): workspaces
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/workspaces.ftl.

## Save workspace dialog

workspaces-save-title = Save workspace
workspaces-save-intro = Saves this window's tabs, groups, pane layout and each pane's view under a name.
workspaces-save-name-hint = Workspace name
# Number of tabs in a window or saved workspace.
workspaces-tabs = { $count ->
    [one] { $count } tab
   *[other] { $count } tabs
}
# Tabs and tab groups of the window being saved.
workspaces-tabs-and-groups = { $tabs ->
        [one] { $tabs } tab
       *[other] { $tabs } tabs
    } · { $groups ->
        [one] { $groups } group
       *[other] { $groups } groups
    }

## Manage workspaces dialog

workspaces-manage-title = Workspaces
workspaces-manage-empty = No saved workspaces yet. Use “Save workspace as…” to keep this window's layout.
workspaces-manage-hotkey-hint = Hotkey, e.g. Ctrl+Alt+1
workspaces-manage-done = Done
workspaces-manage-edit = Edit
workspaces-manage-up = Up
workspaces-manage-up-tip = Move up (slot order)
workspaces-manage-new-window = New window
workspaces-manage-open = Open
workspaces-manage-slots-note = The first nine are also “open_workspace:1” … “:9” for key bindings.

## Close a window with several tabs

workspaces-close-title = { $count ->
    [one] Close a window with { $count } tab?
   *[other] Close a window with { $count } tabs?
}
workspaces-close-body-main = Move its tabs to the main window, or close them (Ctrl+Shift+T reopens closed tabs). Settings → Views → Windows & workspaces can remember the answer.
workspaces-close-body-next = Move its tabs to the next window, or close them (Ctrl+Shift+T reopens closed tabs). Settings → Views → Windows & workspaces can remember the answer.
workspaces-close-move = Move tabs
workspaces-close-close = Close tabs

## Tab history graph ("Where was I?")

workspaces-history-title = Where was I?
workspaces-history-intro = Every folder this tab visited. Branches keep the places back / forward would forget.
workspaces-history-empty = Nothing yet — browse a little first.

## Toasts

# $n: slot number 1–9.
workspaces-no-slot = No workspace { $n } yet
workspaces-no-slot-body = Save one with “Save workspace as…”.
workspaces-pinned-cant-group = Pinned tabs can't be grouped
workspaces-keep-one-tab = A window keeps at least one tab
workspaces-main-keeps-tab = The main window keeps at least one tab
workspaces-saved = Saved workspace “{ $name }”
workspaces-updated = Updated workspace “{ $name }”
workspaces-deleted = Deleted workspace “{ $name }”
workspaces-opened = Opened workspace “{ $name }”
workspaces-name-exists = A workspace with that name exists
# $chord: what the user typed as a hotkey.
workspaces-bad-chord = “{ $chord }” is not a key chord
workspaces-nothing-to-open = Workspace “{ $name }” has no folders to open
workspaces-no-other-windows = There are no other windows
workspaces-merged = { $count ->
    [one] Merged { $count } window
   *[other] Merged { $count } windows
}

## Command palette

workspaces-palette-title = Workspace: { $name }

## Tab context menu

workspaces-menu-add-new-group = Add tab to new group
workspaces-menu-add-to-group = Add tab to group
workspaces-unnamed-group = Unnamed group
workspaces-menu-remove-from-group = Remove from group
workspaces-menu-move-new-window = Move to new window
workspaces-menu-move-to-window = Move to window

## Group chip menu

workspaces-group-rename = Rename group
workspaces-group-expand = Expand group
workspaces-group-collapse = Collapse group
workspaces-group-new-tab = New tab in group
workspaces-group-ungroup = Ungroup
workspaces-group-to-new-window = Move group to new window
workspaces-group-close = Close group

## Group colors

workspaces-color-red = Red
workspaces-color-orange = Orange
workspaces-color-yellow = Yellow
workspaces-color-green = Green
workspaces-color-cyan = Cyan
workspaces-color-blue = Blue
workspaces-color-purple = Purple
workspaces-color-gray = Gray

## App menu

workspaces-menu = Workspaces
workspaces-menu-save-as = Save current as…
workspaces-menu-update = Update current workspace
workspaces-menu-manage = Manage workspaces…

## Sidebar

workspaces-timeline = Timeline
workspaces-timeline-tip = Files changed today and this week

## Pane layouts

workspaces-layout-single = Single pane
workspaces-layout-dual = Two panes, side by side
workspaces-layout-stacked = Two panes, stacked
workspaces-layout-quad = Four panes (2×2)

## Windows

# Window list entry: $title is the window's current folder, $count its other tabs.
workspaces-window-more-tabs = { $count ->
    [one] { $title } (+{ $count } tab)
   *[other] { $title } (+{ $count } tabs)
}
workspaces-main-window = Main window — { $title }
