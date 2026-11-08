//! Windows packaged (Store / MSIX) applications for the palette's app launcher, without
//! the shell's app list: the packages registered for this user (the AppModel repository in
//! the registry), the apps each package's `AppxManifest.xml` declares (those hidden from
//! Start left out), and their names from the resource-string cache Windows keeps for them
//! (`MrtCache`: the strings the Start menu and Explorer resolved). Apps whose name cannot
//! be found there are skipped. The parsing is plain code, tested on every OS.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

/// One app declared in a package manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManifestApp {
    pub id: String,
    /// Raw `DisplayName` (plain text or an `ms-resource:` reference).
    pub display_name: String,
    pub description: String,
    /// `AppListEntry="none"`: not shown in Start.
    pub hidden: bool,
    /// `Square44x44Logo` (the app-list icon), else `Square150x150Logo`, relative to the
    /// package root, without its scale / size qualifiers.
    pub logo: String,
}

/// `Name_Version_Arch_ResourceId_PublisherId` split into (name, version, family name).
pub(crate) fn split_full_name(full: &str) -> Option<(&str, Vec<u32>, String)> {
    let parts: Vec<&str> = full.split('_').collect();
    if parts.len() != 5 || parts[0].is_empty() || parts[4].is_empty() {
        return None;
    }
    let version = parts[1].split('.').map(|n| n.parse().unwrap_or(0)).collect();
    Some((parts[0], version, format!("{}_{}", parts[0], parts[4])))
}

/// An application user model id we pass on: `family!app`, letters, digits, `.`, `-`, `_`.
pub(crate) fn valid_aumid(aumid: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    aumid.split_once('!').is_some_and(|(f, a)| ok(f) && ok(a))
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn unescape(v: &str) -> String {
    v.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The `<Application>` elements of a manifest with their `VisualElements`.
pub(crate) fn manifest_apps(xml: &str) -> Vec<ManifestApp> {
    let mut r = Reader::from_str(xml);
    let mut out: Vec<ManifestApp> = Vec::new();
    let mut in_app = false;
    loop {
        let (e, empty) = match r.read_event() {
            Ok(Event::Start(e)) => (e, false),
            Ok(Event::Empty(e)) => (e, true),
            Ok(Event::End(e)) => {
                if local(e.name().as_ref()) == "Application" {
                    in_app = false;
                }
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => continue,
        };
        let attrs: Vec<(String, String)> =
            e.attributes().flatten().map(|a| (local(a.key.as_ref()).to_string(), unescape(&a.value))).collect();
        let attr = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        match local(e.name().as_ref()) {
            "Application" => {
                out.push(ManifestApp { id: attr("Id").unwrap_or_default(), ..Default::default() });
                in_app = !empty;
            }
            "VisualElements" if in_app => {
                if let Some(app) = out.last_mut() {
                    app.display_name = attr("DisplayName").unwrap_or_default();
                    app.description = attr("Description").unwrap_or_default();
                    app.hidden = attr("AppListEntry").is_some_and(|v| v.eq_ignore_ascii_case("none"));
                    app.logo = attr("Square44x44Logo").or_else(|| attr("Square150x150Logo")).unwrap_or_default();
                }
            }
            _ => {}
        }
    }
    out.retain(|a| !a.id.is_empty());
    out
}

/// The best file for logo `stem.ext` among a folder's `names`: the qualified variants
/// Windows picks from (`AppList.targetsize-48_altform-unplated.png`, `AppList.scale-200.png`,
/// …). Plain (unplated) target sizes near 48 px first, then plated ones, then the largest
/// scale, then the file without qualifiers; contrast variants never.
pub(crate) fn pick_logo(names: &[String], stem: &str, ext: &str) -> Option<String> {
    let (stem, ext) = (stem.to_lowercase(), ext.to_lowercase());
    let mut best: Option<(i64, &String)> = None;
    for n in names {
        let lower = n.to_lowercase();
        let quals = if lower == format!("{stem}.{ext}") {
            String::new()
        } else {
            match lower.strip_prefix(&format!("{stem}.")).and_then(|r| r.strip_suffix(&format!(".{ext}"))) {
                Some(q) => q.to_string(),
                None => continue,
            }
        };
        if quals.contains("contrast-") {
            continue;
        }
        let mut score = 0i64;
        let mut size = None;
        let mut scale = None;
        for q in quals.split(['_', '.']).filter(|q| !q.is_empty()) {
            if let Some(v) = q.strip_prefix("targetsize-") {
                size = v.parse::<i64>().ok();
            } else if let Some(v) = q.strip_prefix("scale-") {
                scale = v.parse::<i64>().ok();
            } else if q == "altform-unplated" {
                score += 50;
            } else if q.starts_with("altform-") {
                score -= 20;
            }
        }
        score += match (size, scale) {
            // 48 px and a little more first, much larger next, small ones last.
            (Some(s), _) if s >= 48 => 1000 - (s - 48).min(400),
            (Some(s), _) => 500 + s,
            (None, Some(sc)) => 300 + sc.min(400) / 4,
            (None, None) => 100,
        };
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, n));
        }
    }
    best.map(|(_, n)| n.clone())
}

