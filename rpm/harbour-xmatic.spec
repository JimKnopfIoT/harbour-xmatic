# Neutral packaging metadata — no personal identifiers (see anonymity rules).
# The build host's name would otherwise end up in the RPM header.
%define _buildhost reproducible-builder
Name:       harbour-xmatic
Summary:    Matrix client for Sailfish OS
# Kept in sync with the last published release; dev builds append +main.<date>.
Version:    0.21.0
Release:    1
License:    ASL 2.0
URL:        https://github.com/JimKnopfIoT/harbour-xmatic
Source0:    %{name}-%{version}.tar.bz2
Vendor:     harbour-xmatic contributors
Packager:   harbour-xmatic contributors

Requires:   sailfishsilica-qt5
# Round profile pictures are masked with QtGraphicalEffects (Avatar.qml).
Requires:   qt5-qtgraphicaleffects
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
BuildRequires: pkgconfig(sailfishsecrets)
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
they do in modern clients. Voice calls run over WebRTC via GStreamer. The
Rust core is linked in statically and is built ahead of packaging by
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
* Sat Aug 22 2026 harbour-xmatic contributors 0.21.0-1
- Formatted messages are shown as formatting instead of as raw text. Bold,
  italic, code blocks, quotes, lists, headings and links now render; what a
  message carries as HTML is rewritten inside the app into the small markup
  Qt can draw, character by character, so nothing a sender wrote is ever
  handed to a markup parser. Pictures in a message body become their
  description rather than a download, sender-chosen colours are dropped, and
  a link keeps its target only if that target is an ordinary web address -
  whether it can be tapped at all is still the setting under Account.
- The app now says when it is storing your session and messages unencrypted.
  It has always fallen back to unencrypted storage when the device's secure
  storage would not hand out a key, and it has always said so only in the
  system log, which is not somewhere anyone looks. The encryption page names
  the state and the reason. Where an encrypted store is possible but the
  existing one predates it, it also offers the one operation that works:
  sign out, sign back in. That deletes this device's keys, so it is refused
  unless a key backup exists on the server.
- Two copies of the app can no longer run on the same data. The client runs
  its database without the SDK's cross-process lock, which is a deliberate
  choice - the lock cost about fifty disk syncs per second while idle - but
  it is only safe with one process, and nothing was enforcing that. A lock
  file taken before the database opens does now; a second start hands over to
  the running one.
- The recovery key is treated like the login password on its way into the
  app: the buffer that carried it is overwritten, and the core wipes its own
  copy. A key that was just generated has to stay readable long enough to
  write it down, so that one is only dropped when the page closes.
- The interface is available in the twenty-four official languages of the
  European Union, in Russian, Norwegian and Icelandic. It also no longer
  simply follows the phone: Account now has a language of its own, English
  included, because only the German translation has been checked by a native
  speaker and one has to be able to get back out of the others. A change takes
  effect the next time the app starts.

* Fri Aug 21 2026 harbour-xmatic contributors 0.20.0-1
- A new room is now described completely at the moment it is created, because
  that is the only moment the server accepts most of it: topic, a public
  address people can type elsewhere, who may read the history, whom to invite
  right away, whether only moderators may write, whether everyone invited
  starts with the creator's rights, and whether the room stays on this server.
  Encryption stays where it was, as the one decision that cannot be undone.
- A message that cannot be decrypted now says why. The reason was there all
  along - the protocol library works it out and hands it over - and it was
  being thrown away, so every case read the same. The one that matters is
  "the sender did not share the key because they consider this device
  insecure", which names a setting on the other side instead of leaving both
  people guessing.
- Six dialogs were stuck in portrait on a device that is held sideways, among
  them the warning about unverified recipients, which appeared rotated by
  ninety degrees. They turn with the device now, and the short ones scroll so
  their input field cannot end up behind the keyboard.
- Tapping a quote jumps to the message it quotes. That was never wired up at
  all, and the same jump used by "show in conversation" for a pinned message
  only ever worked when the message happened to be loaded already: it waited
  for the row count to change, and a page of history that renders as nothing -
  a stretch of membership changes does it - never changes that count. It now
  retries on its own and says so when the message is beyond the loaded
  history, instead of doing nothing at all.
- A picture can carry a caption, and an answer can carry a picture. Picking a
  file used to send it on the spot, which made all three impossible at once:
  no caption, no reply, and no way back after tapping the wrong thumbnail.
  There is now a page between picking and sending that shows what is about to
  go out, quotes the message being answered, and takes the caption. Both
  belong there because neither can be added afterwards.
* Fri Aug 21 2026 harbour-xmatic contributors 0.19.0-1
- Every member has a profile page now, one level below the member list.
  Tapping a member, or a sender's picture in a conversation, opens it: the
  picture at full size, the address to copy with a tap, the role, since when
  they are a member and who invited them, the rooms you share, and whether
  their encryption identity is trusted. That last one has four answers, not
  two - the one worth knowing is "the identity changed since you verified
  it", which is shown in red together with the way out.
