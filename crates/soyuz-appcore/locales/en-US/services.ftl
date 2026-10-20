### Soyuz UI strings (en-US source): services
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/services.ftl.


## Preview kinds (shown in the preview header)

services-kind-folder = Folder
services-kind-image = Image
services-kind-svg = SVG image
services-kind-text = Text
services-kind-markdown = Markdown
services-kind-archive = Archive
services-kind-audio = Audio
services-kind-video = Video
services-kind-pdf = PDF document
services-kind-binary = Binary
services-kind-document = Document
services-kind-spreadsheet = Spreadsheet
services-kind-presentation = Presentation
services-kind-ebook = E-book
services-kind-font = Font
services-kind-model = 3D model

## Preview messages

services-preview-internal-error = internal error
services-preview-failed = Preview failed
# $reason: panic message.
services-preview-crashed = The previewer crashed on this file ({ $reason }).
services-preview-entry-not-found = entry not found
services-preview-cant-read-file = Can't read this file
services-preview-limited = Preview limited to the first 64 MiB
services-preview-pdf-no-feature = Page rendering needs the optional `pdf` build feature. Showing the raw bytes is one click away.
services-preview-cant-read-image = Can't read this image
services-preview-unsupported-image = Unsupported image
services-preview-image-too-large = Image too large to preview
# $mp: image size in megapixels; $limit: the configured limit.
services-preview-image-too-large-body = { $mp } MP exceeds the { $limit } MP preview limit ([preview] max_image_mp).
services-preview-cant-decode-image = Can't decode this image
services-preview-script = Script
services-preview-script-failed = The script previewer failed
services-preview-nested-archive = Nested archive
services-preview-nested-archive-body = Open it from its folder to browse it.
services-preview-password = Password protected
services-preview-password-body = Press Enter to open it and type the password.
services-preview-cant-read-archive = Can't read this archive

## Preview fact values (labels are translated by the preview panel)

# $ratio: percentage of space saved.
services-fact-compression-saved = { $ratio }% saved

## Audio / video metadata values

services-media-wave = WAVE audio
services-media-ieee-float = IEEE float
# $bits: sample size, e.g. 16.
services-media-bits = { $bits }-bit
services-media-mono = Mono
services-media-stereo = Stereo
services-media-flac = FLAC audio
services-media-mp3 = MP3 audio
services-media-quicktime = QuickTime movie
services-media-mpeg4-audio = MPEG-4 audio
services-media-tracks-av = Video + audio
services-media-tracks-video = Video
services-media-tracks-audio = Audio

## Rich previews (documents, e-books, fonts, 3D models, audio, video, PDF)

services-preview-too-large = Too large to preview
# $size, $limit: MiB; $key: the config key.
services-preview-too-large-body = { $size } MiB exceeds the { $limit } MiB limit ([preview] { $key }).
services-preview-sheet-shown = { $rows } rows × { $cols } columns
services-preview-cant-read-document = Can't read this document
services-preview-cant-read-ebook = Can't read this e-book
services-preview-cant-read-font = Can't read this font
services-preview-woff2 = WOFF 2 font
services-preview-woff2-body = WOFF 2 needs a Brotli and glyph-transform decoder that isn't built in. TTF, OTF, TTC and WOFF preview.
services-preview-cant-render-font = Can't render this font
services-preview-monospaced = Monospaced
services-preview-faces = { $count } (first shown)
services-preview-cant-read-model = Can't read this model
services-preview-decimated = { $count } (decimated)
services-preview-waveform-skipped = Skipped above { $size } MiB ([preview] max_audio_mib)
services-preview-poster-disk-only = Poster frames are only made for files on disk.
# $tag: the GitHub release that carries the libraries (ffmpeg-runtime-9.0.2); $dir: the config folder's ffmpeg/ sub-folder.
services-preview-no-ffmpeg-libs = The FFmpeg libraries aren't loaded, so there is no poster frame. Soyuz ships them next to the program; to add them, run “Download FFmpeg runtime” (Settings → Preview → Media or the command palette), get the "{ $tag }" release from GitHub and unpack it into { $dir }, or point Settings → Preview → Media at their folder.
services-preview-crashed-last-time = Not previewed: decoding this file crashed the media library last time. Open it in Quick Look to try again.
services-preview-no-pdf-renderer = This build has no PDF renderer (cargo feature `pdf`).
services-preview-no-pdf-renderer-short = No PDF renderer in this build
services-preview-cant-read-pdf = Can't read this PDF
services-preview-text-only = text only

## Audio and video previews

