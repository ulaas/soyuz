### Soyuz UI strings (en-US source): pdf
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/pdf.ftl.


## The PDF viewer (editors::pdf)

# $page: page number (1-based), $pages: page count.
pdf-page-of = Page { $page } of { $pages }
# $percent: zoom in percent, without the % sign.
pdf-zoom-percent = { $percent } %
pdf-link-not-web = Only web and e-mail links can be opened
pdf-no-document = Open a PDF in the viewer first


## The PDF viewer's interface (soyuz-app ui::editors::pdf)

pdf-ui-sidebar = Sidebar
pdf-ui-prev-page = Previous page
pdf-ui-next-page = Next page
# The page field on the toolbar. $page: current page (1-based), $pages: page count.
pdf-ui-page-count = { $page } / { $pages }
pdf-ui-goto-hint = Page
pdf-ui-goto = Go to page…
pdf-ui-zoom = Zoom
pdf-ui-zoom-out = Zoom out
pdf-ui-zoom-in = Zoom in
pdf-ui-fit-width = Fit width
pdf-ui-fit-page = Fit page
pdf-ui-actual-size = Actual size
pdf-ui-rotate-cw = Rotate clockwise
pdf-ui-rotate-ccw = Rotate counterclockwise
pdf-ui-dark-pages = Dark pages
pdf-ui-find = Find
pdf-ui-more = More
pdf-ui-copy = Copy
pdf-ui-select-all = Select all
pdf-ui-copy-page = Copy page as text
pdf-ui-find-selection = Find selection
pdf-ui-thumbnails = Thumbnails
pdf-ui-outline = Outline
pdf-ui-no-outline = No outline
pdf-ui-no-outline-body = This document has no bookmarks.
pdf-ui-find-hint = Find in document
# $current: number of the current match, $count: matches found so far.
pdf-ui-find-of = { $current } of { $count }
# $count: matches found so far.
pdf-ui-find-count = { $count ->
    [one] 1 match
   *[other] { $count } matches
}
pdf-ui-find-none = No matches
# $width, $height: page size in millimetres.
pdf-ui-status-size = { $width } × { $height } mm
# $count: selected characters.
pdf-ui-status-selected = { $count ->
    [one] 1 character selected
   *[other] { $count } characters selected
}
pdf-ui-locked = This PDF is password protected
pdf-ui-locked-body = Type its password to open it.
pdf-ui-locked-wrong = That password did not open it. Try again.
pdf-ui-unlock = Enter password
pdf-ui-failed = The PDF could not be opened
pdf-ui-page-failed = This page could not be drawn
pdf-ui-retry = Retry
# $url: the web address a link in the document points to.
pdf-ui-link-ask = Open { $url }?
pdf-ui-link-open = Open link
# Accessible name of the page area. $name: file name.
pdf-ui-document = PDF document { $name }
pdf-ui-open-in-viewer = Open in viewer
