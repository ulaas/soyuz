//! Outline (bookmarks) and link annotations, read from the raw objects.
//!
//! Destinations: explicit arrays (`[page /XYZ left top zoom]`, `/Fit…`), names (name tree
//! `/Names /Dests` or the old catalog `/Dests`) and `GoTo` actions. Links carry a URI or
//! a destination; `Launch`, `JavaScript`, `GoToR` and other actions are ignored. Callers
//! open URIs only through [`is_web_uri`] (http, https, mailto). Caps: 64 outline levels,
//! [`MAX_NODES`] outline items, a cycle guard on `/Next` / `/First` chains.

use std::collections::{HashMap, HashSet};

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::Point;

use crate::services::preview_pdf::decode_pdf_string;

/// Deepest outline nesting read.
pub const MAX_DEPTH: usize = 64;
/// Most outline items read.
pub const MAX_NODES: usize = 20_000;
/// Most links read per page.
pub const MAX_LINKS: usize = 5_000;

/// A place in the document: page (0-based) and, when the PDF gives one, the point to put
/// at the top-left of the view, in rendered page points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dest {
    pub page: u32,
    pub top: Option<f32>,
    pub left: Option<f32>,
}

/// What a link does.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Dest(Dest),
    /// Any URI scheme as written; open only after [`is_web_uri`].
    Uri(String),
}

/// A link area (`[x0, y0, x1, y1]` in rendered page points) and its target.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

/// One outline entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub title: String,
    pub dest: Option<Dest>,
    pub uri: Option<String>,
    pub children: Vec<Outline>,
}

/// `http:`, `https:` or `mailto:` (the only links the viewer opens).
pub fn is_web_uri(uri: &str) -> bool {
    let u = uri.trim().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")
}

/// Reads destinations of one document.
struct Resolver<'a> {
    pdf: &'a Pdf,
    /// Page object → page index.
    pages: HashMap<ObjectIdentifier, u32>,
    catalog: Option<Dict<'a>>,
}

impl<'a> Resolver<'a> {
    fn new(pdf: &'a Pdf) -> Resolver<'a> {
        let pages = pdf.pages().iter().enumerate().filter_map(|(i, p)| Some((p.raw().obj_id()?, i as u32))).collect();
        let xref = pdf.xref();
        Resolver { pdf, pages, catalog: xref.get::<Dict<'a>>(xref.root_id()) }
    }

    fn page(&self, i: u32) -> Option<&'a Page<'a>> {
        self.pdf.pages().get(i as usize)
    }

    /// A destination object: array, `<< /D array >>`, or a name / string to look up.
    fn dest(&self, obj: Object<'a>, depth: usize) -> Option<Dest> {
        match obj {
            Object::Array(a) => self.explicit(&a),
            Object::Dict(d) => self.dest(d.get::<Object<'a>>("D")?, depth),
            Object::Name(n) if depth < 4 => self.named(n.as_ref(), depth),
            Object::String(s) if depth < 4 => self.named(s.as_bytes(), depth),
            _ => None,
        }
    }

    fn explicit(&self, a: &Array<'a>) -> Option<Dest> {
        let mut it = a.raw_iter();
        let page = match it.next()? {
            MaybeRef::Ref(r) => *self.pages.get(&r.into())?,
            MaybeRef::NotRef(Object::Number(n)) => u32::try_from(n.as_i64()).ok()?,
            MaybeRef::NotRef(Object::Dict(d)) => *self.pages.get(&d.obj_id()?)?,
            _ => return None,
        };
        let kind = match it.next() {
            Some(MaybeRef::NotRef(Object::Name(n))) => n.as_str().to_string(),
            _ => String::new(),
        };
        let nums: Vec<Option<f32>> = it
            .map(|o| match o {
                MaybeRef::NotRef(Object::Number(n)) => Some(n.as_f32()),
                _ => None,
            })
            .collect();
        let at = |i: usize| nums.get(i).copied().flatten();
        let (left, top) = match kind.as_str() {
            "XYZ" => (at(0), at(1)),
            "FitH" | "FitBH" => (None, at(0)),
            "FitV" | "FitBV" => (at(0), None),
            "FitR" => (at(0), at(3)),
            _ => (None, None),
        };
        if page as usize >= self.pdf.pages().len() {
            return None;
        }
        let mut d = Dest { page, top: None, left: None };
        if (left.is_some() || top.is_some())
            && let Some(pg) = self.page(page)
        {
            let crop = pg.intersected_crop_box();
            let user = Point::new(left.map_or(crop.x0, f64::from), top.map_or(crop.y1, f64::from));
            let p = pg.initial_transform(true).to_kurbo() * user;
            d.top = Some(p.y as f32);
            d.left = Some(p.x as f32);
        }
        Some(d)
    }

    /// Look `name` up in the name tree (`/Names /Dests`), then in the catalog `/Dests`.
    fn named(&self, name: &[u8], depth: usize) -> Option<Dest> {
        let cat = self.catalog.as_ref()?;
        if let Some(tree) = cat.get::<Dict<'a>>("Names").and_then(|n| n.get::<Dict<'a>>("Dests"))
            && let Some(obj) = name_tree_get(&tree, name)
        {
            return self.dest(obj, depth + 1);
        }
        let old = cat.get::<Dict<'a>>("Dests")?;
        self.dest(old.get::<Object<'a>>(name)?, depth + 1)
    }

    /// Target of an outline item or link: `/Dest` or a `/GoTo` / `/URI` action.
    fn target(&self, d: &Dict<'a>) -> (Option<Dest>, Option<String>) {
        if let Some(obj) = d.get::<Object<'a>>("Dest") {
            return (self.dest(obj, 0), None);
        }
        let Some(a) = d.get::<Dict<'a>>("A") else { return (None, None) };
        match a.get::<Name<'_>>("S").as_ref().map(|n| n.as_str()) {
            Some("GoTo") => (a.get::<Object<'a>>("D").and_then(|o| self.dest(o, 0)), None),
            Some("URI") => {
                let uri = a.get::<hayro::hayro_syntax::object::String<'_>>("URI").map(|s| {
                    // URIs are 7-bit ASCII by the spec; be lenient with UTF-8.
                    String::from_utf8_lossy(s.as_bytes()).trim().to_string()
                });
                (None, uri.filter(|u| !u.is_empty()))
            }
            _ => (None, None),
        }
    }
}

