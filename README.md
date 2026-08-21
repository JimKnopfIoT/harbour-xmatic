# xmatic

A native [Matrix](https://matrix.org) client for **Sailfish OS**, with a Silica
interface and a protocol core built on
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk).

Sailfish OS 5 still ships Qt 5.6, which rules out the existing Qt Matrix
libraries. Rather than hand-writing another Client-Server API implementation,
xmatic links the same Rust core that modern clients use and keeps Qt as the view
layer. Sliding sync, end-to-end encryption, cross-signing, device verification
and key backup therefore come from upstream rather than from this repository.

## What works

* Sign-in through the homeserver's own page (OAuth 2.0 / MAS), session survives
  restarts
* Classic password sign-in on homeservers without OAuth — the login page
  detects what the server speaks; the password is never stored or logged
* Session tokens and new local stores encrypted with a key from Sailfish
  Secrets (the first start asks once; declining keeps the app working
  unencrypted)
* Room list over Simplified Sliding Sync, with search and unread counts,
  grouped so favourites sit at the top and low-priority rooms at the bottom
* Timeline in encrypted rooms: send, reply, edit, delete, paginate, read
  receipts
* Attachments: pictures with full-screen view and zoom, video playback, files;
  saving, sharing and forwarding to another room
* Voice messages, recorded and played in place
* Device verification in both directions, cross-signing state, key backup and
  recovery
* A warning before a message goes into an encrypted room where a recipient
  still has devices you have not verified, with a per-user "do not warn again"
* Notifications for rooms that are not on screen, with a scheduled wake-up
  while the app is in the background
* Sync survives a network loss: after flight mode or a dead WLAN the app
  reconnects on its own and says so in the header while it does; messages
  written offline are sent automatically once the network is back
* Own profile: change display name and avatar in the app
* Mark rooms favourite or low priority, or mute them; encrypted, favourite,
  muted and low-priority rooms each carry their own marker, and muted as well
  as low-priority rooms stay quiet (no notifications)
* **A share target**: a link from the browser, a picture from the gallery or a
  file from the file manager can be sent into a room straight from the
  system's share dialog, whether or not the app is already running
* **Profile pictures** in the chat list, in a room's member list and next to
  other people's messages; a one-to-one chat shows the other person even when
  the room carries no picture of its own, and anyone without one gets a circle
  with their initial
* A message's actions move onto a page of their own in landscape, where a
  context menu would run off the bottom of the screen
* **Pinned messages**: a banner with the newest pin's text sticks to the top
  of the room, with a page of all pins and a jump straight to the message
* **Discover rooms**: search a public room directory — the own homeserver,
  matrix.org, matrixrooms.info, hackint.org or any server added by hand —
  with topic and member count as the preview, and join directly
