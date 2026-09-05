# xmatic

A native [Matrix](https://matrix.org) client for **Sailfish OS**, with a Silica
interface and a protocol core built on
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk).

Sailfish OS 5 still ships Qt 5.6, which rules out the existing Qt Matrix
libraries. xmatic links the same Rust core that modern clients use and keeps Qt
as the view layer, so sliding sync, end-to-end encryption, cross-signing,
device verification and key backup come from upstream rather than from this
repository.

## What works

* Sign-in through the homeserver's own page (OAuth 2.0 / MAS), by device code
  on another machine, or by password where the server offers no OAuth; the
  session survives restarts
* Session and local stores encrypted with a key from Sailfish Secrets. A
  device whose system does not provide that key service creates no store at
  all: it says what is missing, how to install it and how to check the result,
  and running without encryption is an explicit choice rather than a silent
  fallback
* Four coloured lines say how safe the device is — backup, recovery,
  cross-signing, local storage. Green is in order, orange is a fault you can
  clear, red is missing. Where something is not green they come up once after
  starting, with the action that fits and "later" always available
* Room list over Simplified Sliding Sync: search, unread counts, favourites,
  low priority, mute
* Timeline in encrypted rooms: send, reply, edit, delete, paginate. A message
  that could not be sent can be sent again or discarded
* Reactions, sent and shown, grouped by character with a count; the picker's
  first tab is yours to fill - press and hold any emoji to keep it there - and
  the keyboard covers what the set does not
* Formatted messages rendered as formatting — bold, italic, struck through,
  underlined, code, quotes, lists, headings, links. The HTML a message carries
  is rewritten inside the app instead of being handed to a markup parser, and
  what the platform's text renderer cannot draw is drawn another way rather
  than dropped
* Formatting on the way out too: hold a word in the message field to mark it,
  then bold, italic, struck through, underlined or monospace from the row that
  appears. Markers typed by hand do the same, and a message without any ships
  no second copy of itself
* Attachments: pictures with full-screen zoom, video, files; save, share,
  forward. Voice messages recorded and played in place
* One picker for an attachment with two tabs: the gallery, divided by the
  folders that exist, and the file system from the home folder down. Both
  select several files at once, into one list
* Pictures are made smaller before they go out and lose their metadata with it,
  the place a photograph was taken included. A screenshot keeps every pixel it
  has. A switch on the send page keeps a single picture as it lies
* A switch for the return key: it sends, or it breaks the line. Where it sends,
  holding the send arrow breaks the line instead, and the keyboard stays up
* Device verification in both directions, cross-signing, key backup and
  recovery, and a warning before sending to devices you never verified
* Notifications for rooms not on screen, with a background wake-up; tapping one
  opens the room it is about. Messages written offline go out when the network
  returns
* Rooms: create (public or invite-only, encrypted or not, both settled at
  creation), invite, join by address, leave, direct chats, and a search across
  public room directories
* A room opens where you stopped reading, counts as read while it is read, and
  can be marked read from the chat list without opening it
* Matrix links lead into the app: a permalink or a plain #room:server opens the
  room, or offers to join it - the tap itself never joins
* Spaces as their own navigation level: nested, unread badges, rooms added and
  moved by long-press, either list as the start page
* Members: list per room, profile with the encryption identity, moderation
  where the power levels allow it, account-wide ignore list
* Threads: answer in one, or start one from any message; replies stay in the
  room's timeline as well
* Pinned messages, own display name and avatar, profile pictures
* Whether a room is encrypted is a padlock in front of its name: closed, or
  open with its body struck through
* Text typed and not sent stays in the room until it is; it is kept in memory
  only and never written to disk
* Who read a message, and who set a reaction — both asked for on demand rather
  than carried by every row
* Deleting a message takes a countdown, the way leaving a room does
* A share target for the system's share dialog, running or not
* Voice and video calls over WebRTC. The media is encrypted between the two
  devices either way; in an encrypted room the signalling is too, which is what
  makes the call end-to-end and not merely encrypted on the wire
* Twenty-nine interface languages, picked in the app rather than only by the
  phone's setting

## What it looks like

| | | |
|---|---|---|
| <img src="screenshots/01-security-status.jpg" alt="Security at a glance" width="240"> | <img src="screenshots/02-verification.jpg" alt="Verification" width="240"> | <img src="screenshots/03-encryption.jpg" alt="Encryption" width="240"> |
| What is not in order yet, and what to do about it | Seven emoji, in the same order on both devices | Backup, recovery, cross-signing, local storage |
| <img src="screenshots/04-account.jpg" alt="The account" width="240"> | <img src="screenshots/05_1-appearance-colours.jpg" alt="Colours" width="240"> | <img src="screenshots/05_2-appearance-options.jpg" alt="More appearance" width="240"> |
| The account, the device and this app | Every bubble, name and text colour is yours to set | Opacity, the return key, the keyboard, reactions as pictures |
| <img src="screenshots/06-privacy.jpg" alt="Privacy" width="240"> | <img src="screenshots/07-privacy-device.jpg" alt="On this device" width="240"> | <img src="screenshots/08-languages.jpg" alt="Languages" width="240"> |
| Who may call, and what others learn | What stays here, and for how long | Twenty-nine languages, switchable in the app |
| <img src="screenshots/09-rooms.jpg" alt="The room list" width="240"> | <img src="screenshots/10-conversation.jpg" alt="A conversation" width="240"> | <img src="screenshots/11-room-menu.jpg" alt="The room's menu" width="240"> |
| The room list, with search | Replies, reactions, a pinned message and a picture | What a room offers, including a call |
| <img src="screenshots/12-room-info.jpg" alt="Room info" width="240"> | <img src="screenshots/13-rooms-menu.jpg" alt="Starting something" width="240"> | <img src="screenshots/14-directory.jpg" alt="The room directory" width="240"> |
| A room's own settings and address | New chat, new room, join, discover | Public rooms, from your server or another |