- Moderation moved onto that page: removing, banning and lifting a ban,
  making somebody a moderator or an admin. Each entry only appears where the
  room's power levels allow it, and making an admin asks first, because it
  cannot be taken back.
- Threads. A message that starts one carries a marker with the number of
  replies, a reply inside one says so, and both open a page of their own
  with its own composer. Replies in threads keep appearing in the
  conversation as well, so nothing is hidden from anybody who never opens
  the thread page.
- A notification now disappears when the room is read somewhere else. Read
  receipts travel between clients, so reading on a desktop clears the count
  here - and with the notification the banner, the feed entry and the
  communication LED go too, instead of claiming a message is still waiting.
- A reply whose quoted message cannot be loaded says so, rather than showing
  an ellipsis for good. That happens when the quoted event lives on a server
  ours cannot fetch it from, or history visibility forbids it.
- The account's ignored users are listed under Account, with a tap to stop
  ignoring somebody. The list belongs to the account and holds in every
  client. The send warning's "do not warn me about this user again" can be
  taken back there too - it promised that and had nowhere to do it.
- Room info offers renegotiating the encryption: the next message starts a
  fresh session and hands its key to every device again. The remedy when the
  other side reports it cannot read what this device sends.
- A homeserver that cannot do the sync this app needs is now named as such.
  Before, every sync failed, the offline mode turned that into "offline",
  and the banner flashed over an empty room list - which reads as a network
  fault and is not one. The app asks the server on start and says plainly
  that it is not supported.
- Sign-in failures say what went wrong. A server that only offers its own
  web sign-in (SSO), which this app cannot use yet, is no longer reported
  the same way as a wrong password.
- Security: text that other people write - display names, room names, server
  error messages - is rendered as plain text everywhere. A display name
  could previously contain markup, and a picture in it was fetched on sight.
  Error messages no longer carry room, user or event identifiers into the
  device log.

* Thu Aug 20 2026 harbour-xmatic contributors 0.18.3-1
- Idle no longer costs measurable CPU. The client held a cross-process store
  lock whose lease was rewritten into the crypto store every 50 ms - about
  fifty disk syncs per second, guarding against a second process that the
  single-instance launcher already rules out. The lock is single-process now;
  measured idle load fell from 3.4% to 0.3%, and the flash is left alone.
- Web links in messages can be tapped, behind a new switch under Account,
  off by default: a tapped link opens the browser, and that is attack
  surface one opts into. Only http(s) ever becomes a link, the shown text is
  the target itself, and message bodies are still never rendered as markup.
- The notification banner no longer shows the quoted message for edited
  replies. The edit fallback hid the quote block from the preview's
  stripper, so the banner could show readers their own words as if they
  were the news.
* Tue Aug 18 2026 harbour-xmatic contributors 0.18.2-1
- The room search keeps the keyboard. Every letter re-filters the list and
  the list comes back as a full reset; on each reset the list view recreated
  its current row and handed that row the focus, taking the keyboard from the
  search field in the header. The list has no current row any more, so there
  is nothing to hand the focus to - measured on the device, no loss in thirty
  keystrokes where before it was every one. The room directory search had the
  same layout and gets the same fix. (The 0.9.1 hand-back ran before the
  focus was taken and therefore never did anything.)
- Profile pictures are round. They were square with a highlight-coloured ring
  drawn across them - on the stock ambience a light-blue circle on every room
  icon - because a rectangular clip cannot round an image. A proper mask does.
- A notification can carry the message. Off by default, because the banner
  also lands on the lock screen; the switch is under Account -> This app. On,
  the banner shows the latest text (a picture, file, voice message or
  location is named as such, an undecryptable event says so) instead of the
  count.
- The appearance page has a visible "Reset to defaults" button - the entry
  in its pull-down was easy to miss - and the opacity slider follows a reset
  and a change of element instead of keeping its last dragged position.

* Tue Aug 18 2026 harbour-xmatic contributors 0.18.1-1
- Password sign-in no longer flickers into an empty room list: the sign-in
  reset the local store a second time, under the client it went on to use.
  The reset now belongs to whoever builds the client.
- A missing store key is a locked session, never a re-login. The key in the
  device's secrets storage is released only through the system's own
  approval dialog, which the app never allowed to appear — so after a reboot
  the encrypted session read as "no session", the login page came up and the
  sign-in cleared the store: a new device and a recovery-key round for every
  affected user. The app now asks with the system dialog (once per boot),
  logs the storage's reason, never replaces a key while encrypted data
  exists, and shows a "Locked" page with a retry when the key is not at
  hand.

