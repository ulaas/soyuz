// A rule script: add a rule step "Run script → tidy-downloads" in
// Settings → Automations. `rule(path, ctx)` runs for every matching item; changes it
// asks for become jobs (undoable), like the rule's own steps.

fn rule(path, ctx) {
    let ext = extension(path).to_lower();
    let kind = if ["jpg", "jpeg", "png", "gif", "webp", "heic"].contains(ext) {
        "Images"
    } else if ["pdf", "docx", "xlsx", "pptx", "txt", "md"].contains(ext) {
        "Documents"
    } else if ["zip", "7z", "rar", "tar", "gz"].contains(ext) {
        "Archives"
    } else {
        ()
    };
    if kind == () { return; }
    let dest = join_path(parent(path), kind);
    if !exists(dest) { mkdir(dest); }
    move_to(path, dest);
}