Taken on a phone with a camera cutout, across several ambiences — the app takes
its colours from the one you use.

## What does not

* **Homeservers that sign in through their own web page (SSO).** OAuth 2.0 /
  MAS, the device code and the password flow work; the SSO redirect does not,
  and the login page says so.
* **Clients that only speak MatrixRTC.** They dropped the classic 1:1 call
  flow (MSC2746); where a client still offers the legacy button, that one
  interoperates.
* Received video is converted frame by frame on the CPU — functional, but not
  as smooth as the outgoing direction. No GStreamer sink on this device draws
  into a QML scene.
* The app has to stay open; its cover on the home screen is the running
  process. There is no background daemon, the same way other messengers on
  this platform work.

  Push notifications are the one way around that, and they are **off by
  default, have their own setting, and do nothing until switched on by hand**.
  The feature exists because users asked for it. It needs a UnifiedPush
  distributor installed separately and a Matrix push gateway you choose
  yourself, and it discloses metadata — which rooms, at what times — to two
  parties that otherwise know nothing about you. Read
  [docs/PUSH.md](docs/PUSH.md) before turning it on. Leaving it off changes
  nothing: no address is created and your homeserver is told nothing.
* Spoilers are marked rather than hidden: the text renderer available here
  cannot hide a run of text.
* **Most translations are unreviewed.** German is the project language,
  Norwegian came largely from a Norwegian speaker, and Finnish, Swedish and
  Danish had a correction pass. The rest, Chinese and Hindi among them, are a
  single machine pass. An offer to review one is welcome — open an issue and
  you get a numbered sheet.
* Reactions are drawn as the characters they are, which on this Qt means
  monochrome. Pictures of your own can be used instead (Appearance); none are
  shipped and none are downloaded.

## Requirements

* Sailfish OS 5.0 or newer on aarch64, or Sailfish OS 4.6 on armv7hl
  (community ports — see the note below), developer mode enabled
* Sailfish OS Platform SDK with the `SailfishOS-5.0.0.62-aarch64` target, plus
  `gstreamer1.0-devel`, `gstreamer1.0-plugins-base-devel` and
  `gstreamer1.0-plugins-bad-devel` installed into it
* Rust via rustup (version pinned in `rust-toolchain.toml`) and `cbindgen`
* A homeserver supporting Simplified Sliding Sync (MSC4186) — Synapse 1.114 or
  newer

## Building

```sh
cargo install cbindgen                     # once
scripts/build-core.sh                      # cross-build the Rust core
mb2 -t SailfishOS-5.0.0.62-aarch64 build   # build the RPM
```

32-bit, against Sailfish 4.6 with the matching SDK target:

```sh
SFOS_RELEASE=4.6.0.13 SFOS_ARCH=armv7hl scripts/build-core.sh
mb2 -t SailfishOS-4.6.0.13-armv7hl build
```

The package lands in `RPMS/`; install it with `pkcon install-local`. If the SDK
lives somewhere other than `$HOME/SailfishOS-Platform-SDK`, set
`SAILFISH_SDK_ROOT` (and optionally `SFOS_RELEASE` / `SFOS_ARCH`) first.

### Note for ported devices

Some hand-built port images leave the phone user out of the `privileged`
group. Sailfish's sandbox then cannot finish its setup and *any* sandboxed app
silently never starts. Official images are unaffected, and xmatic itself asks
for no privileged permissions. Check before installing:

```sh
id | grep -o privileged
```

If that prints nothing: `devel-su usermod -a -G privileged $USER`, then reboot.

The browser on Sailfish 4.6 cannot render current Matrix sign-in pages. Use
**"Sign in on another device"** instead: the app shows an address and a code,
and the sign-in happens in any other browser.

## How it fits together

```
qml/    Silica UI (Qt 5.6)
src/    C++ bridge: MatrixBridge (QObject), list models, image provider
core/   Rust staticlib "xmatic-core": tokio runtime, matrix-sdk,
        matrix-sdk-ui (SyncService / RoomList / Timeline), bundled SQLite, rustls
```

Commands go into the core as JSON; events come back as JSON through one
callback. That callback fires on a tokio thread, so the C++ side only hops into
the Qt loop with a queued `QMetaObject::invokeMethod`. The SDK's `VectorDiff`
streams map one-to-one onto `QAbstractListModel` insert/remove/change signals —
that mapping is the whole reason for this design. The FFI surface is six C
functions; everything else is a JSON message type.

## Licence

Copyright 2026 JimKnopfIoT. Apache-2.0, matching matrix-rust-sdk. The Rust core
links a number of upstream crates, each under its own permissive licence (see
`core/Cargo.toml` and the crates it pulls in).
