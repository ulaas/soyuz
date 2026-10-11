# RPM package for Soyuz, repackaging the prebuilt release tarball made by scripts/package.sh
# (soyuz-<version>-linux-<arch>.tar.gz, a PREFIX tree). Nothing is compiled here.
#
#   scripts/package.sh --rpm                       # builds the tarball, then this spec
#   rpmbuild -bb installer/linux/soyuz.spec \
#       --define "_sourcedir $PWD/dist" --define "soyuz_version 0.2.0"
#
# The org.freedesktop.FileManager1 D-Bus activation file is deliberately NOT installed:
# Soyuz writes it to the user's ~/.local/share/dbus-1/services/ only on opt-in
# (docs/RELEASING.md, "Linux").

%{!?soyuz_version: %global soyuz_version 0.2.0}
%{!?soyuz_arch: %global soyuz_arch %{_arch}}
%global appid io.github.ulaas.Soyuz
%{!?_metainfodir: %global _metainfodir %{_datadir}/metainfo}

# A prebuilt, already stripped Rust binary: no debuginfo package, no build-id links.
%global debug_package %{nil}
%global _build_id_links none
%global __strip /bin/true

Name:           soyuz
Version:        %{soyuz_version}
Release:        1%{?dist}
Summary:        Keyboard-first file manager that never blocks
License:        MIT
URL:            https://github.com/ulaas/soyuz
Source0:        soyuz-%{soyuz_version}-linux-%{soyuz_arch}.tar.gz
ExclusiveArch:  x86_64 aarch64

# winit, wgpu and fontconfig are loaded with dlopen(), so the automatic ELF dependency
# scan does not see them.
Requires:       libxkbcommon
Requires:       fontconfig
Requires:       hicolor-icon-theme
Recommends:     libwayland-client
Recommends:     libxkbcommon-x11
Recommends:     libX11
Recommends:     libXcursor
Recommends:     libXi
Recommends:     libXrandr
Recommends:     vulkan-loader
Recommends:     mesa-vulkan-drivers
Recommends:     mesa-libEGL

%description
Soyuz is a file manager built around one promise: the window never stops
responding. Every copy, move, delete, extract or checksum runs as a background
job with progress, pause, resume, cancel, conflict handling, history and undo.
Tabs, dual panes, previews, search, archives, an embedded terminal and remote
locations, all keyboard-first.

%prep
%setup -q -n soyuz-%{soyuz_version}-linux-%{soyuz_arch}

%build
# Prebuilt by scripts/package.sh.

%install
install -Dm755 bin/soyuz %{buildroot}%{_bindir}/soyuz
install -Dm644 share/applications/soyuz.desktop %{buildroot}%{_datadir}/applications/soyuz.desktop
install -Dm644 share/metainfo/%{appid}.metainfo.xml %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
for icon in share/icons/hicolor/*/apps/soyuz.*; do
    install -Dm644 "$icon" "%{buildroot}%{_datadir}/${icon#share/}"
done

%check
if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate %{buildroot}%{_datadir}/applications/soyuz.desktop
fi
if command -v appstreamcli >/dev/null; then
    appstreamcli validate --no-net %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
fi
%{buildroot}%{_bindir}/soyuz --version

%files
%license share/licenses/soyuz/LICENSE
%license share/licenses/soyuz/THIRD-PARTY-NOTICES.md
%doc share/doc/soyuz/README.md
%doc share/doc/soyuz/USER_GUIDE.md
%doc share/doc/soyuz/config.example.toml
%{_bindir}/soyuz
%{_datadir}/applications/soyuz.desktop
%{_datadir}/icons/hicolor/*/apps/soyuz.*
%{_metainfodir}/%{appid}.metainfo.xml

%changelog
* Sun Oct 11 2026 The Soyuz contributors - 0.2.0-1
- Version 0.2.0.

* Sun Oct 11 2026 The Soyuz contributors - 0.1.0-1
- First Linux package.