services-audio-unsupported = Unsupported audio ({ $error })
services-audio-no-track = No audio track
services-audio-unsupported-codec = Unsupported codec ({ $error })
services-audio-player-failed = The audio player failed on this file
services-audio-no-device = No audio output device ({ $error })
services-audio-cant-play = Can't play this file ({ $error })
services-audio-no-ffmpeg = The FFmpeg libraries are not loaded ([media] audio_backend = ffmpeg)
services-audio-open-timeout = Opening the file for playback took too long
# $major: the avcodec major version (63).
services-audio-decoder-ffmpeg = FFmpeg (avcodec { $major })
services-audio-decoder-symphonia = Built in (Symphonia)
# $error: what the FFmpeg libraries said.
services-video-no-frame = No poster frame ({ $error })
services-diff-failed = The diff failed on these files

## Places, timeline, stores

services-places-onedrive-work = OneDrive (work)
# Timeline header; counts of files changed.
services-timeline-summary = { $today } today · { $week } this week · { $total } in total
services-store-blocked = { $path } could not be read at startup; not overwriting it (fix or delete it and restart)

## Configuration reload

services-config-reloaded-warnings = Configuration reloaded with warnings
services-config-reloaded = Configuration reloaded
services-config-error-at = config.toml: error at line { $line }, column { $col }
services-config-load-failed = config.toml could not be loaded
# $error: the parser's message (English).
services-config-error-body =
    { $error }
    The previous settings stay active.
services-config-edit = Edit config

## Office documents, e-books and PDF

services-office-bad-container = Not a valid document container ({ $error })
# $part: the file inside the document package.
services-office-no-body = This document has no readable body ({ $part })
services-office-no-body-sheet = This spreadsheet has no readable body ({ $part })
services-office-no-body-slides = This presentation has no readable body ({ $part })
services-office-first-sheet = The first sheet can't be read
services-office-word = Word document
services-office-odt = OpenDocument text
services-office-excel = Excel workbook
services-office-ods = OpenDocument spreadsheet
services-office-pptx = PowerPoint presentation
services-office-odp = OpenDocument presentation
# Fallback name of a worksheet without a name.
services-office-sheet = Sheet
# Boolean spreadsheet cell values.
services-office-true = TRUE
services-office-false = FALSE
services-epub-no-container = Not an EPUB (META-INF/container.xml is missing)
services-epub-no-package = The EPUB names no package document
services-epub-package-unreadable = The EPUB package document can't be read
services-pdf-over-limit = Larger than the { $limit } MiB document limit
services-pdf-cant-read = Can't read this PDF ({ $error })
services-pdf-parser-failed = The PDF parser failed on this file
services-pdf-renderer-stopped = The PDF renderer stopped
services-pdf-renderer-failed = The PDF renderer failed on this file
services-pdf-too-large = This PDF is too large
services-pdf-password = This PDF is password protected
services-pdf-cant-open = Can't open this PDF ({ $error })
services-pdf-no-page = No page { $page } ({ $error })
services-pdf-cant-render-page = Can't render this page ({ $error })
services-pdf-page-empty = The page rendered empty
# $file: pdfium library file name; $folders: searched folders, one per paragraph.
services-pdf-missing =
    Page rendering uses pdfium, which isn't bundled. Download a prebuilt { $file } (e.g. from the bblanchon/pdfium-binaries releases on GitHub, matching your CPU) and put it in one of these folders:

    { $folders }

    Or set [preview] pdfium_dir. No restart is needed.

## Fonts and 3D models

services-font-collection = TrueType collection
services-font-bad-woff = Damaged WOFF file
services-font-woff-table = WOFF table too large
services-font-unreadable = Not a readable font ({ $error })
services-font-empty-collection = The font collection is empty
services-font-unnamed = Unnamed font
services-font-regular = Regular
services-font-glyphs-caption = Glyphs ({ $count } shown)
services-font-cant-render = This font can't be rendered
# Sample sentence of the font specimen; ideally uses every letter of the language.
services-font-pangram = The quick brown fox jumps over the lazy dog.
services-font-characters = Characters
services-model-gltf-binary = glTF binary
services-model-unknown = Unknown model format
services-model-no-triangles = The model has no triangles
services-model-bad-ascii-stl = Damaged ASCII STL
services-model-bad-stl = Damaged STL file
services-model-bad-gltf = Not a readable glTF file ({ $error })

## Archives and tags

