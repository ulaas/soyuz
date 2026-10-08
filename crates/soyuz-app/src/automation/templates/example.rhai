// Soyuz script example. Scripts live in the "scripts" folder next to config.toml and
// reload when you save them. They run in a sandbox: they can read files through the
// API below, and every change they ask for becomes a job (shown for approval, undoable).
//
// Read:   list(dir) stat(path) exists(path) is_dir(path) read_text(path) search(dir, glob)
// Change: copy(src, dir) move_to(src, dir) rename(path, name) trash(paths) mkdir(path)
// UI:     navigate(path) select(paths) toast(text) copy_text(text) tag(paths, "blue")
// Helpers: file_name file_stem extension parent join_path now() format_size format_date

// A custom action: in the command palette (Ctrl+Shift+P), bindable to a key.
register_action(#{
    title: "Copy selected names",
    hotkey: "Ctrl+Alt+N",
    run: |ctx| {
        let text = "";
        for p in ctx.selected {
            if text != "" { text += "\n"; }
            text += file_name(p);
        }
        copy_text(text);
        toast(`${ctx.count} name(s) copied`);
    }
});

// A context-menu entry with a condition: only for two or more selected files.
register_menu_item(#{
    title: "Move into a new folder",
    contexts: ["file"],
    when: |ctx| ctx.count >= 2,
    run: |ctx| {
        let dest = join_path(ctx.dir, "Bundle " + format_date(now(), "%Y-%m-%d %H%M"));
        mkdir(dest);
        move_to(ctx.selected, dest);
    }
});

// A column provider: line count of text files (computed off the UI thread, cached).
register_column(#{
    title: "Lines",
    width: 70,
    extensions: ["txt", "md", "rs", "toml", "csv", "log"],
    value: |entry| {
        let text = read_text(entry.path, 262144);
        text.split("\n").len()
    }
});

// A previewer: CSV files as an aligned table (first 20 rows).
register_previewer(#{
    title: "CSV table",
    extensions: ["csv"],
    preview: |path| {
        let rows = [];
        for line in read_text(path, 65536).split("\n") {
            line.trim();
            if line != "" { rows.push(line.split(",")); }
            if rows.len() >= 20 { break; }
        }
        let widths = [];
        for r in rows {
            for i in 0..r.len() {
                let w = r[i].len();
                if i >= widths.len() { widths.push(w); } else if w > widths[i] { widths[i] = w; }
            }
        }
        let table = "";
        for r in rows {
            let line = "";
            for i in 0..r.len() {
                let cell = r[i];
                cell.pad(widths[i], ' ');
                line += cell + "   ";
            }
            table += line + "\n";
        }
        #{ markdown: `**${file_name(path)}** · ${rows.len()} rows` + "\n\n```\n" + table + "```\n" }
    }
});
