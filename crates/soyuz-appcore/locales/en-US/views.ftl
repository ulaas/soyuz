### Soyuz UI strings (en-US source): views
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/views.ftl.

## Group headers (state/listing.rs)

views-group-folders = Folders
views-group-no-extension = No extension
views-group-no-tags = No tags
views-group-size-empty = Empty
views-group-size-tiny = Tiny (< 16 KiB)
views-group-size-small = Small (< 1 MiB)
views-group-size-medium = Medium (< 128 MiB)
views-group-size-large = Large (< 1 GiB)
views-group-size-huge = Huge (≥ 1 GiB)
views-group-date-unknown = Unknown date
views-group-date-future = In the future
views-group-date-today = Today
views-group-date-yesterday = Yesterday
views-group-date-week = Earlier this week
views-group-date-month = Earlier this month
views-group-date-year = Earlier this year
views-group-date-older = A long time ago

## Kind column (state/listing.rs kind_label)

views-kind-folder = Folder
views-kind-junction = Junction
views-kind-folder-link = Folder link
views-kind-broken-link = Broken link
views-kind-device = Device
views-kind-pipe = Pipe
views-kind-socket = Socket
views-kind-file = File
# $ext: the file extension in upper case, e.g. "WEIRD".
views-kind-ext-file = { $ext } file
views-kind-png-image = PNG image
views-kind-jpeg-image = JPEG image
views-kind-gif-image = GIF image
views-kind-webp-image = WebP image
views-kind-bitmap-image = Bitmap image
views-kind-svg-image = SVG image
views-kind-icon = Icon
views-kind-tiff-image = TIFF image
views-kind-heic-image = HEIC image
views-kind-mp4-video = MP4 video
views-kind-matroska-video = Matroska video
views-kind-quicktime-video = QuickTime video
views-kind-avi-video = AVI video
views-kind-webm-video = WebM video
views-kind-mp3-audio = MP3 audio
views-kind-flac-audio = FLAC audio
views-kind-wav-audio = WAV audio
views-kind-ogg-audio = Ogg audio
views-kind-aac-audio = AAC audio
views-kind-zip-archive = ZIP archive
views-kind-7-zip-archive = 7-Zip archive
views-kind-rar-archive = RAR archive
views-kind-tar-archive = Tar archive
views-kind-gzip-archive = Gzip archive
views-kind-compressed-file = Compressed file
views-kind-disc-image = Disc image
views-kind-pdf-document = PDF document
views-kind-word-document = Word document
views-kind-excel-workbook = Excel workbook
views-kind-powerpoint-deck = PowerPoint deck
views-kind-text-document = Text document
views-kind-text-file = Text file
views-kind-rich-text = Rich text
views-kind-csv-table = CSV table
views-kind-html-page = HTML page
views-kind-stylesheet = Stylesheet
views-kind-rust-source = Rust source
views-kind-python-script = Python script
views-kind-c-source = C source
views-kind-cpp-source = C++ source
views-kind-csharp-source = C# source
views-kind-go-source = Go source
views-kind-java-source = Java source
views-kind-kotlin-source = Kotlin source
views-kind-swift-source = Swift source
views-kind-shell-script = Shell script
views-kind-powershell-script = PowerShell script
views-kind-batch-file = Batch file
views-kind-application = Application
views-kind-installer = Installer
views-kind-library = Library
views-kind-shortcut = Shortcut
views-kind-font = Font
views-kind-log-file = Log file
views-kind-settings = Settings
views-kind-lock-file = Lock file

## Computer view (ui/views/computer.rs)

views-computer-title = Devices and drives
# $free, $total: formatted sizes ("12.3 GB").
views-computer-free-of = { $free } free of { $total }
views-computer-drive = Drive
views-computer-looking = Looking for drives…
views-volume-fixed = Local Disk
views-volume-removable = Removable Disk
views-volume-network = Network Drive
views-volume-optical = CD Drive
views-volume-ram = RAM Disk

## File list (ui/views/flow.rs, miller.rs)

# Screen-reader name of the file list; $title: the folder's display name.
views-a11y-files-in = Files in { $title }
# Screen-reader name of a row; $kind: "folder" | "file".
views-a11y-row = { $kind ->
    [folder] { $name }, folder
   *[other] { $name }, file
}
views-miller-unreadable = Can't read this folder
views-miller-empty = Empty folder

## Trash view (ui/views/trash.rs)

