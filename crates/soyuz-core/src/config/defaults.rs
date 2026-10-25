//! Default values for every config section and the built-in key presets.

use std::collections::BTreeMap;

use crate::columns::{Column, ViewMode};
use crate::size::SizeStyle;
use crate::time::DateFormat;

use super::actions::Action;
use super::keys::{KeyChord, KeyContext, KeyName, KeyPreset, Keymap, Mods};
use super::schema::*;

impl Default for Config {
    fn default() -> Self {
        Config {
            schema: SCHEMA_VERSION,
            general: General::default(),
            appearance: Appearance::default(),
            fonts: super::fonts::Fonts::default(),
            theme: ThemeSection::default(),
            layout: Layout::default(),
            workspace: WorkspaceConfig::default(),
            toolbar: Toolbar::default(),
            context_menu: ContextMenu::default(),
            sidebar: Sidebar::default(),
            tags: TagsSection::default(),
            columns: Columns::default(),
            behaviour: Behaviour::default(),
            open: OpenSection::default(),
            jobs: Jobs::default(),
            terminal: Terminal::default(),
            archive: Archive::default(),
            search: Search::default(),
            preview: Preview::default(),
            media: Media::default(),
            editor: Editor::default(),
            git: Git::default(),
            sync: SyncSection::default(),
            updates: Updates::default(),
            performance: Performance::default(),
            remote: RemoteSection::default(),
            linux: LinuxSection::default(),
            tui: TuiSection::default(),
            keys: Keys::default(),
            ai: super::ai::Ai::default(),
            bookmarks: Vec::new(),
            custom_commands: Vec::new(),
            scripting: Default::default(),
            automation: Default::default(),
            cli: Default::default(),
        }
    }
}

impl Default for TagsSection {
    fn default() -> Self {
        TagsSection { names: vec![String::new(); 8], sync_xdg_tags: true }
    }
}

