# Trixy security notes

Trixy v0.6 is a functional encrypted-messaging prototype, not a production security certification.

## Local-first state

Each computer stores the authoritative local copy of its Trixy identity, contacts, workspace keys, event history, messages, and cached/downloaded attachments in SQLite/local files.

Back up and protect the local computer appropriately. Private identity keys and workspace keys currently live in the local database rather than the macOS Keychain or Windows Credential Manager.

## Firebase transport

Trixy only opens outbound HTTPS connections to configured Firebase Realtime Database URLs. Firebase is used as an encrypted mailbox/capability store, not as the canonical plaintext workspace database.

Trixy v0.6 can connect to several Firebase databases simultaneously. Each workspace is bound to one Firebase connection and its outgoing events/files are routed through that connection.

Every configured Firebase database must publish the included rules. The current prototype uses high-entropy random mailbox/capability identifiers rather than Firebase Authentication. That is intentionally simple for testing, but it is not a substitute for production-grade authorization.

## Cryptography

The current design uses:

- Ed25519 signatures for identity/event authenticity
- X25519-derived keys for direct encrypted workspace invitations
- ChaCha20-Poly1305 for encrypted workspace events and attachment chunks
- SHA-256 for attachment integrity checks and deterministic Firebase connection identifiers

Cryptographic implementation and protocol design have not undergone an independent security audit.

## Contact codes

`TRIXY-CONTACT2-...` codes contain only public identity material, a random mailbox capability, and the Firebase URL for that route. They do not contain the user's private signing or X25519 secret keys.

Treat a contact code as internal routing information rather than a public social profile.

## Workspace/org share links

A `trixy://join/...` link is a **bearer invitation capability**.

The encrypted workspace package is stored at a random Firebase capability path. The link contains:

- the Firebase URL
- a random lookup token
- a random decryption key
- workspace metadata needed to locate/validate the package

The encrypted package contains the workspace key, member public profiles/routes, event history, inviter identity, and a signed join permit.

Joining verifies the inviter's signatures before importing the workspace. Existing workspace members in the package are also added to the recipient's Contacts. The joining user's private keys are never uploaded or placed in the link.

### Important limitations

The v0.6 prototype does **not** yet provide:

- workspace-link expiry
- explicit link revocation
- one-use links
- approval of each join by an online administrator
- Firebase Authentication-based access control
- membership removal/key rotation

Anyone who obtains a still-valid link can join the workspace. Share links only through channels appropriate for the project.

Membership events are gossiped by Trixy v0.6 clients to help clients converge on the member/contact directory. When the original link sharer observes a link-based join, it also sends that joiner a fresh sealed workspace snapshot containing the then-current directory. Older clients should be upgraded before relying on org-link joins.

## Attachments

New files are encrypted in 32 KiB chunks using the workspace key with a fresh nonce for each chunk. The signed message carries expected size and SHA-256, and receivers verify the reconstructed file before accepting it.

Encrypted chunks are currently retained in Firebase so members who were offline can download later. Retention/cleanup and storage quotas need production design.

## Notifications

Notifications are generated locally after Trixy receives and decrypts a remote message. They do not add a push-notification server. Operating-system notification previews may reveal sender/message text on a locked screen depending on OS settings.

## Not approved for regulated data

Do not assume a personal Firebase project or this prototype is approved for PHI, regulated research data, passwords, secrets, or other restricted institutional information. Production use should include institutional security/privacy review, approved cloud configuration, key storage hardening, authenticated authorization rules, update/signing strategy, and an external protocol/code review.