/// Look `key` up in a name tree, depth- and node-capped.
fn name_tree_get<'a>(root: &Dict<'a>, key: &[u8]) -> Option<Object<'a>> {
    let mut stack = vec![(root.clone(), 0usize)];
    let mut seen = 0usize;
    while let Some((node, depth)) = stack.pop() {
        seen += 1;
        if seen > MAX_NODES || depth > MAX_DEPTH {
            return None;
        }
        if let Some(names) = node.get::<Array<'a>>("Names") {
            let mut it = names.iter::<Object<'a>>();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                let kb: Option<Vec<u8>> = match &k {
                    Object::String(s) => Some(s.as_bytes().to_vec()),
                    Object::Name(n) => Some(n.as_ref().to_vec()),
                    _ => None,
                };
                if kb.as_deref() == Some(key) {
                    return Some(v);
                }
            }
        }
        if let Some(kids) = node.get::<Array<'a>>("Kids") {
            for kid in kids.iter::<Dict<'a>>() {
                // Skip subtrees whose /Limits exclude the key.
                if let Some(lim) = kid.get::<Array<'a>>("Limits") {
                    let l: Vec<Vec<u8>> =
                        lim.iter::<hayro::hayro_syntax::object::String<'_>>().map(|s| s.as_bytes().to_vec()).collect();
                    if l.len() == 2 && (key < l[0].as_slice() || key > l[1].as_slice()) {
                        continue;
                    }
                }
                stack.push((kid, depth + 1));
            }
        }
    }
    None
}

/// The document outline (empty without one).
pub(crate) fn outline(pdf: &Pdf) -> Vec<Outline> {
    let r = Resolver::new(pdf);
    let Some(root) = r.catalog.as_ref().and_then(|c| c.get::<Dict<'_>>("Outlines")) else { return Vec::new() };
    let mut seen = HashSet::new();
    let mut count = 0usize;
    items(&r, &root, 0, &mut seen, &mut count)
}

fn items<'a>(
    r: &Resolver<'a>,
    parent: &Dict<'a>,
    depth: usize,
    seen: &mut HashSet<ObjectIdentifier>,
    count: &mut usize,
) -> Vec<Outline> {
    let mut out = Vec::new();
    if depth >= MAX_DEPTH {
        return out;
    }
    let xref = r.pdf.xref();
    let mut next = parent.get_ref("First");
    while let Some(id) = next {
        let id: ObjectIdentifier = id.into();
        if *count >= MAX_NODES || !seen.insert(id) {
            break;
        }
        *count += 1;
        let Some(item) = xref.get::<Dict<'a>>(id) else { break };
        let title = item
            .get::<hayro::hayro_syntax::object::String<'_>>("Title")
            .map(|s| decode_pdf_string(s.as_bytes()))
            .unwrap_or_default();
        let title: String = title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let (dest, uri) = r.target(&item);
        let children = items(r, &item, depth + 1, seen, count);
        out.push(Outline { title: title.trim().to_string(), dest, uri, children });
        next = item.get_ref("Next");
    }
    out
}