views-trash-title = Trash
views-trash-reader-failed = Couldn't start the trash reader
views-trash-reading = Reading the trash…
views-trash-empty-short = Empty
# $size: formatted total size.
views-trash-summary = { $count ->
    [one] { $count } item · { $size }
   *[other] { $count } items · { $size }
}
views-trash-summary-selected = { $count ->
    [one] { $count } item · { $size } · { $selected } selected
   *[other] { $count } items · { $size } · { $selected } selected
}
views-trash-empty-trash = Empty trash
views-trash-empty-trash-tip = Delete everything in the trash permanently
views-trash-delete-permanently = Delete permanently
views-trash-restore = Restore
views-trash-restore-selected = Restore selected
views-trash-refresh = Refresh
views-trash-unreadable = Can't read the trash
views-trash-empty-title = Trash is empty
views-trash-empty-subtitle = Items you delete stay here until you empty the trash.
views-trash-col-original = Original location
views-trash-col-deleted = Date deleted
views-trash-unknown-origin = Unknown
views-trash-open-original = Open original location
views-trash-copy-original = Copy original path

## Pane body states (ui/pane_view.rs)

views-pane-trash-pending = Browsing the trash is not available here yet.
views-pane-recent = Recent
views-pane-recent-pending = Recent items are not tracked yet.
views-pane-search = Search
views-pane-search-pending = Search results will appear here.
views-pane-archive = Archive
views-pane-archive-pending = Browsing archives is not available here yet.
views-pane-tags = Tags
views-pane-tags-pending = Tags are not available yet.
views-pane-access-denied = Access denied
views-pane-not-found = Folder not found
views-pane-not-a-folder = Not a folder
views-pane-cant-open = Can't open this folder
views-pane-hint-go-up = Go up
views-pane-hint-go-back = Go back
views-pane-no-matches = No matches
# $filter: the text typed into the live filter.
views-pane-no-matches-sub = Nothing here matches “{ $filter }”.
views-pane-hint-clear-filter = Clear filter
views-pane-only-hidden = Only hidden items
views-pane-only-hidden-sub = This folder contains hidden items only.
views-pane-hint-show-hidden = Show hidden files
views-pane-timeline-empty = No changes this week
views-pane-timeline-empty-sub = Files you change in the folders Soyuz knows about show up here, newest first.
views-pane-empty = This folder is empty
views-pane-empty-sub = Drop files here or create something new.

## Folder tree (ui/tree)

views-tree-home = Home
views-tree-filter = Filter folders
views-tree-clear-filter = Clear filter
views-tree-a11y = Disk tree
views-col-resize = Resize the { $column } column
views-tree-no-matches = No matching folders
views-tree-open = Open
views-tree-open-new-tab = Open in new tab
views-tree-open-other-pane = Open in other pane
views-tree-collapse = Collapse
views-tree-expand = Expand
views-tree-new-folder = New folder
views-tree-copy-path = Copy path
views-tree-add-favorite = Add to favorites
views-tree-properties = Properties
views-tree-read-failed = Couldn't read the folder
# Name given to a folder created from the tree's context menu.
views-tree-new-folder-name = New folder
views-tree-hidden = Disk tree hidden
views-tree-shown = Disk tree shown

## Preview pane (ui/preview)

views-preview-title = Preview
views-preview-diff = Diff
views-preview-show-hex = Show as hex
views-preview-close = Close preview  (Alt+P)
views-preview-info = INFO
views-preview-nothing-selected = Nothing selected
views-preview-nothing-selected-sub = Select a file to preview it.
views-preview-truncated = Preview truncated
views-preview-text-truncated = … preview truncated ([preview] max_text_kib)
views-preview-archive-entries = { $count ->
    [one] { $count } entry
   *[other] { $count } entries
}
# $format: archive format name ("ZIP"); $size: formatted size.
views-preview-archive-unpacked = { $format } · { $size } unpacked
views-preview-browse = Browse
views-preview-show-bytes = Show bytes
views-preview-folder-unreadable = Can't list this folder
views-preview-folder-counts = { $dirs ->
    [one] { $dirs } folder
   *[other] { $dirs } folders
    } · { $files ->
    [one] { $files } file
   *[other] { $files } files
}
# $size: formatted size of everything inside the folder.
views-preview-folder-size = { $files ->
    [one] { $size } in { $files } file
   *[other] { $size } in { $files } files
}
views-preview-folder-size-running = { $files ->
    [one] { $size } in { $files } file …
   *[other] { $size } in { $files } files …
}
views-preview-folder-direct = { $size } in direct files
views-preview-folder-hidden = { $count } hidden
# $name: newest item; $when: relative time ("5 minutes ago").
views-preview-folder-newest = Newest: { $name } · { $when }
views-preview-folder-notes = Notes
views-preview-folder-notes-edit = Edit

