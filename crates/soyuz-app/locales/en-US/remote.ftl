### Soyuz UI strings (en-US source): remote
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/remote.ftl.

## Toasts and prompts

remote-sign-in-failed = Sign-in failed. Check the user name and password.
# $server: e.g. "bob@example.com".
remote-disconnected = Disconnected from { $server }
remote-downloading = Downloading { $name }…
remote-downloading-body = It opens when the download finishes.
remote-download-failed = Couldn't download { $name }
remote-copied = { $count ->
    [one] Copied { $count } remote item
   *[other] Copied { $count } remote items
}
remote-cut = { $count ->
    [one] Cut { $count } remote item
   *[other] Cut { $count } remote items
}
remote-clipboard-body = Paste into any folder or server with Ctrl+V.
remote-nothing-to-paste = Nothing to paste
remote-nothing-to-paste-body = Copy or cut files first (Ctrl+C / Ctrl+X).
remote-no-other-folder = No folder in the other pane
remote-delete-title = Delete from server?
# $count items ($name when there is one) on $server.
remote-delete-message = { $count ->
    [one] “{ $name }” will be deleted from { $server } permanently. Servers have no trash, so this can't be undone.
   *[other] { $count } items will be deleted from { $server } permanently. Servers have no trash, so this can't be undone.
}
remote-new-folder-name = New folder
remote-unknown-time = unknown
# Properties toast; $when is a date, $size a formatted size.
remote-props-folder = Folder · modified { $when }
remote-props-file = { $size } · modified { $when }
# Commands remote locations can't do.
remote-unavailable = { $what ->
    [new-file] New files aren't available on remote locations
    [duplicate] Duplicate isn't available on remote locations
    [links] Links aren't available on remote locations
    [terminal] Terminals aren't available on remote locations
    [reveal] The system file manager isn't available on remote locations
   *[other] This command isn't available on remote locations
}

## Sidebar (Network section)

remote-network = Network
remote-state-connected = Connected
remote-state-connecting = Connecting…
remote-state-failed = Failed: { $error }
remote-state-idle = Not connected
remote-open-new-tab = Open in new tab
remote-edit-connection = Edit connection…
remote-disconnect = Disconnect
remote-copy-address = Copy address
remote-remove = Remove from sidebar
remote-connect = Connect to server…
remote-connect-tip = SFTP, WebDAV, FTP / FTPS or S3-compatible storage

## Connect dialog

remote-dialog-connect-title = Connect to server
remote-dialog-edit-title = Edit connection
remote-protocol = Protocol
remote-bucket = Bucket
remote-server = Server
remote-region = Region
remote-port = Port
remote-endpoint = Endpoint
remote-sign-in-with = Sign in with
remote-access-key = Access key
remote-user = User
remote-user-hint = user name
remote-secret-key = Secret key
remote-key-passphrase = Key passphrase
remote-key-passphrase-hint = if the key has one
remote-password = Password
remote-secret-unchanged = unchanged
remote-key-file-label = Private key file
remote-agent-note = Uses the Windows OpenSSH agent (or Pageant). Add keys with ssh-add.
remote-agent-note-unix = Uses your SSH agent (SSH_AUTH_SOCK). Add keys with ssh-add.
remote-prefix = Prefix (optional)
remote-folder = Folder (optional)
remote-save = Save in the sidebar (Network)
remote-name-hint = Name (optional)
remote-remember = Remember the secret (Windows Credential Manager)
remote-remember-linux = Remember the secret (Secret Service keyring)
remote-connect-button = Connect
remote-auth-password = Password
remote-auth-key-file = Key file
remote-auth-agent = SSH agent
remote-auth-access-keys = Access keys
remote-auth-public = Public
remote-auth-anonymous = Anonymous
remote-error-bucket = Enter the bucket name.
remote-error-server = Enter the server name.
# $port: what the user typed.
remote-error-port = “{ $port }” is not a valid port.
remote-error-sftp-user = SFTP needs a user name.

## Host key prompt

remote-hostkey-changed-title = Server key changed!
remote-hostkey-trust-title = Trust this server?
# $host: server name, with ":port" when not 22.
remote-hostkey-changed-body = { $host } presented a different key than the one you trusted before. This can mean the server was reinstalled, or that someone is intercepting the connection.
remote-hostkey-first-body = This is the first connection to { $host }. Check that the fingerprint matches the server's before trusting it.
remote-hostkey-previous = Previously trusted
remote-hostkey-trust-new = Trust the new key
remote-hostkey-trust = Trust and connect

## New folder / rename on a server

remote-new-folder-title = New folder
remote-name = Name
remote-create = Create
remote-error-name = Enter a name without slashes.
