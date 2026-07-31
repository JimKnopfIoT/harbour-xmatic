# Neutral packaging metadata — no personal identifiers (see anonymity rules).
# The build host's name would otherwise end up in the RPM header.
%define _buildhost reproducible-builder
Name:       harbour-xmatic
Summary:    Matrix client for Sailfish OS
# Kept in sync with the last published release; dev builds append +main.<date>.
Version:    0.13.0
Release:    1
License:    ASL 2.0
URL:        https://github.com/JimKnopfIoT/harbour-xmatic
Source0:    %{name}-%{version}.tar.bz2
Vendor:     harbour-xmatic contributors
Packager:   harbour-xmatic contributors

Requires:   sailfishsilica-qt5
Requires:   sailfish-content-graphics
Requires:   nemo-qml-plugin-notifications-qt5
# Calls load these at run time rather than linking them, so they have to be
# named explicitly: webrtcbin and DTLS live in plugins-bad, the RTP session
# management in plugins-good, Opus in plugins-base, and the ICE agent in
# libnice. Without them the app still runs and reports that the device has no
# WebRTC support, but no call can be placed.
Requires:   gstreamer1.0-plugins-bad
Requires:   gstreamer1.0-plugins-good
Requires:   gstreamer1.0-plugins-base
# By path, not by name: rpmlint rejects depending on a library package
# directly, and the plugin file is what actually has to be present. %{_libdir}
# is /usr/lib64 on aarch64 and /usr/lib on armv7hl/i486.
Requires:   %{_libdir}/gstreamer-1.0/libgstnice.so
# Depend on the QML module by path: requiring the library package by name is
# what rpmlint objects to, and the module is what the app actually imports.
Requires:   %{_libdir}/qt5/qml/Nemo/KeepAlive/qmldir
BuildRequires: pkgconfig(sailfishapp)
BuildRequires: pkgconfig(Qt5Core)
BuildRequires: pkgconfig(Qt5Qml)
BuildRequires: pkgconfig(Qt5Quick)
BuildRequires: pkgconfig(Qt5Network)
BuildRequires: pkgconfig(Qt5DBus)
BuildRequires: pkgconfig(Qt5Multimedia)
BuildRequires: pkgconfig(gstreamer-1.0)
BuildRequires: pkgconfig(gstreamer-sdp-1.0)
BuildRequires: pkgconfig(gstreamer-webrtc-1.0)
BuildRequires: pkgconfig(libpulse)
BuildRequires: pkgconfig(keepalive)
BuildRequires: desktop-file-utils

%description
Native Matrix client for Sailfish OS with a Silica interface. The protocol
core is built on matrix-rust-sdk, so sliding sync and end-to-end encryption
including cross-signing, device verification and key backup work the same way
they do in Element X. Voice calls run over WebRTC via GStreamer. The Rust core
is linked in statically and is built ahead of packaging by
scripts/build-core.sh.

%prep
%setup -q

%build
# Bakes the package version into the binary for the About page. Passed as
# bare tokens — quotes do not survive rpm+shell+qmake+make, so the source
# stringifies the macro itself.
%qmake5 "DEFINES+=XMATIC_VERSION=%{version}-%{release}"
%make_build

%install
%qmake5_install
# sailfishapp.prf disables qmake's strip, and the statically linked Rust core
# carries a lot of debug info, so strip explicitly.
strip %{buildroot}%{_bindir}/%{name}