views-preview-kind-folder = Folder
views-preview-kind-image = Image
views-preview-kind-svg = SVG image
views-preview-kind-text = Text
views-preview-kind-archive = Archive
views-preview-kind-audio = Audio
views-preview-kind-video = Video
views-preview-kind-pdf = PDF document
views-preview-kind-binary = Binary
views-preview-kind-document = Document
views-preview-kind-spreadsheet = Spreadsheet
views-preview-kind-presentation = Presentation
views-preview-kind-ebook = E-book
views-preview-kind-font = Font
views-preview-kind-model = 3D model

## Info rows (ui/preview/render.rs info_rows / info_label)

# $files, $dirs: counts inside a folder.
views-info-contains = { $files ->
    [one] { $files } file
   *[other] { $files } files
    }, { $dirs ->
    [one] { $dirs } folder
   *[other] { $dirs } folders
}
# $size: formatted size; $bytes: exact byte count.
views-info-size-bytes = { $bytes ->
    [one] { $size } ({ $bytes } byte)
   *[other] { $size } ({ $bytes } bytes)
}
views-info-type = Type
views-info-size = Size
views-info-contains-label = Contains
views-info-compressed = Compressed
views-info-where = Where
views-info-modified = Modified
views-info-created = Created
views-info-accessed = Accessed
views-info-attributes = Attributes
views-info-mode = Mode
views-info-owner = Owner
views-info-title = Title
views-info-artist = Artist
views-info-album = Album
views-info-album-artist = Album artist
views-info-year = Year
views-info-genre = Genre
views-info-track = Track
views-info-composer = Composer
views-info-container = Container
views-info-format = Format
views-info-sample-rate = Sample rate
views-info-channels = Channels
views-info-bit-depth = Bit depth
views-info-duration = Duration
views-info-dimensions = Dimensions
views-info-frame-rate = Frame rate
views-info-bitrate = Bitrate
views-info-tracks = Tracks
views-info-color = Color
views-info-compression = Compression
views-info-encoding = Encoding
views-info-encrypted = Encrypted
views-info-entries = Entries
views-info-language = Language
views-info-lines = Lines
views-info-note = Note
views-info-pdf-version = PDF version
views-info-previewer = Previewer
views-info-pages = Pages
views-info-author = Author
views-info-producer = Producer
views-info-publisher = Publisher
views-info-published = Published
views-info-version = Version
views-info-epub-version = EPUB version
views-info-page-size = Page size
views-info-paragraphs = Paragraphs
views-info-codec = Codec
views-info-copyright = Copyright
views-info-designer = Designer
views-info-weight = Weight
views-info-spacing = Spacing
views-info-variable-axes = Variable axes
views-info-bounds = Bounds
views-info-faces = Faces
views-info-meshes = Meshes
views-info-drawn = Drawn
views-info-shown = Shown
views-info-renderer = Renderer
views-info-waveform = Waveform
views-info-video = Video
views-info-audio = Audio
views-info-subtitles = Subtitles
views-info-pixel-format = Pixel format
views-info-hdr = HDR
views-info-rotation = Rotation
views-info-video-bitrate = Video bitrate
views-info-audio-bitrate = Audio bitrate
views-info-comment = Comment
views-info-encoder = Encoder
views-info-words = Words
views-info-sheets = Sheets
views-info-slides = Slides
views-info-chapters = Chapters
views-info-family = Family
views-info-style = Style
views-info-glyphs = Glyphs
views-info-triangles = Triangles
views-info-vertices = Vertices

## Diff (ui/preview/diff.rs)

views-diff-swap = Swap sides
views-diff-unified = Unified
views-diff-side-by-side = Side by side
views-diff-failed = Can't compare these files
views-diff-identical = The files are identical
views-diff-binary-differ = Binary files differ
views-diff-binary-sub = At least one side isn't text, so there is no line diff.
views-diff-none = No differences
views-diff-none-sub = Both files have the same content.
views-diff-truncated = … diff truncated ([preview] max_diff_kib)

## Rich previews (ui/preview/rich.rs)

