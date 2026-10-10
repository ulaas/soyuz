# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue: use GitHub's
**Report a vulnerability** button (Security tab → Advisories) on this repository.

Include the Soyuz version (Help → About), your OS, and a minimal way to reproduce the
problem (a crafted file, archive or folder is ideal). You can expect an acknowledgement
within 7 days and a fix or mitigation within 90 days. Credit is given unless you ask
otherwise.

## Supported versions

Only the latest release and `main`. Soyuz is pre-1.0, so there are no backports.

## Scope

Soyuz runs with your own privileges and does not try to defend against malware already
running as you. It does defend you against the data it merely looks at: browsing a folder,
previewing a file or opening an archive must never run code chosen by the file's author,
change files you didn't ask it to change, send file contents or names anywhere, destroy
data without a way back, or hang the UI. Reports in that area are especially welcome.
