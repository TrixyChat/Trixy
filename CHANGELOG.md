# Trixy 0.6.1

- Fixed the Rust E0308 compile failure in the Share profile dialog by making both match arms return `()`.
- Updated deprecated `ComboBox::from_id_source` calls to `from_id_salt`.
- Made egui stroke widths explicitly `f32` for newer Rust compiler compatibility.

# Changelog

## 0.6.1

- Redesigned the top toolbar and left navigation as softer floating Apple-inspired surfaces instead of rigid full-width bars.
- Added a Trixy app mark plus macOS `.icns`, Windows `.ico`, and cross-platform PNG app icons.
- Added **Workspaces / Contacts** segmented navigation with instant search.
- Added a searchable contact directory that groups each person across every Firebase connection where they are reachable.
- Added support for **multiple Firebase Realtime Database URLs at the same time**.
- Added a Firebase Connections section in Settings with friendly labels for each database.
- Each workspace is now explicitly routed through one Firebase connection; new workspaces choose the connection at creation time.
- New `TRIXY-CONTACT2-...` profile codes include the selected Firebase URL so importing a profile can add the required connection automatically.
- Added encrypted `trixy://join/...` workspace/org links.
- Joining a workspace link automatically adds that Firebase database, imports the workspace, and adds every workspace member to Contacts.
- Reusable org links now converge dynamically: when the original sharer sees a new link-based join, Trixy sends the joiner a fresh encrypted workspace/member snapshot so later joiners learn contacts added after the link was first created.
- Added a signed `member_joined` event for workspace-link admission while retaining existing direct person-to-person invitations.
- Sync now polls every configured Firebase database and routes each outgoing packet/file through the workspace's assigned connection.
- Preserved the v0.5 database, message protocol, attachment format, notification behavior, and existing Workmsg/Trixy upgrade paths.

## 0.5.0

- Rebuilt the desktop interface around an Apple-inspired light design system.
- Added a fixed bottom message composer so attachments cannot push the Send control off-screen.
- Changed selected-file previews to one horizontally scrolling chip row.
- Refreshed workspace headers, messages, attachment cards, setup, status presentation, and notification toasts.

## 0.4.0

- Renamed Workmsg to **Trixy** while retaining the existing `workmsg_v1` Firebase root for compatibility.
- Added incoming-message alerts and native application-attention requests.
- Changed new attachment chunks from 512 KiB to 32 KiB.
- Moved new attachment storage to random synthetic mailbox paths so files reuse the Firebase permission path used by chat.
- Added independent retry for attachment chunks.
- Added legacy failed-transfer fallback for v0.3 attachments.

## 0.3.0

- Added the light theme, code block rendering, file picker/drag-and-drop attachments, Windows source builds, and macOS/Windows packaging scripts.