views-rich-first-paragraphs = … preview shows the first { $count } paragraphs
views-rich-doc-no-text = This document has no text.
views-rich-first-sheet-only = Only the first sheet is previewed
views-rich-sheet-empty = This sheet is empty
views-rich-more-rows = … more rows ([preview] sheet_rows)
views-rich-no-slides = No slides found.
views-rich-untitled-slide = Untitled slide
views-rich-untitled = Untitled
views-rich-contents = CONTENTS
views-rich-model-hint-full = Drag to rotate · scroll to zoom · double-click resets
views-rich-model-hint = Drag to rotate
views-rich-play = Play
views-rich-audio-mono = mono
views-rich-audio-stereo = stereo
# $count: channels (6).
views-rich-audio-channels = { $count } channels
views-player-off = The video player is off ([media] player)
# $error: why the file could not be opened.
views-player-cant-open = Can't open this file for playback ({ $error })
views-rich-pause = Pause
# $name: the file shown by the video player (its surface is a play / pause button).
views-player-surface = Video: { $name }
views-player-opening = Opening…
views-player-position = Position
views-player-volume = Volume
views-player-mute = Mute
views-player-unmute = Unmute
views-player-loop = Loop
views-rich-poster-note = Poster frame (FFmpeg) · playback opens in your player (Enter)
views-rich-prev-page = Previous page  (PgUp)
views-rich-next-page = Next page  (PgDn)
views-rich-page-of = Page { $page } of { $pages }
views-rich-pages-text-only = { $pages ->
    [one] { $pages } page · text only
   *[other] { $pages } pages · text only
}
views-rich-page = Page
views-rich-text = Text
views-rich-pdf-no-text = No extractable text on the first pages.
views-rich-pdf-text-caption = TEXT (FIRST PAGES)
views-rich-pdfium-needed = PDF page preview needs pdfium
views-rich-pdf-locked = This PDF is password protected
views-rich-pdf-locked-body = Enter its password to see the pages and the text. Soyuz keeps it until it closes, never on disk.
views-rich-pdf-unlock = Enter password…

## Quick Look (ui/quicklook.rs)

views-quicklook-close = Close  (Esc)
# $pos: 1-based position of the shown item; $count: visible items.
views-quicklook-position = { $pos } of { $count }
views-quicklook-hint-close = close
views-quicklook-hint-browse = browse
views-quicklook-hint-open = open
views-quicklook-hint-play = play / pause

## Search bar (ui/search.rs)

