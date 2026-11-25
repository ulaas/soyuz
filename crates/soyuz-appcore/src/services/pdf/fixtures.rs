//! PDFs built in memory with lopdf for the engine tests.

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

/// One page's content: text lines `(x, y, size, text)` in PDF user space (y up) plus
/// raw extra operators appended after the text.
pub(crate) struct PageSpec {
    pub size: (i64, i64),
    pub rotate: i64,
    pub lines: Vec<(i64, i64, i64, String)>,
    pub extra: Vec<Operation>,
}

impl PageSpec {
    pub fn letter(lines: &[(i64, i64, i64, &str)]) -> PageSpec {
        PageSpec {
            size: (612, 792),
            rotate: 0,
            lines: lines.iter().map(|(x, y, s, t)| (*x, *y, *s, t.to_string())).collect(),
            extra: Vec::new(),
        }
    }
}

/// A document from `pages`; returns the document, its page ids and the catalog id so
/// callers can add outline / annotations before saving.
pub(crate) fn build(pages: Vec<PageSpec>) -> (Document, Vec<ObjectId>, ObjectId) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let mut kids = Vec::new();
    let mut ids = Vec::new();
    for spec in pages {
        let mut ops = Vec::new();
        for (x, y, size, text) in &spec.lines {
            ops.push(Operation::new("BT", vec![]));
            ops.push(Operation::new("Tf", vec!["F1".into(), (*size).into()]));
            ops.push(Operation::new("Td", vec![(*x).into(), (*y).into()]));
            ops.push(Operation::new("Tj", vec![Object::string_literal(text.as_str())]));
            ops.push(Operation::new("ET", vec![]));
        }
        ops.extend(spec.extra);
        let content = Content { operations: ops };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), spec.size.0.into(), spec.size.1.into()],
        };
        if spec.rotate != 0 {
            page.set("Rotate", spec.rotate);
        }
        let page_id = doc.add_object(page);
        kids.push(page_id.into());
        ids.push(page_id);
    }
    let count = kids.len() as i64;
    doc.objects
        .insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal("Fixture doc"),
        "Author" => Object::String(vec![0xFE, 0xFF, 0, b'A', 0, b'y'], lopdf::StringFormat::Hexadecimal),
        "CreationDate" => Object::string_literal("D:20240131120000Z"),
    });
    doc.trailer.set("Info", info_id);
    (doc, ids, catalog_id)
}