services-archive-password = This archive is password protected
services-archive-wrong-password = Wrong password
# $error: details from the archive reader (English).
services-archive-corrupt = Corrupt archive: { $error }
# $format: archive format name, e.g. Rar.
services-archive-unsupported = { $format } archives are not supported
services-archive-open-failed = Couldn't open { $name }
services-archive-temp-failed = cannot create a temporary folder: { $error }
services-archive-unsafe-name = “{ $name }” is not a safe file name
services-tag-red = Red
services-tag-orange = Orange
services-tag-yellow = Yellow
services-tag-green = Green
services-tag-cyan = Cyan
services-tag-blue = Blue
services-tag-purple = Purple
services-tag-gray = Gray
services-tag-unknown = Unknown tag “{ $name }”.

## Index search

services-index-bad-size = Can't read the size “{ $size }”
services-index-size-range = The minimum size is larger than the maximum
services-index-empty-query = Type something to search for
services-index-no-folder = Choose a folder to search in
services-index-no-roots = No folders are indexed; add one in Settings → Preview & search

## Crash recovery and safe mode

services-crash-safe-mode = Safe mode
services-crash-safe-mode-body = Default settings, no session restore, no folder watching, thumbnails or git status. Restart Soyuz normally to leave safe mode.
services-crash-nothing-to-resume = Nothing was left to resume
services-crash-resuming = { $count ->
    [one] Resuming { $count } operation
   *[other] Resuming { $count } operations
}
services-crash-nothing-to-roll-back = Nothing had been completed, so there is nothing to roll back
services-crash-rolling-back = { $count ->
    [one] Rolling back { $count } operation
   *[other] Rolling back { $count } operations
}
services-crash-deleted = { $count ->
    [one] Deleted { $count } crash report
   *[other] Deleted { $count } crash reports
}
services-crash-restart-failed = Couldn't restart in safe mode

## OS clipboard (Linux / macOS)

services-clipboard-no-tool = neither wl-copy nor xclip is available

## Search

services-search-gone = This search is no longer available. Run it again from the search bar.
services-search-invalid-pattern = Invalid pattern: { $error }
services-search-mode-contains = Contains
services-search-mode-glob = Glob
services-search-mode-regex = Regex
services-search-mode-fuzzy = Fuzzy
services-search-date-any = Any time
services-search-date-day = Last 24 hours
services-search-date-week = Last 7 days
services-search-date-month = Last 30 days
services-search-date-year = Last 12 months
services-search-date-older = Older than a year
services-search-kind-any = Any kind
services-search-kind-files = Files
services-search-kind-folders = Folders
services-search-kind-images = Images
services-search-kind-videos = Videos
services-search-kind-audio = Audio
services-search-kind-documents = Documents
services-search-kind-archives = Archives
services-search-kind-code = Code
# Search tab titles. $text: the search text; $folder: the folder searched.
services-search-title-all-indexed = Everything in indexed folders
services-search-title-indexed = “{ $text }” in indexed folders
services-search-title-all = Everything in { $folder }
services-search-title = “{ $text }” in { $folder }

## Git status

services-git-ignored = Ignored
services-git-untracked = Untracked
services-git-type-changed = Type changed
services-git-renamed = Renamed
services-git-added = Added
services-git-deleted = Deleted
services-git-modified = Modified
services-git-conflict = Conflict
services-git-unavailable = git not available: { $error }
services-git-unusual-filter = refusing to run git status: unusual filter driver name
services-git-timeout = git status timed out
services-git-failed = git status failed

## Update check

services-update-no-repo = no repository configured
services-update-github-only = only github.com repositories are supported
services-update-expected-owner-name = expected owner/name
services-update-bad-repo = “{ $repo }” is not a GitHub owner/name
services-update-unexpected-reply = unexpected reply: { $error }
# $message: message from GitHub (English).
services-update-github-message = GitHub: { $message }
services-update-no-releases = the repository has no published releases
services-update-no-client = cannot build HTTP client: { $error }
services-update-network = network error: { $error }
services-update-reply-too-large = reply too large
services-update-not-found = no releases found (check the repository name)
# $status: HTTP status, e.g. "403 Forbidden".
services-update-http-status = GitHub replied { $status } { $message }
services-update-cannot-start = cannot start: { $error }
services-update-no-asset = this release has no download for this platform
# $code: Windows error code, e.g. 0x800B0100.
services-update-bad-signature = the installer's signature is invalid ({ $code }); the download was deleted
services-update-no-checksum = the release publishes no SHA-256 checksum for this file, so it cannot be verified
services-update-checksums-disagree = the release's checksums disagree with each other; not downloading
services-update-bad-asset-name = bad asset name
services-update-cannot-create = cannot create { $path }: { $error }
services-update-cannot-write-file = cannot write { $path }: { $error }
services-update-download-failed = download failed: { $status }
services-update-cancelled = download cancelled
services-update-too-large = download is unexpectedly large
services-update-cannot-write = cannot write the download: { $error }
# $expected, $got: first 12 hex digits of the SHA-256 hashes.
services-update-checksum-mismatch = checksum mismatch (expected { $expected }…, got { $got }…); the download was deleted
services-update-cannot-rename = cannot rename the download: { $error }
services-update-available = Soyuz { $version } is available
services-update-available-body = Open Settings → About & updates to read the release notes and download it.

