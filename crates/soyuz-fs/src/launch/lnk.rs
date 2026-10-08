//! MS-SHLLINK (`.lnk`) parser: header, LinkInfo (`LocalBasePath` / `CommonPathSuffix`,
//! network share), StringData (arguments, working dir, relative path) and the
//! `EnvironmentVariableDataBlock`. Pure byte parsing; compiled and tested on every OS.

use std::io;
use std::path::{Path, PathBuf};

const HEADER_SIZE: u32 = 0x4C;
const LINK_CLSID: [u8; 16] =
    [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

const HAS_LINK_TARGET_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;

const ENVIRONMENT_VARIABLE_DATA_BLOCK: u32 = 0xA000_0001;

/// What a shortcut points at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shortcut {
    /// Absolute target from LinkInfo (local or `\\server\share\...`) or the environment block.
    pub target: Option<String>,
    pub name: Option<String>,
    pub relative_path: Option<String>,
    pub working_dir: Option<String>,
    pub arguments: Option<String>,
    pub icon: Option<String>,
    /// `%VAR%`-style target from the EnvironmentVariableDataBlock (unexpanded).
    pub env_target: Option<String>,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("not a valid shortcut: {msg}"))
}

fn u16_at(b: &[u8], o: usize) -> io::Result<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| bad("truncated"))
}

fn u32_at(b: &[u8], o: usize) -> io::Result<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| bad("truncated"))
}

/// NUL-terminated code-page string (decoded as Latin-1, exact for ASCII paths).
fn ansi_z(b: &[u8], o: usize) -> Option<String> {
    let s = b.get(o..)?;
    let end = s.iter().position(|&c| c == 0)?;
    Some(s[..end].iter().map(|&c| c as char).collect())
}

/// NUL-terminated UTF-16LE string.
fn utf16_z(b: &[u8], o: usize) -> Option<String> {
    let s = b.get(o..)?;
    let units: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&units))
}

/// Parse a `.lnk` file's bytes.
pub(crate) fn parse(b: &[u8]) -> io::Result<Shortcut> {
    if u32_at(b, 0)? != HEADER_SIZE || b.get(4..20) != Some(&LINK_CLSID[..]) {
        return Err(bad("wrong header"));
    }
    let flags = u32_at(b, 0x14)?;
    let mut pos = HEADER_SIZE as usize;
    let mut out = Shortcut::default();

    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        pos += 2 + u16_at(b, pos)? as usize;
    }
    if flags & HAS_LINK_INFO != 0 {
        let li = pos;
        let size = u32_at(b, li)? as usize;
        let header_size = u32_at(b, li + 4)? as usize;
        let li_flags = u32_at(b, li + 8)?;
        let lbp_off = u32_at(b, li + 16)? as usize;
        let cnrl_off = u32_at(b, li + 20)? as usize;
        let cps_off = u32_at(b, li + 24)? as usize;
        let (lbp_uni, cps_uni) =
            if header_size >= 0x24 { (u32_at(b, li + 28)? as usize, u32_at(b, li + 32)? as usize) } else { (0, 0) };
        let suffix = if cps_uni != 0 { utf16_z(b, li + cps_uni) } else { ansi_z(b, li + cps_off) }.unwrap_or_default();
        if li_flags & 1 != 0 {
            let base = if lbp_uni != 0 { utf16_z(b, li + lbp_uni) } else { ansi_z(b, li + lbp_off) };
            if let Some(base) = base.filter(|s| !s.is_empty()) {
                out.target = Some(join_win(&base, &suffix));
            }
        } else if li_flags & 2 != 0 && cnrl_off != 0 {
            let c = li + cnrl_off;
            let net_off = u32_at(b, c + 8)? as usize;
            let net = if net_off > 0x14 {
                let uni = u32_at(b, c + 20)? as usize;
                utf16_z(b, c + uni)
            } else {
                ansi_z(b, c + net_off)
            };
            if let Some(net) = net.filter(|s| !s.is_empty()) {
                out.target = Some(join_win(&net, &suffix));
            }
        }
        if size == 0 {
            return Err(bad("empty LinkInfo"));
        }
        pos = li + size;
    }

    for (flag, slot) in [
        (HAS_NAME, &mut out.name),
        (HAS_RELATIVE_PATH, &mut out.relative_path),
        (HAS_WORKING_DIR, &mut out.working_dir),
        (HAS_ARGUMENTS, &mut out.arguments),
        (HAS_ICON_LOCATION, &mut out.icon),
    ] {
        if flags & flag == 0 {
            continue;
        }
        let count = u16_at(b, pos)? as usize;
        pos += 2;
        if flags & IS_UNICODE != 0 {
            let bytes = b.get(pos..pos + count * 2).ok_or_else(|| bad("string data truncated"))?;
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            *slot = Some(String::from_utf16_lossy(&units));
            pos += count * 2;
        } else {
            let bytes = b.get(pos..pos + count).ok_or_else(|| bad("string data truncated"))?;
            *slot = Some(bytes.iter().map(|&c| c as char).collect());
            pos += count;
        }
    }

    // ExtraData blocks until the terminal block (size < 4).
    while let Ok(size) = u32_at(b, pos) {
        let size = size as usize;
        if size < 8 {
            break;
        }
        if u32_at(b, pos + 4)? == ENVIRONMENT_VARIABLE_DATA_BLOCK {
            let uni = utf16_z(b, pos + 8 + 260).filter(|s| !s.is_empty());
            out.env_target = uni.or_else(|| ansi_z(b, pos + 8).filter(|s| !s.is_empty()));
        }
        pos += size;
    }
    Ok(out)
}