views-search-flat-needs-folder = Flat view works in folders
# Title of the flat view; $folder: the folder's name.
views-search-flat-title = All files in { $folder }
views-search-save-failed = Couldn't save searches
views-search-removed = Removed saved search “{ $name }”
views-search-failed = Search failed
views-search-no-folders-to-measure = No folders to measure
views-search-close = Close search  (Esc)
views-search-save-tip = Save this search
views-search-stop = Stop
views-search-run = Search
views-search-everywhere = Everywhere
# Scope chip; $folder: folder name.
views-search-in = in { $folder }
views-search-everywhere-tip = Search every indexed folder
views-search-in-tip = Search in { $path }
views-search-this-folder = This folder
views-search-this-folder-named = This folder ({ $folder })
views-search-all-indexed = All indexed folders
views-search-all-indexed-tip = Instant results from the background index
views-search-all-indexed-off-tip = Turn on the background index in Settings → Preview & search for instant results
views-search-gitignore-tip = Respect .gitignore files
views-search-hidden = Hidden
views-search-hidden-tip = Include hidden files
views-search-kind = Kind
views-search-kind-tip = Filter by kind
views-search-date = Date
views-search-date-tip = Modified date
views-search-size = Size
views-search-size-tip = File size
views-search-contents = Contents
views-search-contents-tip = Search inside files
views-search-case-tip = Match case
views-search-mode-tip = How the name is matched
views-search-regex-tip = Regular expression
views-search-mode-contains = Contains
views-search-mode-glob = Glob
views-search-mode-regex = Regex
views-search-mode-fuzzy = Fuzzy
views-search-mode-contains-tip = Name contains the text
views-search-mode-fuzzy-tip = Loose match, ranked by relevance
views-search-kind-any = Any kind
views-search-kind-files = Files
views-search-kind-folders = Folders
views-search-kind-images = Images
views-search-kind-videos = Videos
views-search-kind-audio = Audio
views-search-kind-documents = Documents
views-search-kind-archives = Archives
views-search-kind-code = Code
views-search-date-any = Any time
views-search-date-today = Last 24 hours
views-search-date-week = Last 7 days
views-search-date-month = Last 30 days
views-search-date-year = Last 12 months
views-search-date-older = Older than a year
views-search-size-caption = FILE SIZE
views-search-size-min = Min
views-search-size-min-hint = e.g. 10 MB
views-search-size-max = Max
views-search-size-max-hint = any
views-search-size-tiny = Tiny (< 16 KB)
views-search-size-large = Large (> 100 MB)
views-search-size-huge = Huge (> 1 GB)
views-search-size-any = Any size
views-search-hint-content = Search file contents…
views-search-hint-index = Search the index…  kind:image  size:>5MB  modified:<7d  tag:red  -draft
views-search-hint-names = Search names in this folder and below…  (kind:image size:>5MB …)
views-search-hint-glob = in files: *.rs
views-search-save-caption = SAVE SEARCH
views-search-saved = Saved “{ $name }”
views-search-save-one-failed = Couldn't save the search
# Status strip; $title: the search's title; $count: results so far.
views-search-head = { $count ->
    [one] { $title } — { $count } result
   *[other] { $title } — { $count } results
}
# $ms: elapsed milliseconds.
views-search-from-index = from the index in { $ms } ms
# $count: entries scanned; $secs: elapsed seconds (formatted).
views-search-scanned = scanned { $count } in { $secs } s
views-search-limit-reached = result limit reached
views-search-cancelled = cancelled
views-search-unreadable = { $count } unreadable
views-search-cancel-tip = Stop searching
views-search-relevance = Relevance
views-search-relevance-tip = Order by match quality
views-search-edit = Edit
views-search-edit-smart-tip = Edit this smart folder
views-search-smart-folder = Smart folder
views-search-smart-folder-tip = Save as a smart folder in the sidebar (stays up to date)
views-search-refine = Refine
views-search-refine-tip = Edit this search
views-search-sidebar = Searches
views-search-run-saved = Run search
views-search-remove = Remove
views-search-defined-in-config = Defined in config.toml

## Smart folders (ui/smart.rs)

views-smart-need-name = Give the smart folder a name
views-smart-query-empty = The query is empty
views-smart-name-exists = There already is a smart folder called “{ $name }”
views-smart-saved = Smart folder “{ $name }” saved
views-smart-save-failed = Couldn't save the smart folder
views-smart-deleted = Deleted smart folder “{ $name }”
views-smart-deleted-body = Only the saved query was removed; no files were touched.
views-smart-rebuilding = Rebuilding the index
views-smart-rebuilding-body = Searches walk the disk until it is done.
# Toast title above an error; $name: the smart folder.
views-smart-error-title = Smart folder “{ $name }”
views-smart-section = Smart folders
views-smart-open = Open
views-smart-open-new-tab = Open in new tab
views-smart-edit = Edit or rename…
views-smart-delete = Delete…
views-smart-new = New smart folder…
views-smart-new-tip = A saved query that stays up to date, e.g. kind:image modified:<7d
views-smart-help-words = name contains the words / the phrase
views-smart-help-pattern = name pattern, extensions
views-smart-help-kind = image video audio document archive code file folder
views-smart-help-size = file size
views-smart-help-age = age (h d w mo y) or date
views-smart-help-tag = color tag, path text, folder
views-smart-help-content = words inside text files (needs content indexing)
views-smart-help-logic = exclude, either, group
views-smart-dialog-edit = Edit smart folder
views-smart-dialog-new = New smart folder
views-smart-name = Name
views-smart-name-hint = Recent photos
views-smart-query = Query
views-smart-live = Answered by the index; refreshes by itself as files change.
views-smart-not-covered = The index doesn't cover this yet; the folder searches the disk until it does.
views-smart-index-off = Indexing is off, so the folder searches the disk when opened (Settings → Preview & search).
views-smart-include-hidden = Include hidden files
views-smart-type-query = Type a query
views-smart-name-taken = That name is taken
views-smart-delete-title = Delete smart folder
views-smart-delete-confirm = Delete the smart folder “{ $name }”?
views-smart-delete-body = Only the saved query goes away; the files it shows are not touched.

## Index status (ui/smart.rs status_line / status_chip)