## Linux (data)

services-watch-degraded = Change watching is limited
# $secs = poll interval in whole seconds.
services-watch-degraded-body = The system's limit of watched folders is reached, so some folders are re-checked every { $secs } s instead.

## Linux (platform)

services-clipboard-unavailable = the clipboard is not available (no Wayland data-control and no X11 display)
services-fm1-failed = Could not register "show in folder" (org.freedesktop.FileManager1): { $error }
services-fm1-taken = Another app ({ $owner }) already handles "show in folder" (org.freedesktop.FileManager1). Close it or turn off its file-manager service, then turn this setting off and on again.

# $kind: "system" | "flatpak" | "appimage" | "other".
services-update-package-managed = { $kind ->
    [system] This copy of Soyuz was installed by your system's package manager. Update it there (for example with dnf, apt or your software center); Soyuz never overwrites package-managed files.
    [flatpak] This copy of Soyuz runs as a Flatpak. Update it with “flatpak update” or your software center.
    [appimage] This copy of Soyuz runs as an AppImage. Download the new AppImage and replace the old file.
   *[other] This copy of Soyuz cannot update itself. Install the new version the same way you installed this one.
}
services-update-install-not-tarball = only the Linux release tarball (.tar.gz) can be installed in place
services-update-install-no-binary = the archive does not contain bin/{ $name }
services-update-install-bad-archive = the archive is damaged or not a Soyuz release
services-update-install-not-elf = bin/soyuz in the archive is not a program for this computer's processor; nothing was changed
services-update-install-lib-not-elf = lib/soyuz/{ $name } in the archive is not a library for this computer's processor; nothing was changed
services-update-install-changed = the downloaded file changed after it was verified; download it again
# $path: folder of the running soyuz executable.
services-update-install-no-permission = cannot write to { $path } ({ $error }). If Soyuz was installed system-wide, update it with sudo, e.g. “sudo tar -C /usr/local --strip-components=1 -xf <file>”.
services-update-install-failed = installing the update failed: { $error }
## FFmpeg runtime download (services::updates::ffmpeg_runtime)
services-ffmpeg-dl-flatpak = The Flatpak uses the FFmpeg of its runtime; there is nothing to download.
services-ffmpeg-dl-msix = The Microsoft Store / MSIX package carries the FFmpeg libraries; reinstall the package to repair them.
services-ffmpeg-dl-unsupported = No FFmpeg runtime is published for this system and processor. Install FFmpeg with your package manager and choose its folder in Settings → Preview → Media.
services-ffmpeg-dl-no-repo = No update repository is set (Settings → About & updates), so Soyuz does not go online.
services-ffmpeg-dl-busy = The FFmpeg runtime is already being downloaded.
services-ffmpeg-dl-no-release = The repository has no “{ $tag }” release.
services-ffmpeg-dl-no-asset = The “{ $tag }” release has no download for this system.
services-ffmpeg-dl-bad-archive = the downloaded archive is damaged ({ $error })
services-ffmpeg-dl-incomplete = the downloaded archive does not contain the five FFmpeg libraries and their licence; nothing was changed
services-ffmpeg-dl-in-use = cannot replace { $path } ({ $error }). If Soyuz loaded libraries from there, restart it and try again.
services-ffmpeg-dl-started = Downloading the FFmpeg runtime
services-ffmpeg-dl-started-body = It is verified and loaded when it arrives; you can keep working.
services-ffmpeg-dl-cannot = Cannot download the FFmpeg runtime
services-ffmpeg-dl-loaded = The FFmpeg libraries are already loaded
services-ffmpeg-dl-done = FFmpeg runtime installed
services-ffmpeg-dl-done-body = The libraries are in { $dir } and are loading now; video previews and the player work without a restart.
services-ffmpeg-dl-failed = The FFmpeg runtime could not be installed
services-ffmpeg-dl-cancelled = FFmpeg runtime download cancelled

## macos
services-clipboard-mac-failed = the files could not be put on the macOS pasteboard