/// The `ms-resource://` URIs an `ms-resource:` reference of package `name` may stand for:
/// a bare name lives in `Resources/`, a path is taken from the package root.
pub(crate) fn resource_uris(name: &str, reference: &str) -> Vec<String> {
    let Some(r) = reference.strip_prefix("ms-resource:") else { return Vec::new() };
    if r.starts_with("//") {
        return vec![format!("ms-resource:{r}")];
    }
    if let Some(abs) = r.strip_prefix('/') {
        return vec![format!("ms-resource://{name}/{abs}")];
    }
    let mut v = vec![format!("ms-resource://{name}/Resources/{r}")];
    if r.contains('/') {
        v.insert(0, format!("ms-resource://{name}/{r}"));
    }
    v
}

/// A cache key as Windows writes it, normalized for lookups (`@{full?uri}`, lower case, no
/// space after the `?`).
pub(crate) fn cache_key(raw: &str) -> String {
    match raw.split_once('?') {
        Some((a, b)) => format!("{a}?{}", b.trim_start()).to_lowercase(),
        None => raw.to_lowercase(),
    }
}

/// The text `value` (plain or `ms-resource:`) of package `full` stands for, from `cache`
/// (keys normalized by [`cache_key`]); `None` when an `ms-resource:` reference is not there.
pub(crate) fn resolve(full: &str, value: &str, cache: &HashMap<String, String>) -> Option<String> {
    let value = value.trim();
    if !value.starts_with("ms-resource:") {
        return (!value.is_empty()).then(|| value.to_string());
    }
    let (name, _, _) = split_full_name(full)?;
    resource_uris(name, value)
        .iter()
        .find_map(|uri| cache.get(&cache_key(&format!("@{{{full}?{uri}}}"))))
        .filter(|s| !s.trim().is_empty())
        .cloned()
}