* Sun Aug 16 2026 harbour-xmatic contributors 0.18.0-1
- A room can ask for mentions only, right here: the room page's mute switch
  became a four-way choice — account default, every message, mentions and
  keywords only, muted. Stored with the account's push rules, so it holds in
  every client, and the banner already follows it.
- The conversation's colours are the user's now. A new appearance page under
  Account offers one spectrum field with a circle marker, a grey ramp, a hex
  code that reads and types, and RGB sliders for the fine end; a selector
  says what is being coloured — either bubble, the sender name, either text
  colour — plus an opacity slider for the bubble fills. Everything defaults
  to following the ambience, a live preview shows the effect first, and one
  pull-down entry resets the lot.

* Sun Aug 16 2026 harbour-xmatic contributors 0.17.2-1
- Notifications follow the account's push rules. A room set to "mentions
  only" — in any client, the rule lives on the account — no longer banners
  and sounds for every message: only the events the rules say should notify
  raise the banner, and the banner counts those. The unread badge and the
  cover deliberately keep counting everything.
- The message text no longer reads its own laid-out width back, which the
  device journal reported as a binding loop on every wrapped message.

* Sun Aug 16 2026 harbour-xmatic contributors 0.17.1-1
- The bar of a reply quote now runs down its left side instead of across its
  top. The horizontal bar separated two identically styled names in a
  received bubble — the author above it, the quoted sender below — and which
  was which was anyone's guess.

* Sun Aug 16 2026 harbour-xmatic contributors 0.17.0-1
- A reply shows what it answers even when that message first has to be
  fetched: the quote holds its place while loading instead of collapsing,
  and a fetch the server refused gets one more try instead of staying empty
  forever.
- Own message bubbles, the quote inside them and the unread counters are
  readable on every ambience. Strong highlight fills swallowed their text on
  some colour schemes; bubbles and counter are now soft tints, a mention
  marks itself with a ring around the counter, and the number grew to a
  readable size.
- The cover no longer shows the account's identifier. Instead it counts what
  is new: rooms with unread messages over the messages themselves — 2/7 says
  seven new messages across two rooms.
- The About page can no longer name a build it is not: both of its version
  lines are forced into step with the package at build time, each having
  quietly gone stale once.

* Sat Aug 15 2026 harbour-xmatic contributors 0.16.0-1
- Sign in with username and password on homeservers without OAuth. The login
  page detects what the server speaks and only then offers the password form;
  the password is never stored, logged or kept, and the process now refuses
  debugger and memory access from other apps.
- The session file and newly created local stores are encrypted with a key
  held in the device's secrets storage. The first start asks for permission
  once; without it the app keeps working exactly as before.

* Tue Aug 11 2026 harbour-xmatic contributors 0.15.0-1
- A room that has been replaced now says so. When a room outgrows its version
  it is not migrated but replaced by a new one, and the old room keeps its
  history while silently accepting no new messages — it looked like a
  conversation that had merely gone quiet. The room list marks it, the room
  itself shows a banner, and one tap joins the room that took its place.
- New room-info page, opened by tapping the room's name: topic, address,
  members, encryption, access, room version and the internal room id, plus
  mute, favourite and low priority. The room's pull-down menu is shorter for
  it — ten entries were barely draggable on a small screen in landscape.

* Sat Aug 01 2026 harbour-xmatic contributors 0.14.0-1
- A message written while the network was gone now leaves as soon as it comes
  back. The send queue switches itself off after a failed send and waits for
  the client to switch it on again; nothing did, so the message sat there until
  the app was restarted — and it looked sent the whole time. Messages that did
  not get out are marked as such.
- Pictures can no longer appear in the wrong room. Attachments were remembered
  under a row number that started again from zero in every room, so the first
  picture of one room could be shown in place of the first picture of the next.
- The timeline no longer freezes. Opening a room could deadlock the core, after
  which nothing in the conversation worked until the app was restarted.
- Verifying a device now unlocks the old messages it was verified for: the key
  backup announces itself through a stream the app did not listen to, so the
  keys were never fetched afterwards.
- A verification can be started again after one stalled. Asking a second time
  used to cancel both attempts.
- The warning before sending into an encrypted room no longer says "everything
  checked" when nothing was: a member whose devices were never downloaded is
  now asked for instead of counted as verified.
- Reading a room and running a verification or a call no longer cancel each
  other's event subscription, which had made calls unreliable and left
  verifications looking stalled.
- The sync service is restarted when it gives up, instead of leaving the app
  quietly offline until the next start.
- "Load older messages" is no longer greyed out because something unrelated is
  loading, and the automatic history fetch stops instead of asking forever.
- Stickers and polls are visible instead of empty rows; quotes of older
  messages fill in; the room directory survives a dropped page; the pinned view
  no longer reports an error when there is nothing more to load.

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