views-index-starting = Starting the index…
# $pct: percent done (integer); $count: items scanned.
views-index-crawling-pct = Indexing { $pct }% · { $count ->
    [one] { $count } item
   *[other] { $count } items
}
views-index-crawling = Indexing · { $count ->
    [one] { $count } item
   *[other] { $count } items
}
views-index-verifying = Checking for changes · { $count ->
    [one] { $count } item
   *[other] { $count } items
}
views-index-catching-up = Applying changes from the NTFS journal…
views-index-contents = Reading file contents · { $count }
views-index-idle = Indexed · { $files ->
    [one] { $files } file
   *[other] { $files } files
    }, { $folders ->
    [one] { $folders } folder
   *[other] { $folders } folders
}
views-index-failed = Index off: { $error }
views-index-chip-pct = Indexing { $pct }%
views-index-chip-indexing = Indexing
views-index-chip-updating = Updating index
views-index-chip-contents = Indexing contents
views-index-chip-starting = Index
views-index-chip-idle = Indexed
views-index-chip-off = Index off
views-index-chip-tip = Click for index settings; right-click for more
views-index-settings = Index settings…
views-index-rebuild = Rebuild index

## Archives (ui/archive.rs)

views-archive-opening = Opening { $name }…
# Job labels (jobs panel); $name: archive or archive file name.
views-archive-job-extract = Extract { $name }
views-archive-extract-failed = Couldn't start the extraction
views-archive-job-compress = Compress { $name }
# $archive: the zip's file name.
views-archive-job-delete = { $count ->
    [one] Delete { $count } item from { $archive }
   *[other] Delete { $count } items from { $archive }
}
views-archive-job-add = { $count ->
    [one] Add { $count } item to { $archive }
   *[other] Add { $count } items to { $archive }
}
views-archive-job-rename = Rename { $name } in { $archive }
views-archive-select-to-extract = Select an archive to extract
views-archive-nothing-to-extract = Nothing to extract
views-archive-read-failed = Couldn't read the archive
views-archive-compress-needs-folder = Compress works on files in a folder
views-archive-select-to-compress = Select files or folders to compress
views-archive-test-ok = { $name } is OK
views-archive-test-ok-body = { $count } entries tested
views-archive-test-damaged = { $name } has { $count } damaged entries
views-archive-test-failed = Couldn't test { $name }
views-archive-extract-selected = Extract selected…
views-archive-extract-all = Extract all…
views-archive-extract-here = Extract here
views-archive-extract = Extract
views-archive-extract-to-folder = Extract to folder
views-archive-extract-to = Extract to…
views-archive-test = Test archive
views-archive-compress-menu = Compress…
views-archive-password-title = Password required
views-archive-password-body = “{ $name }” is encrypted. Enter its password to browse it.
views-archive-password-body-pdf = “{ $name }” is encrypted. Enter its password to preview it (kept until Soyuz closes).
views-archive-pdf-password-menu = PDF password…
views-archive-pdf-password-select = Select one PDF file first
views-archive-password-wrong = That password didn't work.
views-archive-unlock = Unlock
views-archive-extract-what = { $count ->
    [one] { $count } selected item from { $name }
   *[other] { $count } selected items from { $name }
}
views-archive-n-archives = { $count } archives
views-archive-destination = Destination folder
views-archive-subfolder = Create a folder named after the archive
views-archive-compress = Compress
views-archive-name = Archive name
views-archive-format = Format
views-archive-compression = Compression
views-archive-level-store = Store
views-archive-level-fast = Fast
views-archive-level-normal = Normal
views-archive-level-max = Maximum
views-archive-password-optional = Password (optional)
views-archive-names-encrypted = File names are encrypted too.
views-archive-exists = “{ $name }” already exists
views-archive-exists-body = Choose another name.
views-archive-select-to-delete = Select entries to delete from the archive
views-archive-select-one-to-rename = Select one entry to rename
views-archive-nothing-to-add = Nothing to add
views-archive-nothing-to-add-body = Copy files first (Ctrl+C), then paste here.
views-archive-add-here = Add copied files here
views-archive-rename-menu = Rename in archive…
views-archive-delete-menu = Delete from archive
views-archive-rename-title = Rename in archive
# $kind: "folder" | "file"; $name: entry path inside the archive; $archive: archive file name.
views-archive-rename-body = { $kind ->
    [folder] New name for the folder “{ $name }” in { $archive }
   *[other] New name for the file “{ $name }” in { $archive }
}
views-archive-no-slashes = A name can't contain slashes.