* Spaces list their linked-but-not-joined rooms as joinable
* Direct chats, joining rooms by address, accepting invitations
* **Create a room** — public (listed in the homeserver's directory) or by
  invitation only, encrypted or not, both settled at creation because neither
  can be changed the same way afterwards — **invite** people into it, and
  **leave** a room again; on an invitation, leaving declines it
* **Member list** per room: every member's full Matrix address, sorted by
  power level
* **Member profile**, from the member list or a sender's picture in the
  conversation: the picture full size, the address to copy, the role, since
  when they are a member and who invited them, the rooms you share, and the
  state of their encryption identity — including the warning that it changed
  after you verified it. Direct message, verify, ignore; and, where the
  room's power levels allow it, remove, ban, or make somebody a moderator
* **Threads**: a message that started one carries a marker with the reply
  count and opens a page of its own; replies keep showing in the
  conversation as well, so nothing is hidden from anyone who never opens it
* **Ignored users** are listed under Account and can be taken off the list
  there; the list belongs to the account and holds in every client
* **Spaces** as their own navigation level: create, delete, add rooms by
  long-press, move rooms between spaces, nested spaces, unread badges per
  space, and a choice of rooms or spaces as the start page (swipe sideways for
  the other)
* **Voice and video calls** over WebRTC, encrypted, with the homeserver's TURN
  relay; the outgoing picture follows how the phone is held

## What does not

* **Homeservers that sign in through their own web page (SSO).** OAuth 2.0 /
  MAS, the device code and the classic password sign-in work; the SSO
  redirect flow is not implemented, and the login page says so rather than
  leaving you to retype a password that was never wrong.
* **Calling clients that only speak MatrixRTC.** They dropped the classic 1:1
  call flow (MSC2746). Where a client still offers a "legacy" call button,
  that one interoperates; the MatrixRTC one does not.
* Received video is converted frame by frame on the CPU, because this device
  has no GStreamer sink that draws into a QML scene. It works, but it is not
  as smooth as the outgoing direction.
* The app has to stay open — its cover on the home screen is the running
  process. There is no background daemon, the same way other messengers on
  this platform work.

## Requirements

* Sailfish OS 5.0 or newer on aarch64, or Sailfish OS 4.6 on armv7hl
  (community ports like the Gemini PDA — see the note below), developer
  mode enabled
* Sailfish OS Platform SDK with the `SailfishOS-5.0.0.62-aarch64` target, plus
  `gstreamer1.0-devel`, `gstreamer1.0-plugins-base-devel` and
  `gstreamer1.0-plugins-bad-devel` installed into that target
* Rust via rustup (the version is pinned in `rust-toolchain.toml`) and
  `cbindgen`
* A homeserver supporting Simplified Sliding Sync (MSC4186) — Synapse 1.114 or
  newer

## Building

```sh
cargo install cbindgen                     # once
scripts/build-core.sh                      # cross-build the Rust core
mb2 -t SailfishOS-5.0.0.62-aarch64 build   # build the RPM
```

For a 32-bit build against Sailfish 4.6 (with the matching SDK target
installed):

```sh
SFOS_RELEASE=4.6.0.13 SFOS_ARCH=armv7hl scripts/build-core.sh
mb2 -t SailfishOS-4.6.0.13-armv7hl build
```

The package lands in `RPMS/`. Copy it to a device and install it with
`pkcon install-local`.

If your SDK lives somewhere other than `$HOME/SailfishOS-Platform-SDK`, set
`SAILFISH_SDK_ROOT` (and optionally `SFOS_RELEASE` / `SFOS_ARCH`) first.

### Note for ported devices (e.g. Gemini PDA)

**On official Sailfish images this is already the case and you need to do
nothing** — the app itself asks for no privileged permissions. This concerns
only some hand-built community port images, where the `privileged` group
membership is missing for the phone user. Sailfish's app sandbox cannot finish
its setup then, and any sandboxed app — including this one — silently never
starts. **Check this in the terminal before installing:**

```sh
id | grep -o privileged
```

If that prints nothing, fix it once as root and reboot:

```sh
devel-su usermod -a -G privileged $USER
```

Also, the browser shipped with Sailfish 4.6 cannot render the sign-in pages
of current Matrix authentication servers. Use **"Sign in on another device"**
on the login page instead: the app shows a short address and a code, you
approve the sign-in from any browser on any machine, and the app picks the
session up by itself.

## How it fits together

```
qml/    Silica UI (Qt 5.6)
src/    C++ bridge: MatrixBridge (QObject), list models, image provider
core/   Rust staticlib "xmatic-core": tokio runtime, matrix-sdk,
        matrix-sdk-ui (SyncService / RoomList / Timeline), bundled SQLite, rustls
```

Commands go into the core as JSON; events come back as JSON through one
callback. The callback fires on a tokio thread, so the C++ side only hops into
the Qt loop with a queued `QMetaObject::invokeMethod`. The SDK's `VectorDiff`
streams map one-to-one onto `QAbstractListModel` insert/remove/change signals —
that mapping is the whole reason for this design. The FFI surface is six C
functions; everything else is a JSON message type.

## Licence

Apache-2.0, matching matrix-rust-sdk. The Rust core links a number of upstream
crates, each under its own permissive licence (see `core/Cargo.toml` and the
crates it pulls in).