pub(crate) fn save(mut doc: Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// A filled rectangle `(x, y, w, h)` in user space, colour `rgb` 0–1.
pub(crate) fn rect_op(x: i64, y: i64, w: i64, h: i64, rgb: (f32, f32, f32)) -> Vec<Operation> {
    vec![
        Operation::new("rg", vec![rgb.0.into(), rgb.1.into(), rgb.2.into()]),
        Operation::new("re", vec![x.into(), y.into(), w.into(), h.into()]),
        Operation::new("f", vec![]),
    ]
}

/// Three Letter pages of Helvetica text; page 1 has two lines.
pub(crate) fn text_pages() -> Vec<u8> {
    let (doc, _, _) = build(vec![
        PageSpec::letter(&[(72, 700, 24, "Hello world"), (72, 650, 24, "again here, Hello")]),
        PageSpec::letter(&[(72, 700, 24, "Second page text")]),
        PageSpec::letter(&[(72, 700, 24, "Third page")]),
    ]);
    save(doc)
}

/// `n` Letter pages, each saying `Page <i> of the sweep`.
pub(crate) fn many_pages(n: usize) -> Vec<u8> {
    let specs = (0..n).map(|i| PageSpec::letter(&[(72, 700, 18, &format!("Page {i} of the sweep"))])).collect();
    save(build(specs).0)
}

/// Letter, landscape A4 and a 300 × 300 square.
pub(crate) fn mixed_sizes() -> Vec<u8> {
    let mut a = PageSpec::letter(&[(72, 700, 12, "Letter")]);
    a.size = (612, 792);
    let mut b = PageSpec::letter(&[(72, 500, 12, "Landscape")]);
    b.size = (842, 595);
    let mut c = PageSpec::letter(&[(20, 150, 12, "Square")]);
    c.size = (300, 300);
    save(build(vec![a, b, c]).0)
}

/// One Letter page with `/Rotate 90`, a blue block at user (100, 600, 200 × 100) and text.
pub(crate) fn rotated() -> Vec<u8> {
    let mut p = PageSpec::letter(&[(72, 400, 30, "Rotated text")]);
    p.rotate = 90;
    p.extra = rect_op(100, 600, 200, 100, (0.0, 0.0, 1.0));
    save(build(vec![p]).0)
}

/// Three pages with an outline (explicit, named and GoTo destinations, a URI entry and a
/// cycle back to the first item) and link annotations on page 0 (URI, explicit dest,
/// named dest, Launch and JavaScript actions that must be ignored).
pub(crate) fn with_nav() -> Vec<u8> {
    let (mut doc, ids, catalog) = build(vec![
        PageSpec::letter(&[(72, 700, 24, "Contents")]),
        PageSpec::letter(&[(72, 700, 24, "Chapter two")]),
        PageSpec::letter(&[(72, 700, 24, "Section")]),
    ]);
    let s = |t: &str| Object::string_literal(t);
    // Named destination "ch2" through the name tree.
    let ch2 = Object::Array(vec![ids[1].into(), "XYZ".into(), 0.into(), 792.into(), Object::Null]);
    let dests = doc.add_object(dictionary! { "Names" => vec![s("ch2"), ch2] });
    let names = doc.add_object(dictionary! { "Dests" => dests });

    let outlines = doc.new_object_id();
    let (o1, o2, o21, o3) = (doc.new_object_id(), doc.new_object_id(), doc.new_object_id(), doc.new_object_id());
    doc.objects.insert(
        o1,
        Object::Dictionary(dictionary! {
            "Title" => s("Chapter 1"), "Parent" => outlines, "Next" => o2,
            "Dest" => vec![ids[0].into(), "XYZ".into(), 72.into(), 720.into(), 0.into()],
        }),
    );
    doc.objects.insert(
        o2,
        Object::Dictionary(dictionary! {
            "Title" => s("Chapter 2"), "Parent" => outlines, "Prev" => o1, "Next" => o3,
            "Dest" => s("ch2"), "First" => o21, "Last" => o21, "Count" => 1,
        }),
    );
    doc.objects.insert(
        o21,
        Object::Dictionary(dictionary! {
            "Title" => Object::String(vec![0xFE, 0xFF, 0, b'S', 0, 0xE9, 0, b'c'], lopdf::StringFormat::Hexadecimal),
            "Parent" => o2,
            "A" => dictionary! { "S" => "GoTo", "D" => vec![ids[2].into(), "FitH".into(), 500.into()] },
        }),
    );
    doc.objects.insert(
        o3,
        Object::Dictionary(dictionary! {
            "Title" => s("Website"), "Parent" => outlines, "Prev" => o2,
            // A cycle: Next points back at the first item.
            "Next" => o1,
            "A" => dictionary! { "S" => "URI", "URI" => s("https://example.com/") },
        }),
    );
    doc.objects.insert(
        outlines,
        Object::Dictionary(dictionary! { "Type" => "Outlines", "First" => o1, "Last" => o3, "Count" => 3 }),
    );

    let link = |rect: [i64; 4], extra: lopdf::Dictionary| {
        let mut d = dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => rect.iter().map(|v| Object::from(*v)).collect::<Vec<_>>(),
        };
        for (k, v) in extra {
            d.set(k, v);
        }
        d
    };
    let annots = vec![
        Object::Dictionary(link(
            [72, 700, 272, 730],
            dictionary! { "A" => dictionary! { "S" => "URI", "URI" => s("https://example.com/docs") } },
        )),
        Object::Dictionary(link(
            [72, 600, 272, 630],
            dictionary! { "Dest" => vec![ids[2].into(), "XYZ".into(), 0.into(), 400.into(), Object::Null] },
        )),
        Object::Dictionary(link([72, 500, 272, 530], dictionary! { "Dest" => s("ch2") })),
        Object::Dictionary(link(
            [72, 400, 272, 430],
            dictionary! { "A" => dictionary! { "S" => "Launch", "F" => s("calc.exe") } },
        )),
        Object::Dictionary(link(
            [72, 300, 272, 330],
            dictionary! { "A" => dictionary! { "S" => "JavaScript", "JS" => s("app.alert(1)") } },
        )),
        Object::Dictionary(
            dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()] },
        ),
    ];
    if let Ok(page) = doc.get_object_mut(ids[0]).and_then(|o| o.as_dict_mut()) {
        page.set("Annots", annots);
    }
    if let Ok(cat) = doc.get_object_mut(catalog).and_then(|o| o.as_dict_mut()) {
        cat.set("Outlines", outlines);
        cat.set("Names", names);
    }
    save(doc)
}

/// One Letter page showing 日本語 in a non-embedded Adobe-Japan1 CID font
/// (`UniJIS-UCS2-H`), so it renders only through a system-font fallback.
pub(crate) fn cjk_unembedded() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "KozMinPr6N-Regular", "Flags" => 4,
        "FontBBox" => vec![0.into(), (-120).into(), 1000.into(), 880.into()],
        "ItalicAngle" => 0, "Ascent" => 880, "Descent" => -120, "CapHeight" => 700, "StemV" => 80,
    });
    let cid_font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "KozMinPr6N-Regular",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Japan1"), "Supplement" => 6,
        },
        "FontDescriptor" => descriptor, "DW" => 1000,
    });
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "KozMinPr6N-Regular-UniJIS-UCS2-H",
        "Encoding" => "UniJIS-UCS2-H", "DescendantFonts" => vec![cid_font.into()],
    });
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let text: Vec<u8> = "日本語".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 48.into()]),
            Operation::new("Td", vec![72.into(), 600.into()]),
            Operation::new("Tj", vec![Object::String(text, lopdf::StringFormat::Hexadecimal)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "Resources" => resources,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    save(doc)
}

/// The two-page "Hello from page one" document encrypted (RC4, 128-bit) with user
/// password `password` (owner password `owner`).
pub(crate) fn encrypted(password: &str) -> Vec<u8> {
    let (mut doc, _, _) = build(vec![
        PageSpec::letter(&[(72, 700, 24, "Hello from page one")]),
        PageSpec::letter(&[(72, 700, 24, "Second page text")]),
    ]);
    let id = || Object::String(b"0123456789abcdef".to_vec(), lopdf::StringFormat::Hexadecimal);
    doc.trailer.set("ID", Object::Array(vec![id(), id()]));
    let version = lopdf::EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: password,
        key_length: 128,
        permissions: lopdf::Permissions::all(),
    };
    let state = lopdf::EncryptionState::try_from(version).unwrap();
    doc.encrypt(&state).unwrap();
    save(doc)
}