impl Default for General {
    fn default() -> Self {
        General {
            start_location: "home".into(),
            open_folders_in: OpenFoldersIn::SameTab,
            single_click_open: false,
            confirm_delete: true,
            confirm_permanent_delete: true,
            show_hidden: false,
            honour_dot_hidden: true,
            show_system: false,
            show_extensions: true,
            natural_sort: true,
            case_insensitive_sort: true,
            folders_first: true,
            size_style: SizeStyle::Binary,
            date_format: DateFormat::default(),
            default_view: ViewMode::Details,
            double_click_speed_ms: 400,
            language: "auto".into(),
            strict_windows_names: false,
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: "system".into(),
            density: Density::Cozy,
            ui_scale: 1.0,
            font: String::new(),
            mono_font: String::new(),
            font_size: 13.0,
            reduced_motion: false,
            kinetic_scrolling: true,
            rubber_band: true,
            toast_time_scale: 0.5,
            high_contrast: false,
            icon_style: IconStyle::Colored,
            show_thumbnails: true,
            thumbnail_size: 96,
            list_end_space: 64,
            sidebar_width: 220.0,
            custom_titlebar: true,
            backdrop: Backdrop::None,
            snap_layouts: true,
            native_dialogs: true,
            fallback_fonts: FallbackFonts::Auto,
            color_emoji: true,
            color_theme: "orbit".into(),
            accent: "theme".into(),
            extension_badges: true,
            folder_thumbnails: true,
            video_thumbnails: true,
            pdf_thumbnails: true,
            program_icons: true,
            os_thumbnail_cache: true,
            write_os_thumbnails: true,
            icon_pack: String::new(),
            thumb_disk_cache_mib: 256,
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            sidebar: true,
            preview_pane: false,
            dual_pane: false,
            dual_pane_orientation: Orientation::Vertical,
            status_bar: true,
            tab_strip: TabStripMode::Always,
            terminal_height: 260.0,
            min_file_list_width: 320.0,
            activity_panel: ActivityPlacement::Right,
            focus_mode_hides: ["sidebar", "status_bar", "preview", "bottom_panel", "activity", "ai_panel", "tabs"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        WorkspaceConfig {
            new_window: NewWindowMode::InProcess,
            launch: LaunchMode::NewWindow,
            tear_off: true,
            tear_off_distance: 48.0,
            close_window_tabs: CloseWindowTabs::Ask,
            restore_windows: true,
            panels_follow_window: true,
            sync_relative: true,
            history_graph_limit: 200,
            timeline_days: 7,
            timeline_scan_folders: 40,
            timeline_max_items: 1000,
            timeline_use_index: true,
        }
    }
}

impl Default for Toolbar {
    fn default() -> Self {
        Toolbar {
            items: [
                "back",
                "forward",
                "up",
                "|",
                "breadcrumb",
                "spacer",
                "filter",
                "view_menu",
                "sort_menu",
                "toggle_dual_pane",
                "activity",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        use SidebarSection::*;
        Sidebar {
            sections: vec![Favorites, Devices, Portal, Tags, Trash, Network, Cloud, Smart],
            mode: SidebarMode::Places,
            tree_follow: true,
            tree_drag_expand_ms: 700,
            portal: Vec::new(),
            gtk_bookmarks: true,
            system_folders: true,
            system_folder_list: [
                "appdata",
                "local_appdata",
                "programdata",
                "temp",
                "xdg_config",
                "xdg_data",
                "xdg_cache",
            ]
            .map(String::from)
            .to_vec(),
            unmounted_devices: UnmountedDevices::All,
            hidden_devices: Vec::new(),
            disk_health: true,
        }
    }
}

impl Default for Columns {
    fn default() -> Self {
        let details = vec![Column::Name, Column::Size, Column::Kind, Column::Modified];
        let widths = details.iter().map(|c| (c.to_string(), c.default_width())).collect();
        Columns { details, widths }
    }
}

impl Default for OpenSection {
    fn default() -> Self {
        OpenSection {
            remember_choice: true,
            choose_when_unknown: true,
            run_binaries: true,
            store_apps_via_explorer: true,
            default_apps: Vec::new(),
            last_used: BTreeMap::new(),
        }
    }
}

impl Default for Behaviour {
    fn default() -> Self {
        Behaviour {
            rename_selects: RenameSelects::Stem,
            paste_conflict: PasteConflict::Ask,
            paste_images: true,
            clipboard_history: 10,
            drag_default: DragDefault::Auto,
            remember_folder_views: true,
            restore_session: true,
            auto_refresh: true,
            watch_debounce_ms: 250,
            watch_poll_interval_ms: 2000,
            type_ahead: TypeAhead::Jump,
            middle_click: MiddleClick::NewTab,
            spring_loaded_folders: true,
            spring_load_delay_ms: 700,
            drag_out: true,
        }
    }
}

impl Default for Jobs {
    fn default() -> Self {
        Jobs {
            max_parallel: 4,
            per_device_serial: true,
            reset_apply_all_between_jobs: true,
            verify_after_copy: false,
            copy_buffer_mib: 8,
            unbuffered_threshold_mib: 256,
            preserve: vec![PreserveItem::Times, PreserveItem::Attrs, PreserveItem::Xattr],
            reflink: ReflinkPref::Auto,
            copy_duplicates: DuplicatesPref::Clone,
            history_limit: 500,
            trash_unsupported: TrashUnsupportedPolicy::Ask,
            auto_open_activity: true,
            io_stats: true,
        }
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Terminal {
            shell: String::new(),
            args: Vec::new(),
            follow_navigation: true,
            cd_on_tab_switch: true,
            shell_integration: true,
            escape_returns_focus: true,
            scrollback: 10_000,
            font_size: 13.0,
            theme: "auto".into(),
            per_tab: false,
            follow_shell: false,
            osc52: Osc52Policy::Ask,
            close_window_with_sessions: CloseWindowSessions::Ask,
        }
    }
}

impl Default for Archive {
    fn default() -> Self {
        Archive {
            preserve_special_bits: false,
            bomb_ratio: 100,
            bomb_floor_mib: 1024,
            max_entries: 1_000_000,
            max_entry_mib: 32 * 1024,
            edit_level: 6,
            nested_max_mib: 2048,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Search {
            threads: 0,
            respect_gitignore: false,
            content_max_mib: 8,
            document_text: true,
            saved: Vec::new(),
            index: SearchIndex::default(),
        }
    }
}

impl Default for SearchIndex {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        SearchIndex {
            enabled: false,
            roots: s(&["~"]),
            exclude: s(&[
                "node_modules",
                ".git",
                ".hg",
                ".svn",
                "target",
                "__pycache__",
                ".cache",
                "$RECYCLE.BIN",
                "System Volume Information",
                "AppData/Local/Temp",
                "AppData/Local/Packages",
                "AppData/Local/Microsoft/Windows/INetCache",
                "AppData/Local/**/Cache",
                "AppData/Local/**/Code Cache",
                "AppData/Local/**/GPUCache",
                "AppData/Local/CrashDumps",
                "AppData/Roaming/**/Cache",
                "Library/Caches",
            ]),
            linux_exclude: s(&[
                ".local/share/Trash",
                ".var/app/*/cache",
                ".local/share/containers",
                ".local/share/Steam",
                "~/snap",
                "/proc",
                "/sys",
                "/dev",
                "/run",
            ]),
            content: false,
            content_max_kib: 1024,
            content_extensions: s(&[
                "txt", "md", "markdown", "rst", "org", "log", "csv", "json", "toml", "yaml", "yml", "ini", "cfg",
                "xml", "html", "css", "js", "ts", "tsx", "jsx", "rs", "py", "go", "java", "kt", "c", "h", "cpp", "hpp",
                "cs", "rb", "php", "swift", "sh", "ps1", "bat", "sql", "lua", "tex",
            ]),
            usn_journal: true,
            throttle_ms: 2,
            smart_refresh_secs: 2,
            use_for_search: true,
            rescan_interval_secs: 600,
        }
    }
}

impl Default for LinuxSection {
    fn default() -> Self {
        LinuxSection { window_backend: WindowBackend::Auto, own_file_manager1: false, clipboard_fallback_tools: false }
    }
}

impl Default for Preview {
    fn default() -> Self {
        Preview {
            max_text_kib: 512,
            max_image_mp: 40,
            syntax_theme: "auto".into(),
            pdf: true,
            thumb_cache_mib: 64,
            thumb_idle_trim_secs: 60,
            preview_cache_mib: 96,
            pdfium_dir: String::new(),
            office: true,
            epub: true,
            fonts: true,
            models: true,
            model_turntable: true,
            audio_waveform: true,
            audio_playback: true,
            video: true,
            diff: true,
            max_document_mib: 64,
            sheet_rows: 200,
            max_model_mib: 128,
            max_audio_mib: 512,
            max_diff_kib: 1024,
            folder_notes: true,
            folder_notes_files: ["README.md", "README.markdown", "README.txt", "README", "NOTES.md", ".notes.md"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl Default for Media {
    fn default() -> Self {
        Media {
            ffmpeg: true,
            ffmpeg_dir: String::new(),
            threads: 0,
            hw_decode: HwDecode::Auto,
            gpu_yuv: false,
            audio_backend: AudioBackend::Auto,
            player: true,
            autoplay: false,
            quicklook_space_plays: false,
            loop_: false,
            volume: 80,
            muted: false,
            seek_step_secs: 5.0,
            max_px: 1280,
            tui_fps: 8,
            poster_secs: 0.0,
            tools: true,
            faststart: true,
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            dock: EditorDock::Right,
            font_size: 13.0,
            tab_width: 4,
            wrap: false,
            show_whitespace: false,
            max_text_mib: 20,
            backup_to_trash: true,
            jpeg_quality: 92,
            max_image_mp: 200,
        }
    }
}

impl Default for Git {
    fn default() -> Self {
        Git { status_column: true, refresh_secs: 5 }
    }
}

impl Default for SyncSection {
    fn default() -> Self {
        SyncSection {
            scheduler: true,
            reports_kept: 20,
            change_debounce_secs: 10,
            scheduled_dry_run: false,
            previous_versions: true,
            shadow_probe_max: 64,
            treemap_diff_content: false,
        }
    }
}

impl Default for RemoteSection {
    fn default() -> Self {
        RemoteSection {
            cache_ttl_secs: 30,
            poll_secs: 0,
            connect_timeout_secs: 20,
            io_timeout_secs: 60,
            preview_cache_mb: 512,
            preview_max_mb: 64,
            share_enumeration: true,
            discover: true,
            share_minutes: 30,
            share_port: 0,
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Keys { preset: KeyPreset::Explorer, cmd_is_ctrl: true, bindings: BTreeMap::new(), marks: false }
    }
}

// ---------------------------------------------------------------------------
// Key presets
// ---------------------------------------------------------------------------

const NONE: Mods = Mods::empty();
const CTRL: Mods = Mods::CTRL;
const SHIFT: Mods = Mods::SHIFT;
const ALT: Mods = Mods::ALT;
const CTRL_SHIFT: Mods = Mods::CTRL.union(Mods::SHIFT);
const CTRL_ALT: Mods = Mods::CTRL.union(Mods::ALT);
const ALT_SHIFT: Mods = Mods::ALT.union(Mods::SHIFT);

const DIGITS: [KeyName; 9] = [
    KeyName::Num1,
    KeyName::Num2,
    KeyName::Num3,
    KeyName::Num4,
    KeyName::Num5,
    KeyName::Num6,
    KeyName::Num7,
    KeyName::Num8,
    KeyName::Num9,
];

/// Bindings shared by both presets.
fn common(km: &mut Keymap) {
    use Action as A;
    use KeyContext::{Editor as E, FileList as L, Global as G, Player as P};
    let mut b = |ctx, action, mods, key| km.bind(ctx, action, KeyChord::new(mods, key));

    // Global
    b(G, A::CommandPalette, CTRL_SHIFT, KeyName::P);
    b(G, A::CommandPalette, CTRL, KeyName::K);
    b(G, A::JumpToFolderDialog, CTRL, KeyName::P);
    for (i, k) in DIGITS.iter().enumerate() {
        b(G, A::OpenBookmark(i as u8 + 1), ALT, *k);
    }
    b(G, A::NewTab, CTRL, KeyName::T);
    b(G, A::CloseTab, CTRL, KeyName::W);
    b(G, A::ReopenClosedTab, CTRL_SHIFT, KeyName::T);
    b(G, A::NextTab, CTRL, KeyName::Tab);
    b(G, A::PrevTab, CTRL_SHIFT, KeyName::Tab);
    for (i, k) in DIGITS.iter().enumerate() {
        b(G, A::SelectTab(i as u8 + 1), CTRL, *k);
    }
    b(G, A::NewWindow, CTRL, KeyName::N);
    b(G, A::Workspaces, CTRL_ALT, KeyName::W);
    b(G, A::ToggleQuadPane, SHIFT, KeyName::F9);
    b(G, A::ToggleTerminal, CTRL, KeyName::Backtick);
    b(G, A::FocusAddressBar, CTRL, KeyName::L);
    b(G, A::Back, ALT, KeyName::ArrowLeft);
    b(G, A::Forward, ALT, KeyName::ArrowRight);
    b(G, A::Up, ALT, KeyName::ArrowUp);
    b(G, A::Refresh, CTRL, KeyName::R);
    b(G, A::Filter, CTRL, KeyName::F);
    b(G, A::Search, CTRL_SHIFT, KeyName::F);
    b(G, A::Settings, CTRL, KeyName::Comma);
    b(G, A::Quit, CTRL, KeyName::Q);
    b(G, A::ToggleHidden, CTRL, KeyName::H);
    b(G, A::ToggleSidebar, CTRL, KeyName::B);
    b(G, A::ToggleFocusMode, CTRL_SHIFT, KeyName::B);
    b(G, A::ToggleTreeSidebar, CTRL_SHIFT, KeyName::E);
    b(G, A::TogglePreviewPane, ALT, KeyName::P);
    b(G, A::ToggleActivityPanel, CTRL_SHIFT, KeyName::J);
    b(G, A::ToggleAiPanel, CTRL, KeyName::I);
    b(G, A::Undo, CTRL, KeyName::Z);
    b(G, A::Redo, CTRL, KeyName::Y);
    b(G, A::Redo, CTRL_SHIFT, KeyName::Z);
    // Ctrl +/-/0 scale the whole UI; Ctrl+Alt +/-/0 (and Ctrl+wheel) size icons.
    b(G, A::UiZoomIn, CTRL, KeyName::Plus);
    b(G, A::UiZoomIn, CTRL, KeyName::Equals);
    b(G, A::UiZoomOut, CTRL, KeyName::Minus);
    b(G, A::UiZoomReset, CTRL, KeyName::Num0);
    b(G, A::ZoomIn, CTRL_ALT, KeyName::Plus);
    b(G, A::ZoomIn, CTRL_ALT, KeyName::Equals);
    b(G, A::ZoomOut, CTRL_ALT, KeyName::Minus);
    b(G, A::ZoomReset, CTRL_ALT, KeyName::Num0);
    b(G, A::ViewDetails, CTRL_SHIFT, KeyName::Num1);
    b(G, A::ViewGrid, CTRL_SHIFT, KeyName::Num2);
    b(G, A::ViewList, CTRL_SHIFT, KeyName::Num3);
    b(G, A::ViewMiller, CTRL_SHIFT, KeyName::Num4);
    b(G, A::ViewGallery, CTRL_SHIFT, KeyName::Num5);
    b(G, A::ShowKeybindings, CTRL, KeyName::Slash);
    // The context menu key of Windows / GTK; the terminal UI opens its menu with it.
    b(G, A::ContextMenu, SHIFT, KeyName::F10);
    // Scroll the preview pane without leaving the file list (terminal UI).
    b(G, A::PreviewScrollUp, ALT_SHIFT, KeyName::ArrowUp);
    b(G, A::PreviewScrollDown, ALT_SHIFT, KeyName::ArrowDown);
    b(G, A::PreviewPageUp, ALT, KeyName::PageUp);
    b(G, A::PreviewPageDown, ALT, KeyName::PageDown);

    // File list
    b(L, A::Copy, CTRL, KeyName::C);
    b(L, A::Cut, CTRL, KeyName::X);
    b(L, A::Paste, CTRL, KeyName::V);
    b(L, A::PasteAsLink, CTRL_SHIFT, KeyName::V);
    b(L, A::ClipboardHistory, CTRL_ALT, KeyName::V);
    b(L, A::SelectAll, CTRL, KeyName::A);
    b(L, A::InvertSelection, CTRL_SHIFT, KeyName::I);
    b(L, A::SelectNone, NONE, KeyName::Escape);
    b(L, A::Open, NONE, KeyName::Enter);
    b(L, A::Properties, ALT, KeyName::Enter);
    // Enter opens with the default app, Shift+Enter picks one (Open with…).
    b(L, A::OpenWith, SHIFT, KeyName::Enter);
    b(L, A::BatchRename, CTRL_SHIFT, KeyName::R);
    b(L, A::Delete, NONE, KeyName::Delete);
    b(L, A::DeletePermanent, SHIFT, KeyName::Delete);
    b(L, A::NewFolder, CTRL_SHIFT, KeyName::N);
    b(L, A::NewFile, CTRL_ALT, KeyName::N);
    b(L, A::Duplicate, CTRL, KeyName::D);
    b(L, A::CopyPath, CTRL_SHIFT, KeyName::C);
    b(L, A::CursorUp, NONE, KeyName::ArrowUp);
    b(L, A::CursorDown, NONE, KeyName::ArrowDown);
    b(L, A::CursorPageUp, NONE, KeyName::PageUp);
    b(L, A::CursorPageDown, NONE, KeyName::PageDown);
    b(L, A::CursorHome, NONE, KeyName::Home);
    b(L, A::CursorEnd, NONE, KeyName::End);
    b(L, A::ExtendUp, SHIFT, KeyName::ArrowUp);
    b(L, A::ExtendDown, SHIFT, KeyName::ArrowDown);
    b(L, A::OpenInNewTab, CTRL, KeyName::Enter);
    // "Edit in Soyuz" (Total Commander's F4 = edit).
    b(L, A::OpenInEditor, NONE, KeyName::F4);

    // The built-in editors while a document has the keyboard (they shadow the global
    // chords there). The terminal UI adds its own (`soyuz-tui` overlay: Ctrl+R replace,
    // save as, wrap, maximize, …).
    b(E, A::EditorSave, CTRL, KeyName::S);
    b(E, A::EditorFind, CTRL, KeyName::F);
    b(E, A::EditorReplace, CTRL, KeyName::H);
    b(E, A::EditorGoTo, CTRL, KeyName::G);
    b(E, A::EditorFindNext, NONE, KeyName::F3);
    b(E, A::EditorFindPrev, SHIFT, KeyName::F3);
    b(E, A::EditorCloseDocument, CTRL, KeyName::W);
    b(E, A::EditorNextDocument, CTRL, KeyName::PageDown);
    b(E, A::EditorPrevDocument, CTRL, KeyName::PageUp);

    // The media player while it has the keyboard (preview pane or Quick Look; it shadows
    // the global and list chords there). P plays / pauses (shown in hints); K / J / L as in
    // most video players.
    b(P, A::PlayerPlayPause, NONE, KeyName::P);
    b(P, A::PlayerPlayPause, NONE, KeyName::K);
    b(P, A::PlayerSeekBack, NONE, KeyName::J);
    b(P, A::PlayerSeekForward, NONE, KeyName::L);
    b(P, A::PlayerSeekBack, NONE, KeyName::ArrowLeft);
    b(P, A::PlayerSeekForward, NONE, KeyName::ArrowRight);
    b(P, A::PlayerVolumeDown, NONE, KeyName::Minus);
    b(P, A::PlayerVolumeUp, NONE, KeyName::Equals);
    b(P, A::PlayerMute, NONE, KeyName::M);
}

/// Build the key map for a preset (before `[keys.bindings]` overrides).
pub fn preset(p: KeyPreset) -> Keymap {
    use Action as A;
    use KeyContext::{FileList as L, Global as G};
    let mut km = Keymap::default();
    common(&mut km);
    let mut b = |ctx, action, mods, key| km.bind(ctx, action, KeyChord::new(mods, key));
    match p {
        KeyPreset::Explorer => {
            b(L, A::Rename, NONE, KeyName::F2);
            b(G, A::Search, NONE, KeyName::F3);
            b(G, A::Refresh, NONE, KeyName::F5);
            // In the file list F5/F6 copy/move to the other pane; without a second pane
            // the app falls back to Refresh for F5.
            b(L, A::CopyToOtherPane, NONE, KeyName::F5);
            b(L, A::MoveToOtherPane, NONE, KeyName::F6);
            b(L, A::QuickLook, NONE, KeyName::Space);
            b(L, A::Up, NONE, KeyName::Backspace);
            b(G, A::ToggleDualPane, NONE, KeyName::F9);
        }
        KeyPreset::Commander => {
            b(L, A::QuickLook, NONE, KeyName::F3);
            b(L, A::CopyToOtherPane, NONE, KeyName::F5);
            b(L, A::MoveToOtherPane, NONE, KeyName::F6);
            b(L, A::NewFolder, NONE, KeyName::F7);
            b(L, A::Delete, NONE, KeyName::F8);
            b(L, A::FocusOtherPane, NONE, KeyName::Tab);
            b(L, A::MarkAndMoveDown, NONE, KeyName::Insert);
            b(L, A::MarkToggle, NONE, KeyName::Space);
            b(L, A::Filter, CTRL, KeyName::S);
            b(G, A::Search, ALT, KeyName::F7);
            b(L, A::Rename, SHIFT, KeyName::F6);
            b(G, A::SwapPanes, CTRL, KeyName::U);
            b(L, A::Up, NONE, KeyName::Backspace);
        }
        KeyPreset::Finder => {
            // Finder: Enter renames, Cmd+Down opens, Cmd+Up goes to the parent,
            // Cmd+Backspace trashes, Cmd+I shows info, Cmd+1..4 switch views.
            let mut unbind = |ctx: KeyContext, a: Action| {
                km.bindings.remove(&(ctx, a));
            };
            unbind(L, A::Open);
            unbind(L, A::InvertSelection);
            for i in 1..=9 {
                unbind(G, A::SelectTab(i));
            }
            for a in [A::ViewDetails, A::ViewGrid, A::ViewList, A::ViewMiller, A::ViewGallery] {
                unbind(G, a);
            }
            let mut b = |ctx, action, mods, key| km.bind(ctx, action, KeyChord::new(mods, key));
            b(L, A::Rename, NONE, KeyName::Enter);
            b(L, A::Open, CTRL, KeyName::ArrowDown);
            b(L, A::Open, CTRL, KeyName::O);
            b(G, A::Up, CTRL, KeyName::ArrowUp);
            b(L, A::Delete, CTRL, KeyName::Backspace);
            b(L, A::Properties, CTRL, KeyName::I);
            b(L, A::InvertSelection, CTRL_SHIFT, KeyName::I);
            b(L, A::QuickLook, NONE, KeyName::Space);
            b(G, A::Back, CTRL, KeyName::OpenBracket);
            b(G, A::Forward, CTRL, KeyName::CloseBracket);
            b(G, A::ViewGrid, CTRL, KeyName::Num1);
            b(G, A::ViewDetails, CTRL, KeyName::Num2);
            b(G, A::ViewMiller, CTRL, KeyName::Num3);
            b(G, A::ViewGallery, CTRL, KeyName::Num4);
            b(G, A::ViewList, CTRL, KeyName::Num5);
            b(G, A::GoTo, CTRL_SHIFT, KeyName::G);
            b(G, A::Home, CTRL_SHIFT, KeyName::H);
            b(G, A::ToggleHidden, CTRL_SHIFT, KeyName::Period);
            b(L, A::CopyPath, CTRL_ALT, KeyName::C);
            b(G, A::ToggleDualPane, NONE, KeyName::F9);
        }
        KeyPreset::Vim => {
            let mut b = |ctx, action, mods, key| km.bind(ctx, action, KeyChord::new(mods, key));
            b(L, A::CursorDown, NONE, KeyName::J);
            b(L, A::CursorUp, NONE, KeyName::K);
            b(L, A::Up, NONE, KeyName::H);
            b(L, A::Up, NONE, KeyName::Backspace);
            b(L, A::Open, NONE, KeyName::L);
            b(L, A::CursorEnd, SHIFT, KeyName::G);
            b(L, A::ExtendDown, SHIFT, KeyName::J);
            b(L, A::ExtendUp, SHIFT, KeyName::K);
            b(L, A::Back, SHIFT, KeyName::H);
            b(L, A::Forward, SHIFT, KeyName::L);
            b(L, A::Filter, NONE, KeyName::Slash);
            b(L, A::MarkAndMoveDown, NONE, KeyName::Space);
            b(L, A::InvertSelection, NONE, KeyName::V);
            b(L, A::QuickLook, NONE, KeyName::I);
            b(L, A::Undo, NONE, KeyName::U);
            b(L, A::Rename, NONE, KeyName::F2);
            b(L, A::CommandPalette, SHIFT, KeyName::Semicolon);
            b(G, A::ToggleDualPane, NONE, KeyName::F9);
            let seq = |a: KeyName, b: KeyName| vec![KeyChord::new(NONE, a), KeyChord::new(NONE, b)];
            let mut s = |action, sq| km.bind_sequence(L, action, sq);
            s(A::CursorHome, seq(KeyName::G, KeyName::G));
            s(A::Home, seq(KeyName::G, KeyName::H));
            s(A::NextTab, seq(KeyName::G, KeyName::T));
            s(A::PrevTab, vec![KeyChord::new(NONE, KeyName::G), KeyChord::new(SHIFT, KeyName::T)]);
            s(A::Copy, seq(KeyName::Y, KeyName::Y));
            s(A::CopyPath, seq(KeyName::Y, KeyName::P));
            s(A::CopyName, seq(KeyName::Y, KeyName::N));
            s(A::Cut, seq(KeyName::D, KeyName::D));
            s(A::Delete, seq(KeyName::D, KeyName::Delete));
            s(A::Paste, seq(KeyName::P, KeyName::P));
            s(A::PasteAsLink, seq(KeyName::P, KeyName::L));
            s(A::Rename, seq(KeyName::C, KeyName::W));
            s(A::ToggleHidden, seq(KeyName::Z, KeyName::H));
            s(A::NewFolder, seq(KeyName::G, KeyName::N));
        }
    }
    km
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_no_conflicts() {
        for p in KeyPreset::ALL {
            let km = preset(p);
            assert!(km.conflicts().is_empty(), "{p:?}: {:?}", km.conflicts());
            assert!(km.prefix_conflicts().is_empty(), "{p:?}: {:?}", km.prefix_conflicts());
            assert!(km.bindings.len() > 40);
        }
    }

    #[test]
    fn default_apps_by_extension() {
        let vlc = DefaultApp { name: "VLC".into(), program: "/usr/bin/vlc".into(), ..Default::default() };
        let mpv = DefaultApp { name: "mpv".into(), program: "/usr/bin/mpv".into(), ..Default::default() };
        let mut o = OpenSection::default();
        assert!(o.remember_choice && o.app_for("a.mp4").is_none());
        for p in KeyPreset::ALL {
            let chord = KeyChord::new(Mods::SHIFT, KeyName::Enter);
            assert_eq!(preset(p).lookup(KeyContext::FileList, chord), Some(&Action::OpenWith), "{p:?}");
        }
        o.set_default("MP4", Some(vlc.clone()));
        o.set_default(".mkv", Some(vlc.clone()));
        assert_eq!(o.default_apps.len(), 1, "one rule per application");
        assert_eq!(o.default_apps[0].extensions, ["mp4", "mkv"]);
        assert_eq!(o.app_for("Clip.MKV").map(|a| a.name.as_str()), Some("VLC"));
        assert!(o.app_for("mp4").is_none() && o.app_for(".mp4").is_none(), "a bare extension is not a match");
        o.set_default("mkv", Some(mpv));
        assert_eq!(o.app_for("x.mkv").map(|a| a.name.as_str()), Some("mpv"));
        assert_eq!(o.app_for("x.mp4").map(|a| a.name.as_str()), Some("VLC"));
        o.set_default("mp4", None);
        assert!(o.app_for("x.mp4").is_none());
        assert_eq!(o.default_apps.len(), 1, "a rule without extensions goes");
        let gz = DefaultApp { extensions: vec!["tar.gz".into()], ..vlc };
        assert!(gz.matches("a.TAR.GZ") && !gz.matches("a.gz"));
        assert_eq!(OpenSection::ext_of("a.b.TXT").as_deref(), Some("txt"));
        assert_eq!(OpenSection::ext_of(".bashrc"), None);
        assert_eq!(OpenSection::ext_of("README"), None);

        // Files without an extension: the "." key.
        assert_eq!(OpenSection::key_of("README"), ".");
        assert_eq!(OpenSection::key_of(".bashrc"), ".");
        assert_eq!(OpenSection::key_of("a.Rs"), "rs");
        assert!(o.choose_when_unknown);
        assert!(o.run_binaries);
        assert!(o.store_apps_via_explorer);
        let edit = DefaultApp { name: "Editor".into(), program: "/usr/bin/ed".into(), ..Default::default() };
        o.set_default(".", Some(edit));
        for name in ["README", "Makefile", ".bashrc"] {
            assert_eq!(o.app_for(name).map(|a| a.name.as_str()), Some("Editor"), "{name}");
        }
        assert!(o.app_for("notes.txt").is_none(), "only files without an extension");
        assert_eq!(o.rule_for(".").map(|a| a.name.as_str()), Some("Editor"));
        assert!(o.rule_for("txt").is_none());
        o.set_default(".", None);
        assert!(o.app_for("README").is_none());

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrap {
            open: OpenSection,
        }
        let w = Wrap { open: o };
        let s = toml::to_string(&w).unwrap();
        assert_eq!(toml::from_str::<Wrap>(&s).unwrap(), w, "{s}");
    }

    #[test]
    fn player_plays_on_p_and_k_in_every_preset() {
        for p in KeyPreset::ALL {
            let km = preset(p);
            for k in [KeyName::P, KeyName::K] {
                let chord = KeyChord::new(Mods::empty(), k);
                assert_eq!(km.lookup(KeyContext::Player, chord), Some(&Action::PlayerPlayPause), "{p:?} {k:?}");
            }
            // P is the key the hints show.
            assert_eq!(km.chords_for(&Action::PlayerPlayPause).first().map(|c| c.key), Some(KeyName::P), "{p:?}");
        }
        assert!(!Media::default().quicklook_space_plays);
    }

    #[test]
    fn linux_index_exclusions_apply_only_on_linux() {
        let ix = SearchIndex::default();
        let eff = ix.effective_exclude();
        assert!(eff.starts_with(&ix.exclude));
        assert_eq!(eff.iter().any(|p| p == "/proc"), cfg!(target_os = "linux"));
        assert!(ix.linux_exclude.iter().all(|p| !ix.exclude.contains(p)));
    }

    #[test]
    fn tui_defaults_and_round_trip() {
        let t = TuiSection::default();
        assert_eq!(t.color_mode, TuiColorMode::Auto);
        assert_eq!(t.icons, TuiIcons::Unicode);
        assert_eq!(t.borders, TuiBorders::Rounded);
        assert_eq!(t.sidebar_width, 28);
        assert_eq!(t.sidebar_width_clamped(), 28);

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrap {
            tui: TuiSection,
        }
        for color_mode in TuiColorMode::ALL {
            for icons in [TuiIcons::Nerd, TuiIcons::Unicode, TuiIcons::Ascii] {
                for borders in [TuiBorders::Rounded, TuiBorders::Plain, TuiBorders::Ascii, TuiBorders::None] {
                    let w = Wrap { tui: TuiSection { color_mode, icons, borders, sidebar_width: 40, ..t.clone() } };
                    let s = toml::to_string(&w).unwrap();
                    assert_eq!(toml::from_str::<Wrap>(&s).unwrap(), w, "{s}");
                }
            }
        }
        let s = toml::to_string(&Wrap { tui: TuiSection { color_mode: TuiColorMode::Ansi256, ..t.clone() } }).unwrap();
        assert!(s.contains("color_mode = \"ansi256\""), "{s}");
        let s = toml::to_string(&Wrap { tui: TuiSection { borders: TuiBorders::None, ..t } }).unwrap();
        assert!(s.contains("borders = \"none\""), "{s}");
    }

    #[test]
    fn tui_color_mode_accepts_counts() {
        let mode = |v: &str| toml::from_str::<TuiSection>(&format!("color_mode = {v}")).map(|t| t.color_mode);
        assert_eq!(mode("256").unwrap(), TuiColorMode::Ansi256);
        assert_eq!(mode("16").unwrap(), TuiColorMode::Ansi16);
        assert_eq!(mode("\"256\"").unwrap(), TuiColorMode::Ansi256);
        assert_eq!(mode("\"16\"").unwrap(), TuiColorMode::Ansi16);
        assert_eq!(mode("\"truecolor\"").unwrap(), TuiColorMode::Truecolor);
        assert_eq!(mode("\"mono\"").unwrap(), TuiColorMode::Mono);
        assert_eq!(mode("\"auto\"").unwrap(), TuiColorMode::Auto);
        assert_eq!(mode("\"ansi16\"").unwrap(), TuiColorMode::Ansi16);
        assert!(mode("8").is_err());
        assert!(mode("\"rainbow\"").is_err());
        // Missing keys fall back to the defaults.
        let t: TuiSection = toml::from_str("mouse = false").unwrap();
        assert_eq!(t.color_mode, TuiColorMode::Auto);
        assert_eq!(t.sidebar_width, 28);
    }

    #[test]
    fn tui_t3_t4_keys_defaults_round_trip_and_clamps() {
        let t = TuiSection::default();
        assert_eq!(t.osc52, TuiOsc52::Auto);
        assert_eq!(t.images, TuiImages::Auto);
        assert_eq!((t.jobs_rows, t.jobs_rows_clamped()), (8, 8));
        assert_eq!(t.cell_pixels_clamped(), (10, 20));
        assert!((t.preview_ratio_clamped() - 0.38).abs() < 1e-6);
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrap {
            tui: TuiSection,
        }
        for osc52 in TuiOsc52::ALL {
            for images in TuiImages::ALL {
                let w = Wrap { tui: TuiSection { osc52, images, cell_pixels: [8, 16], ..t.clone() } };
                let s = toml::to_string(&w).unwrap();
                assert_eq!(toml::from_str::<Wrap>(&s).unwrap(), w, "{s}");
            }
        }
        let t: TuiSection = toml::from_str(
            "osc52 = \"never\"\nimages = \"halfblocks\"\njobs_rows = 99\ncell_pixels = [0, 500]\npreview_ratio = 0.9",
        )
        .unwrap();
        assert_eq!((t.osc52, t.images), (TuiOsc52::Never, TuiImages::Halfblocks));
        assert_eq!(t.jobs_rows_clamped(), 30);
        assert_eq!(t.cell_pixels_clamped(), (1, 200));
        assert!((t.preview_ratio_clamped() - 0.6).abs() < 1e-6);
        let low = TuiSection { jobs_rows: 0, preview_ratio: 0.0, ..TuiSection::default() };
        assert_eq!(low.jobs_rows_clamped(), 3);
        assert!((low.preview_ratio_clamped() - 0.2).abs() < 1e-6);
        let nan = TuiSection { preview_ratio: f32::NAN, ..TuiSection::default() };
        assert!((nan.preview_ratio_clamped() - 0.38).abs() < 1e-6);
        assert!(toml::from_str::<TuiSection>("images = \"svga\"").is_err());
    }

    #[test]
    fn tui_t5_t6_keys_defaults_round_trip_and_clamps() {
        let t = TuiSection::default();
        assert_eq!(t.shell_mode, TuiShellMode::Pane);
        assert_eq!((t.terminal_rows, t.terminal_rows_clamped()), (12, 12));
        assert!((t.ai_ratio_clamped() - 0.4).abs() < 1e-6);
        assert_eq!(t.editor, "");
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrap {
            tui: TuiSection,
        }
        for shell_mode in TuiShellMode::ALL {
            let w = Wrap { tui: TuiSection { shell_mode, editor: "code --wait".into(), ..t.clone() } };
            let s = toml::to_string(&w).unwrap();
            assert_eq!(toml::from_str::<Wrap>(&s).unwrap(), w, "{s}");
        }
        let t: TuiSection =
            toml::from_str("shell_mode = \"suspend\"\nterminal_rows = 99\nai_ratio = 0.9\neditor = \"nvim\"").unwrap();
        assert_eq!((t.shell_mode, t.editor.as_str()), (TuiShellMode::Suspend, "nvim"));
        assert_eq!(t.terminal_rows_clamped(), 40);
        assert!((t.ai_ratio_clamped() - 0.6).abs() < 1e-6);
        let low = TuiSection { terminal_rows: 0, ai_ratio: 0.0, ..TuiSection::default() };
        assert_eq!(low.terminal_rows_clamped(), 3);
        assert!((low.ai_ratio_clamped() - 0.25).abs() < 1e-6);
        let nan = TuiSection { ai_ratio: f32::NAN, ..TuiSection::default() };
        assert!((nan.ai_ratio_clamped() - 0.4).abs() < 1e-6);
        assert!(toml::from_str::<TuiSection>("shell_mode = \"tab\"").is_err());
    }

    #[test]
    fn tui_sidebar_width_is_clamped_when_read() {
        let w = |n: u16| TuiSection { sidebar_width: n, ..TuiSection::default() }.sidebar_width_clamped();
        assert_eq!(w(0), 12);
        assert_eq!(w(11), 12);
        assert_eq!(w(12), 12);
        assert_eq!(w(45), 45);
        assert_eq!(w(60), 60);
        assert_eq!(w(500), 60);
    }

    #[test]
    fn explorer_basics() {
        let km = preset(KeyPreset::Explorer);
        let ch = |s: &str| s.parse::<KeyChord>().unwrap();
        assert_eq!(km.lookup(KeyContext::FileList, ch("Ctrl+C")), Some(&Action::Copy));
        assert_eq!(km.lookup(KeyContext::FileList, ch("F2")), Some(&Action::Rename));
        assert_eq!(km.lookup(KeyContext::Terminal, ch("Ctrl+Shift+P")), Some(&Action::CommandPalette));
        let cmd = preset(KeyPreset::Commander);
        assert_eq!(cmd.lookup(KeyContext::FileList, ch("F5")), Some(&Action::CopyToOtherPane));
        assert_eq!(cmd.lookup(KeyContext::FileList, ch("F4")), Some(&Action::OpenInEditor));
        assert_eq!(km.lookup(KeyContext::FileList, ch("F4")), Some(&Action::OpenInEditor));
        assert_eq!(km.lookup(KeyContext::FileList, ch("Ctrl+K")), Some(&Action::CommandPalette));
        assert_eq!(km.lookup(KeyContext::FileList, ch("Ctrl+P")), Some(&Action::JumpToFolderDialog));
    }

    #[test]
    fn finder_and_vim() {
        let ch = |s: &str| s.parse::<KeyChord>().unwrap();
        let f = preset(KeyPreset::Finder);
        assert_eq!(f.lookup(KeyContext::FileList, ch("Enter")), Some(&Action::Rename));
        assert_eq!(f.lookup(KeyContext::FileList, ch("Ctrl+Down")), Some(&Action::Open));
        assert_eq!(f.lookup(KeyContext::Global, ch("Ctrl+2")), Some(&Action::ViewDetails));
        let v = preset(KeyPreset::Vim);
        assert_eq!(v.lookup(KeyContext::FileList, ch("J")), Some(&Action::CursorDown));
        let gg = super::super::keys::parse_sequence("g g").unwrap();
        assert_eq!(
            v.sequence_match(KeyContext::FileList, &gg),
            super::super::keys::SeqMatch::Exact(Action::CursorHome)
        );
    }
}