fn join_win(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        base.to_string()
    } else if base.ends_with('\\') {
        format!("{base}{suffix}")
    } else {
        format!("{base}\\{suffix}")
    }
}

/// Read and resolve `path`: LinkInfo target, else the environment-variable target
/// (expanded), else the relative path against the shortcut's directory.
pub(crate) fn resolve(path: &Path) -> io::Result<(Option<PathBuf>, Shortcut)> {
    let bytes = std::fs::read(path)?;
    let sc = parse(&bytes)?;
    let target =
        sc.target.clone().or_else(|| sc.env_target.as_deref().map(crate::path::expand_env)).map(PathBuf::from).or_else(
            || {
                sc.relative_path.as_ref().map(|r| {
                    let dir = path.parent().unwrap_or(Path::new(""));
                    crate::path::normalize(&dir.join(r.replace('\\', std::path::MAIN_SEPARATOR_STR)))
                })
            },
        );
    Ok((target, sc))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn push_u16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn push_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    fn header(flags: u32) -> Vec<u8> {
        let mut v = Vec::new();
        push_u32(&mut v, HEADER_SIZE);
        v.extend_from_slice(&LINK_CLSID);
        push_u32(&mut v, flags);
        v.resize(HEADER_SIZE as usize, 0);
        v
    }

    fn string_data(v: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        push_u16(v, units.len() as u16);
        for u in units {
            push_u16(v, u);
        }
    }

    /// A shortcut as Explorer writes it for `C:\Windows\notepad.exe` with arguments and a
    /// working directory (ID list omitted, LinkInfo with ANSI local base path).
    pub(crate) fn sample_lnk(base: &str, suffix: &str) -> Vec<u8> {
        let flags = HAS_LINK_INFO | HAS_RELATIVE_PATH | HAS_WORKING_DIR | HAS_ARGUMENTS | IS_UNICODE;
        let mut v = header(flags);
        // LinkInfo: header size 0x1C, VolumeIDAndLocalBasePath.
        let mut li = Vec::new();
        let volume_id = [0x10u8, 0, 0, 0, 3, 0, 0, 0, 0x78, 0x56, 0x34, 0x12, 0x10, 0, 0, 0];
        let lbp_off = 0x1C + volume_id.len();
        let cps_off = lbp_off + base.len() + 1;
        let total = cps_off + suffix.len() + 1;
        push_u32(&mut li, total as u32);
        push_u32(&mut li, 0x1C);
        push_u32(&mut li, 1);
        push_u32(&mut li, 0x1C);
        push_u32(&mut li, lbp_off as u32);
        push_u32(&mut li, 0);
        push_u32(&mut li, cps_off as u32);
        li.extend_from_slice(&volume_id);
        li.extend_from_slice(base.as_bytes());
        li.push(0);
        li.extend_from_slice(suffix.as_bytes());
        li.push(0);
        v.extend_from_slice(&li);
        string_data(&mut v, r"..\..\Windows\notepad.exe");
        string_data(&mut v, r"C:\Users\Public");
        string_data(&mut v, r#"/A "my file.txt""#);
        push_u32(&mut v, 0); // terminal block
        v
    }

    #[test]
    fn local_target_and_strings() {
        let sc = parse(&sample_lnk(r"C:\Windows\notepad.exe", "")).unwrap();
        assert_eq!(sc.target.as_deref(), Some(r"C:\Windows\notepad.exe"));
        assert_eq!(sc.arguments.as_deref(), Some(r#"/A "my file.txt""#));
        assert_eq!(sc.working_dir.as_deref(), Some(r"C:\Users\Public"));
        assert_eq!(sc.relative_path.as_deref(), Some(r"..\..\Windows\notepad.exe"));
        let sc = parse(&sample_lnk(r"C:\", r"Tools\app.exe")).unwrap();
        assert_eq!(sc.target.as_deref(), Some(r"C:\Tools\app.exe"));
    }

    #[test]
    fn network_target_env_block_and_id_list() {
        let flags = HAS_LINK_TARGET_ID_LIST | HAS_LINK_INFO;
        let mut v = header(flags);
        push_u16(&mut v, 4);
        v.extend_from_slice(&[2, 0, 0, 0]); // opaque ID list
        // LinkInfo with CommonNetworkRelativeLink.
        let net = r"\\server\share";
        let cnrl_len = 0x14 + net.len() + 1 + 1;
        let cnrl_off = 0x1C;
        let cps_off = cnrl_off + cnrl_len;
        let suffix = r"dir\file.txt";
        let total = cps_off + suffix.len() + 1;
        push_u32(&mut v, total as u32);
        push_u32(&mut v, 0x1C);
        push_u32(&mut v, 2);
        push_u32(&mut v, 0);
        push_u32(&mut v, 0);
        push_u32(&mut v, cnrl_off as u32);
        push_u32(&mut v, cps_off as u32);
        push_u32(&mut v, cnrl_len as u32);
        push_u32(&mut v, 0);
        push_u32(&mut v, 0x14);
        push_u32(&mut v, 0);
        push_u32(&mut v, 0);
        v.extend_from_slice(net.as_bytes());
        v.push(0);
        v.push(0);
        v.extend_from_slice(suffix.as_bytes());
        v.push(0);
        // EnvironmentVariableDataBlock.
        push_u32(&mut v, 0x314);
        push_u32(&mut v, ENVIRONMENT_VARIABLE_DATA_BLOCK);
        let mut ansi = b"%SystemRoot%\\x.exe".to_vec();
        ansi.resize(260, 0);
        v.extend_from_slice(&ansi);
        let mut uni = Vec::new();
        for c in "%SystemRoot%\\x.exe".encode_utf16() {
            push_u16(&mut uni, c);
        }
        uni.resize(520, 0);
        v.extend_from_slice(&uni);
        push_u32(&mut v, 0);
        let sc = parse(&v).unwrap();
        assert_eq!(sc.target.as_deref(), Some(r"\\server\share\dir\file.txt"));
        assert_eq!(sc.env_target.as_deref(), Some(r"%SystemRoot%\x.exe"));
    }

    #[test]
    fn rejects_garbage_and_resolves_files() {
        assert!(parse(b"not a shortcut at all, definitely not").is_err());
        let mut truncated = sample_lnk(r"C:\x.exe", "");
        truncated.truncate(100);
        assert!(parse(&truncated).is_err());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.lnk");
        std::fs::write(&p, sample_lnk(r"C:\Windows\notepad.exe", "")).unwrap();
        let (target, sc) = resolve(&p).unwrap();
        assert_eq!(target, Some(PathBuf::from(r"C:\Windows\notepad.exe")));
        assert!(sc.arguments.is_some());
    }
}