/// Links of every page, in page order.
pub(crate) fn links(pdf: &Pdf) -> Vec<Vec<Link>> {
    let r = Resolver::new(pdf);
    pdf.pages().iter().map(|p| page_links(&r, p)).collect()
}

fn page_links<'a>(r: &Resolver<'a>, page: &Page<'a>) -> Vec<Link> {
    let Some(annots) = page.raw().get::<Array<'a>>("Annots") else { return Vec::new() };
    let t = page.initial_transform(true).to_kurbo();
    let mut out = Vec::new();
    for a in annots.iter::<Dict<'a>>().take(MAX_LINKS) {
        if a.get::<Name<'_>>("Subtype").as_ref().map(|n| n.as_str()) != Some("Link") {
            continue;
        }
        let Some(rect) = a.get::<[f32; 4]>("Rect") else { continue };
        let target = match r.target(&a) {
            (Some(d), _) => LinkTarget::Dest(d),
            (None, Some(u)) => LinkTarget::Uri(u),
            _ => continue,
        };
        let corners = [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])]
            .map(|(x, y)| t * Point::new(f64::from(x), f64::from(y)));
        let xs = corners.map(|p| p.x as f32);
        let ys = corners.map(|p| p.y as f32);
        let min = |v: [f32; 4]| v.iter().copied().fold(f32::INFINITY, f32::min);
        let max = |v: [f32; 4]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        out.push(Link { rect: [min(xs), min(ys), max(xs), max(ys)], target });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{fixtures, hayro_backend};
    use super::*;

    #[test]
    fn outline_with_named_goto_uri_and_a_cycle() {
        let pdf = hayro_backend::load(Arc::from(fixtures::with_nav()), None).unwrap();
        let o = outline(&pdf);
        let titles: Vec<&str> = o.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Chapter 1", "Chapter 2", "Website"], "the cycle back to item 1 stops the walk");
        assert_eq!(o[0].dest, Some(Dest { page: 0, top: Some(72.0), left: Some(72.0) }));
        assert_eq!(o[1].dest.map(|d| d.page), Some(1), "named destination");
        assert_eq!(o[1].children.len(), 1);
        assert_eq!(o[1].children[0].title, "Séc");
        assert_eq!(o[1].children[0].dest, Some(Dest { page: 2, top: Some(292.0), left: Some(0.0) }), "GoTo FitH");
        assert_eq!(o[2].uri.as_deref(), Some("https://example.com/"));
        assert!(outline(&hayro_backend::load(Arc::from(fixtures::text_pages()), None).unwrap()).is_empty());
    }

    #[test]
    fn links_uri_dest_named_and_ignored_actions() {
        let pdf = hayro_backend::load(Arc::from(fixtures::with_nav()), None).unwrap();
        let all = links(&pdf);
        assert_eq!(all.len(), 3);
        assert!(all[1].is_empty() && all[2].is_empty());
        let l = &all[0];
        assert_eq!(l.len(), 3, "Launch, JavaScript and the text note are dropped: {l:?}");
        assert_eq!(l[0].target, LinkTarget::Uri("https://example.com/docs".into()));
        assert_eq!(l[0].rect, [72.0, 62.0, 272.0, 92.0], "flipped to top-left origin");
        assert_eq!(l[1].target, LinkTarget::Dest(Dest { page: 2, top: Some(392.0), left: Some(0.0) }));
        assert!(matches!(l[2].target, LinkTarget::Dest(Dest { page: 1, .. })));
        assert!(
            is_web_uri("HTTPS://x")
                && is_web_uri("mailto:a@b")
                && !is_web_uri("file:///c:/x")
                && !is_web_uri("javascript:x")
        );
    }
}