#[cfg(windows)]
pub(crate) use imp::scan;

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::launch::InstalledApp;
    use crate::sys::windows::registry::{self as wr, Hive};

    const PACKAGES: &str =
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    const MRT_CACHE: &str = r"Software\Classes\Local Settings\MrtCache";

    /// Every resolved string of the cache. A string cached for several languages or scales
    /// is taken from the most recently written context (the one in use).
    fn mrt_cache() -> HashMap<String, String> {
        let mut best: HashMap<String, (u64, String)> = HashMap::new();
        for pri in wr::subkeys(Hive::CurrentUser, MRT_CACHE) {
            let pri_key = format!(r"{MRT_CACHE}\{pri}");
            for a in wr::subkeys(Hive::CurrentUser, &pri_key) {
                let a_key = format!(r"{pri_key}\{a}");
                for b in wr::subkeys(Hive::CurrentUser, &a_key) {
                    let key = format!(r"{a_key}\{b}");
                    let t = wr::last_write(Hive::CurrentUser, &key).unwrap_or(0);
                    for (name, value) in wr::string_values(Hive::CurrentUser, &key) {
                        if !name.starts_with("@{") {
                            continue;
                        }
                        let k = cache_key(&name);
                        if best.get(&k).is_none_or(|(bt, _)| t >= *bt) {
                            best.insert(k, (t, value));
                        }
                    }
                }
            }
        }
        best.into_iter().map(|(k, (_, v))| (k, v)).collect()
    }

    /// The file names in `dir`.
    fn names_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    /// The best file for an app's logo (`Assets\AppList.png`): a variant next to where the
    /// manifest says, else, for logos kept in a resource package, next to the file Windows
    /// last resolved for it.
    fn logo_path(root: &Path, full: &str, logo: &str, cache: &HashMap<String, String>) -> Option<PathBuf> {
        let rel = PathBuf::from(logo.replace('/', "\\"));
        let stem = rel.file_stem()?.to_string_lossy().into_owned();
        let ext = rel.extension()?.to_string_lossy().into_owned();
        let pick = |dir: &Path| pick_logo(&names_in(dir), &stem, &ext).map(|n| dir.join(n));
        let dir = root.join(rel.parent().unwrap_or(Path::new("")));
        if let Some(p) = pick(&dir) {
            return Some(p);
        }
        let (name, _, _) = split_full_name(full)?;
        let resolved = [logo.replace('/', "\\"), logo.replace('\\', "/")]
            .iter()
            .find_map(|l| cache.get(&cache_key(&format!("@{{{full}?ms-resource://{name}/Files/{l}}}"))))
            .map(PathBuf::from)?;
        resolved.parent().and_then(pick).or_else(|| resolved.is_file().then_some(resolved))
    }

    /// The Start-visible apps of the packages registered for this user; for a family
    /// registered in several versions, the newest.
    pub(crate) fn scan() -> Vec<InstalledApp> {
        let mut newest: HashMap<String, (Vec<u32>, String, PathBuf)> = HashMap::new();
        for full in wr::subkeys(Hive::CurrentUser, PACKAGES) {
            let Some((_, version, family)) = split_full_name(&full) else { continue };
            let Some(root) =
                wr::get_string(Hive::CurrentUser, &format!(r"{PACKAGES}\{full}"), Some("PackageRootFolder"))
            else {
                continue;
            };
            if newest.get(&family).is_none_or(|(v, _, _)| version > *v) {
                newest.insert(family, (version, full, PathBuf::from(root)));
            }
        }
        let mut cache: Option<HashMap<String, String>> = None;
        let mut out = Vec::new();
        for (family, (_, full, root)) in newest {
            let Ok(xml) = std::fs::read_to_string(root.join("AppxManifest.xml")) else { continue };
            for app in manifest_apps(&xml).into_iter().filter(|a| !a.hidden) {
                let aumid = format!("{family}!{}", app.id);
                if !valid_aumid(&aumid) {
                    continue;
                }
                let cache = cache.get_or_insert_with(mrt_cache);
                let Some(name) = resolve(&full, &app.display_name, cache) else {
                    log::debug!("store app {aumid}: no cached name for {}", app.display_name);
                    continue;
                };
                let description = resolve(&full, &app.description, cache)
                    .filter(|d| !d.trim().eq_ignore_ascii_case(name.trim()))
                    .unwrap_or_default();
                let package = split_full_name(&full).map(|(n, _, _)| n.to_string()).unwrap_or_default();
                out.push(InstalledApp {
                    id: aumid.clone(),
                    name,
                    detail: description.clone(),
                    keywords: format!("{package} {description}"),
                    source: root.clone(),
                    icon: logo_path(&root, &full, &app.logo, cache),
                    aumid: Some(aumid),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10">
  <Identity Name="Microsoft.Paint" Version="11.2609.401.0" />
  <Properties><DisplayName>ms-resource:AppStoreName</DisplayName></Properties>
  <Applications>
    <Application Id="App" Executable="PaintApp\mspaint.exe">
      <uap:VisualElements DisplayName="ms-resource:AppDisplayName" Description="ms-resource:AppDescription"
                          Square150x150Logo="a.png" />
    </Application>
    <Application Id="Helper">
      <uap:VisualElements DisplayName="Tom &amp; Jerry" Description="Plain" AppListEntry="none" />
    </Application>
    <Application Id="NoVisuals" />
  </Applications>
</Package>"#;

    #[test]
    fn manifest_apps_are_read() {
        let apps = manifest_apps(MANIFEST);
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].id, "App");
        assert_eq!(apps[0].display_name, "ms-resource:AppDisplayName");
        assert!(!apps[0].hidden);
        assert_eq!(apps[1].display_name, "Tom & Jerry");
        assert!(apps[1].hidden);
        // An empty <Application/> must not take the next app's visual elements.
        assert_eq!(apps[2].id, "NoVisuals");
        assert!(apps[2].display_name.is_empty());
        assert!(manifest_apps("<not xml").is_empty());
    }

    #[test]
    fn full_names_split_into_family_and_version() {
        let (name, v, family) = split_full_name("Microsoft.Paint_11.2609.401.0_x64__8wekyb3d8bbwe").unwrap();
        assert_eq!(name, "Microsoft.Paint");
        assert_eq!(v, [11, 2609, 401, 0]);
        assert_eq!(family, "Microsoft.Paint_8wekyb3d8bbwe");
        assert!(split_full_name("NotAPackage").is_none());
        assert!(v > vec![11, 2504, 531, 0]);
    }

    #[test]
    fn logos_pick_the_best_variant() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = names(&[
            "AppList.png",
            "AppList.scale-100.png",
            "AppList.scale-200.png",
            "AppList.targetsize-16.png",
            "AppList.targetsize-48.png",
            "AppList.targetsize-48_altform-unplated.png",
            "AppList.targetsize-48_altform-lightunplated.png",
            "AppList.targetsize-256_altform-unplated.png",
            "AppList.scale-200_contrast-black.png",
            "AppListOther.png",
            "Wide.scale-200.png",
        ]);
        assert_eq!(pick_logo(&all, "AppList", "png").as_deref(), Some("AppList.targetsize-48_altform-unplated.png"));
        let scales = names(&["applist.scale-100.png", "AppList.scale-200.PNG", "AppList.png"]);
        assert_eq!(pick_logo(&scales, "AppList", "png").as_deref(), Some("AppList.scale-200.PNG"));
        assert_eq!(pick_logo(&names(&["AppList.png"]), "AppList", "png").as_deref(), Some("AppList.png"));
        assert_eq!(pick_logo(&names(&["AppList.scale-200_contrast-white.png", "Other.png"]), "AppList", "png"), None);
        assert_eq!(manifest_apps(MANIFEST)[0].logo, "a.png");
    }

    #[test]
    fn aumids_are_checked() {
        assert!(valid_aumid("Microsoft.Paint_8wekyb3d8bbwe!App"));
        assert!(!valid_aumid("Microsoft.Paint_8wekyb3d8bbwe"));
        assert!(!valid_aumid("a!b c"));
        assert!(!valid_aumid("a!b\\..\\c"));
        assert!(!valid_aumid("!App"));
    }

    #[test]
    fn resource_references_resolve_from_the_cache() {
        let full = "Microsoft.Paint_11.2609.401.0_x64__8wekyb3d8bbwe";
        let mut cache = HashMap::new();
        cache.insert(
            cache_key(&format!("@{{{full}? ms-resource://Microsoft.Paint/Resources/AppDisplayName}}")),
            "Paint".to_string(),
        );
        cache.insert(cache_key(&format!("@{{{full}?ms-resource://Microsoft.Paint/Strings/Tool}}")), "Tool".to_string());
        assert_eq!(resolve(full, "ms-resource:AppDisplayName", &cache).as_deref(), Some("Paint"));
        assert_eq!(resolve(full, "ms-resource:/Resources/AppDisplayName", &cache).as_deref(), Some("Paint"));
        assert_eq!(
            resolve(full, "ms-resource://Microsoft.Paint/Resources/AppDisplayName", &cache).as_deref(),
            Some("Paint")
        );
        assert_eq!(resolve(full, "ms-resource:Strings/Tool", &cache).as_deref(), Some("Tool"));
        assert_eq!(resolve(full, "ms-resource:Missing", &cache), None);
        assert_eq!(resolve(full, "Plain Name", &cache).as_deref(), Some("Plain Name"));
        assert_eq!(resolve(full, "  ", &cache), None);
    }
}