%files
%defattr(-,root,root,-)
%license LICENSE
%{_bindir}/%{name}
%{_datadir}/%{name}
%{_datadir}/applications/%{name}.desktop
# Not named after the package: the D-Bus name is Sailjail's, and the file has
# to carry it. sailfish-share's own file trigger picks the new share method out
# of the desktop file, so nothing has to be run here.
%{_datadir}/dbus-1/services/org.xmatic.xmatic.service
%{_datadir}/icons/hicolor/*/apps/%{name}.png

%changelog
* Fri Jul 31 2026 harbour-xmatic contributors 0.13.0-1
- Rooms can be created. "New room" in the chat list's pulldown asks for a name
  and settles the two things that cannot be changed the same way afterwards:
  whether the room is listed in the homeserver's directory or open by
  invitation only, and whether it is end-to-end encrypted.
- People can be invited into a room by their address, from the room's pulldown
  menu. They show up in the member list as invited until they accept.
- A room can be left — from its row in the chat list or from the room itself,
  both after a remorse timer. On a room you were only invited to, the same
  entry declines the invitation.

* Fri Jul 31 2026 harbour-xmatic contributors 0.12.1-1
- A room stayed silent after it had been opened once: notifications were
  suppressed for the room the app had last visited rather than for the one on
  screen, so exactly the room being tested never made a sound until some other
  room was opened.
- A room left open while the phone is on the cover notifies again.

* Fri Jul 31 2026 harbour-xmatic contributors 0.12.0-1
- Profile pictures: in the chat list, in a room's member list and next to
  other people's messages. A one-to-one chat shows the other person even when
  the room itself carries no picture, and a name without one gets a circle
  with its initial instead of a gap.
- Names and pictures of message senders are complete now; before, they
  appeared for some people and not others depending on what had been synced.
- A message's menu no longer runs off the screen in landscape. There it keeps
  copy and reply and moves everything else — including delete — onto a page of
  its own, which is reachable. Upright the menu is unchanged.

* Fri Jul 31 2026 harbour-xmatic contributors 0.11.1-1
- A new message is audible again: the notification carried no category, and on
  Sailfish the tone, the vibration and the light all hang off that. It now uses
  the system's instant-messaging category, so the tone chosen in the settings
  plays. Should it stay quiet, check that chat sounds are switched on under
  Settings, Sounds and feedback — the app does not override that.
- A message arriving while the phone is in hand shows a banner instead of only
  an entry in the event feed.

* Fri Jul 31 2026 harbour-xmatic contributors 0.11.0-1
- xmatic is offered in the system's share dialog: a link from the browser, a
  picture from the gallery or a file from the file manager can be sent
  straight into a room. Picking the room opens it, so what was sent is
  visible. Sharing also works when the app was not running.
- Muting a room takes effect where it is switched: the marker and the menu
  entry used to keep the old state until the room was opened or the app
  restarted, although the room was muted on the server all along.
- A muted room no longer raises notifications. The mute was written to the
  server's push rules, which the app does not use for its own banners, so a
  muted room went on notifying exactly as before.
- Unmuting works for accounts whose default is to mute everything; until now
  it removed the room's own rule and left the default in charge.
- Muted rooms have their own marker instead of sharing the low-priority one.

* Fri Jul 31 2026 harbour-xmatic contributors 0.10.0-1
- New app icon: the X is gone — it read as the social network's logo rather
  than as this app. In its place, glyphs falling in columns behind a white m,
  on the silhouette the system's own icons use.
- The room's menu is reachable from anywhere in the conversation: the room
  name sits in a fixed strip at the top of the page, and pulling that strip
  down opens the menu. Reaching it no longer means scrolling back through
  the whole history.
- That strip also says whether the room is encrypted, and shows when the app
  has lost the network.
- Tapping the strip opens the room's member list.

* Fri Jul 31 2026 harbour-xmatic contributors 0.9.1-1
- Older messages load in rooms that were joined while the app was running:
  such a room arrived without any history and stopped at its first events.
- A conversation too short to fill the screen fetches history on its own
  instead of waiting for a scroll that cannot happen.
- The pinned-message banner stays one line high; a multi-line pin no longer
  covers the conversation.
- The pinned view says when the server refuses to hand out the pinned
  messages instead of claiming there are none.
- The attachment picker can be used in landscape.

* Thu Jul 30 2026 harbour-xmatic contributors 0.9.0-1
- Room list: favourites group to the top, low-priority rooms to the bottom,
  everything else stays in activity order. Rooms can be marked favourite or
  low priority from their context menu; low-priority rooms no longer raise
  notifications. Encrypted, favourite and low-priority each show their own icon.
- The room search field keeps focus while filtering.
- The message composer scrolls to the cursor instead of pushing long text
  out of view.
- Encrypted rooms warn before sending to a recipient whose devices you have
  not verified, with an option to stop warning about that user.

* Tue Jul 28 2026 harbour-xmatic contributors 0.8.0-1
- Runs on 32-bit devices: an armv7hl build for Sailfish OS 4.6 (Gemini PDA).
- Sign in on another device: an OAuth device-code flow for devices whose
  own browser cannot render the login pages.
- Voice and video calls receive audio and video reliably, including on the
  Gemini: the received call stream is routed to the speaker and unmuted, and
  the Android hardware video decoder (which refuses the stream) is bypassed.
- New direct chats are end-to-end encrypted from creation; existing rooms
  can turn encryption on.
- About page shows the app and core version.

* Sun Jul 26 2026 harbour-xmatic contributors 0.4.0-1
- Own profile: display name and avatar can be changed in the app.
- Rooms can be muted and unmuted; muted rooms show a small marker.
- Messages queued while offline are sent automatically on reconnect.
- Pinned messages: pin from the conversation, a banner with the newest
  pin's text sticks to the top of the room, own page per room, jump
  from a pinned message into the history around it.
- Pure profile changes no longer show as a system row in every room.
- Public room directory: search, preview (topic, member count), join.
- Spaces list their linked-but-not-joined rooms as joinable.

* Sun Jul 26 2026 harbour-xmatic contributors 0.3.1-1
- Sync recovers on its own after a network loss (flight mode, dead WLAN):
  the sync service now runs in offline mode and reconnects when the server
  is reachable again. The room list and space overview show an offline note
  while it does.

* Sat Jul 25 2026 harbour-xmatic contributors 0.3.0-1
- Spaces: own navigation level, create/delete, organise rooms by long-press,
  move between spaces, unread badges, choice of start page. Timeline system
  rows for calls and membership changes. German translations.

* Fri Jul 24 2026 harbour-xmatic contributors 0.2.0-1
- Messaging, encryption, verification, key backup, attachments, voice messages,
  notifications and voice calls.

* Fri Jul 24 2026 harbour-xmatic contributors 0.1.0-1
- M0a scaffold: Silica app with the Rust core cross-built and linked in.
