# Trixy v0.6

Trixy is a small, local-first encrypted project messenger written in Rust. Each computer keeps its identity, contacts, workspaces, messages, attachments, and cryptographic keys in local SQLite storage. Firebase Realtime Database is only an outbound-HTTPS mailbox for encrypted data.

There is no Trixy server process, no inbound listener, no VPN requirement, and no port forwarding.

## What's new in v0.6

### Softer desktop shell

The conversation area and fixed composer from v0.5 are preserved, while the surrounding app has been redesigned:

- floating rounded top toolbar instead of a rigid edge-to-edge bar
- floating rounded sidebar instead of a hard application rail
- custom Trixy logo and native app icons
- quieter system-gray surfaces and Apple-blue accent
- searchable **Workspaces / Contacts** switcher
- workspace connection labels and connection status
- compact modals for profile sharing, joining, people, and settings

### Multiple Firebase databases

A single Trixy profile can now stay connected to several Firebase Realtime Database URLs at the same time.

- Add connections in **Settings**.
- Give each connection a friendly label such as `Biostat`, `Grant Team`, or `Methods Lab`.
- Each workspace belongs to exactly one Firebase connection.
- Trixy polls your mailbox on every configured connection during normal sync.
- Outgoing packets and encrypted files are routed through the workspace's selected connection.

Every Firebase database you add must use the included `firebase-rules.json`.

### Searchable Contacts

The left sidebar now switches between **Workspaces** and **Contacts**. Contacts can be searched by display name or Firebase connection label.

A contact is one cryptographic identity with one or more database routes. This means the same person can participate with you on multiple Firebase databases without becoming several unrelated contacts.

New profile codes start with:

```text
TRIXY-CONTACT2-
```

They include the public profile, Firebase URL, and friendly connection label required for that route. Importing the code automatically adds the connection if it is not already configured.

### Workspace / organization links

A workspace can be shared as an encrypted capability link:

```text
trixy://join/...
```

Use **Share workspace** in the conversation header or People window. The share operation publishes an encrypted workspace package at a random Firebase capability path and puts the random decryption key in the link.

When another Trixy v0.6 user pastes the link into **Join workspace**, Trixy automatically:

1. adds the required Firebase connection
2. verifies the inviter and signed workspace package
3. imports the workspace and current encrypted event history
4. imports every existing workspace member into **Contacts**
5. adds the joining user to the workspace
6. broadcasts a signed membership event so other members learn the new contact

Membership changes are gossiped between clients. When the original link sharer observes a link-based join, it also queues a fresh encrypted workspace snapshot back to that joiner, so reusable org links converge on members/contacts added after the link was first created.

**Workspace links are invitation secrets.** Anyone who obtains a valid unexpired link can join that workspace in this prototype. Link expiry/revocation is not implemented yet.

### Compatibility note

Existing Workmsg/Trixy databases are migrated in place. You do not need to recreate your identity, contacts, workspaces, or messages.

If you start using the new workspace/org-link join feature, update active members of that workspace to **Trixy v0.6 or newer**. Older v0.5 clients do not understand the new signed `member_joined` event type.

## Existing data locations

Trixy first prefers an existing legacy database if it finds one and no newer Trixy database has already been created.

Legacy locations include:

- macOS: `~/Library/Application Support/Workmsg/workmsg-firebase-v02.db`
- Windows: `%APPDATA%\Workmsg\workmsg-firebase-v02.db`

New installations use:

- macOS: `~/Library/Application Support/Trixy/trixy.db`
- Windows: `%APPDATA%\Trixy\trixy.db`
- Linux: `~/.local/share/trixy/trixy.db`

For local testing, override the database path with `TRIXY_DB`. The old `WORKMSG_DB` variable is still accepted.

## Firebase rules

For every Firebase Realtime Database used by Trixy:

1. Open the Firebase project.
2. Open **Realtime Database -> Rules**.
3. Replace the rules with this repository's `firebase-rules.json`.
4. Publish.

Trixy keeps the existing `workmsg_v1` root for upgrade compatibility.

## Run from source

```bash
cargo run --release --bin trixy
```

On first launch, create your profile with a display name and one Firebase Realtime Database URL. Additional databases can be added later in **Settings**.

## Test two clients on one Mac

Open two terminals in the project directory:

```bash
./scripts/run-alice.sh
```

```bash
./scripts/run-bob.sh
```

Give each profile a different display name and use the same Firebase URL for the simplest first test.

To test an organization link with a third local client:

```bash
./scripts/run-carol.sh
```

Create a workspace with Alice and Bob, generate **Share workspace**, paste the link into Carol's **Join** window, and verify that Alice and Bob appear automatically in Carol's Contacts.

## Add a person

On one client, open **Settings -> Share my profile** (or the profile share window), choose the Firebase connection, and copy the `TRIXY-CONTACT2-...` code.

On the other client, choose **Add person** and paste it. The required Firebase connection is imported with the contact.

## Create a workspace on a specific database

Use the `+` workspace control, enter a name, and choose one of your configured Firebase connections. Only contacts that have a route on that connection can be manually added to the workspace.

## Share an entire workspace / org directory

Open the workspace and choose **Share workspace**. Generate a link and send it to another Trixy v0.6 user.

The recipient chooses **Join workspace** and pastes the link. All current workspace members are added to their Contacts automatically.

## Message alerts

Trixy polls every configured Firebase connection about every two seconds while running. New remote messages can:

- show an in-app toast
- request Dock/taskbar attention
- play the platform notification sound
- on macOS, request a Notification Center banner

Trixy must be running for local alerts to occur.

## Code blocks

Matching double backticks render code:

```text
``SELECT * FROM analysis;``
```

Multiline form:

```text
``
fit <- lm(y ~ x, data = dat)
summary(fit)
``
```

Triple-backtick fences are also recognized.

## File attachments

Use the `+` control or drag a file into Trixy. New attachments:

1. are copied into local Trixy storage
2. record byte count and SHA-256 in the signed/encrypted message
3. are split into 32 KiB plaintext chunks
4. encrypt each chunk independently with ChaCha20-Poly1305
5. upload chunks through a random Firebase mailbox capability
6. retry each chunk independently
7. are reconstructed and SHA-256 verified by the receiver

The current test limit is 50 MB per file.

## Packaging

### macOS app

```bash
./scripts/build-macos-app.sh
```

### macOS DMG

```bash
./scripts/build-macos-installer.sh
```

### Windows executable

```powershell
.\scripts\build-windows.ps1
```

### Windows NSIS installer

```powershell
.\scripts\build-windows-installer.ps1
```

The repository also includes `.github/workflows/build-installers.yml` to create macOS and Windows installer artifacts on native GitHub Actions runners.

The generated test packages are not Developer ID / Authenticode signed by default. Managed workplace computers may require signed/notarized distribution.

## Security status

This is still a prototype. See `SECURITY.md` before using it for sensitive work. In particular, do not use the current test build for PHI, regulated research data, credentials, or other restricted institutional data without institutional security review and an approved Firebase deployment.
