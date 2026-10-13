# Push notifications

**This feature exists because users asked for it. It is off by default, it has
its own setting, and it does nothing until it is switched on by hand.**

If you never open Account › Push notifications, nothing on this page applies to
you: no address is created, your homeserver is told nothing, and no third party
sees anything. Leaving it off costs you nothing except that xmatic keeps
behaving as it always has — messages arrive while it runs, and the cover has to
stay open.

Read the rest before turning it on. It is a trade, and the trade is real.

## Why xmatic needs someone else's help for this

xmatic has no background service, by decision. Nothing of it runs while it is
closed, so nothing can be received then.

[UnifiedPush](https://unifiedpush.org) does not change that decision. The part
that stays connected is a *distributor* — a separate app you install, which
holds one connection on behalf of every app on the device and wakes them when
something arrives. xmatic only speaks to it. Without a distributor installed,
this feature cannot be switched on at all, and the page says so.

## What you need

**A UnifiedPush distributor.** On Sailfish OS that's
[Foghorn](https://git.agnos.is/projectmoon/foghorn). Install it, start it, and
point it at an [ntfy](https://ntfy.sh) server, public or your own.

If more than one distributor is installed, the page asks which one xmatic
uses; nothing is picked for you. If the distributor is uninstalled later while
push is on, the page says so instead of claiming push is on.

### The gateway

The homeserver doesn't push to your phone's address directly. It posts to a
Matrix push gateway, which forwards. After you switch push on, xmatic asks
which one to use, and registers with the distributor only once you've picked:

- **Provided**: the gateway built into the push service your distributor
  uses, if it has one (ntfy does, Mozilla's doesn't). If it has none, the page
  says so and offers the public gateway; the other way out is a push service
  with a gateway in the distributor. xmatic finds out after registering.
- **UnifiedPush (public)**: `matrix.gateway.unifiedpush.org`, run by the
  UnifiedPush project.
- **Custom**: a URL you enter.

There's no default. If the push server doesn't answer when asked about its
gateway, the previous answer stays. A gateway set in an older version shows up
as Custom.

Gateways and push addresses must be https. Anything else is refused.

## What leaves your device when this is on

**Not your messages.** The pusher is registered with `event_id_only`, so a
notification carries a room identifier and a message identifier and nothing
else. The text is fetched by your phone, from your homeserver, and decrypted
here with keys that never leave.

**Metadata does, to parties that knew nothing before:**

- **The gateway** learns your push address and, for every notification, which
  room and which message, at what time, and how many notifications are
  waiting for you. Over a week that is an activity
  profile: this account, these rooms, these hours.
- **The push service** (your ntfy server, or Mozilla) sees your push address
  and every push. Gateways send the body unencrypted, so it sees the same room
  and message IDs and the same count.

With ntfy and its own gateway, that's one server (yours, if you run it).
Otherwise it's two.

Neither sees what you wrote. Both see when and in which rooms you're active.

**Your push address is a bearer secret.** Anyone who holds it can send a
notification to your phone — as often as they like. They cannot forge a
message: the content is fetched from your own homeserver and never comes from
the push. What they can do is make your phone wake up, which costs battery and
radio. Treat the address like a password; xmatic never displays it and never
writes it to the log.

## What stays the same

- The sandbox. The woken process runs under the same Sailjail profile as the
  app, takes the same single-process lock on the message store, and has no
  access the app does not. If it cannot take that lock, it does not start.
  Opening the app stops it: the push it is handling finishes and refreshed
  sign-in tokens are saved first, which can delay the app's start by up to
  ten seconds. Should it not let go by then, the app does not start and a
  notification asks you to open it again.
- Being woken is not being unlocked. The woken process reads the storage key
  and opens your session only once a push naming a message has actually
  arrived through your distributor, never merely because something started
  it. The distributor's own messages - a new address, or that it dropped
  xmatic - are no such push: they read no key, and the homeserver hears of
  them at the next app start, or from a session a push already opened.
- Your notification setting. If message text is switched off in Account ›
  Privacy, a push shows "New message" and nothing more, exactly as an ordinary
  arrival does.
- Your push rules. A room you muted stays quiet: the account's own rules decide
  whether a notification is shown and whether it makes a sound, and xmatic
  does not overrule them. A room marked low priority shows no banner, whether
  the app is open or not.

## What happens after a reboot

The key that unlocks xmatic's encrypted storage lives in Sailfish Secrets and
is bound to the device lock. The woken process asks for it only after a push
has arrived, takes it only if Secrets hands it over without asking, and wipes
its own copy as soon as the core has it.

After a restart, until you unlock the phone, a push can't be read. You get a
"New message" banner instead - for every push, since nothing can be looked up,
so in that time a push someone forged shows one too. A new address the
distributor hands out in that time reaches your homeserver at the next app
start.

## Turning it off

Account › Push notifications, switch off. xmatic removes the pusher from your
homeserver, unregisters from the distributor and deletes the stored address
and keys.

Only the pushers this installation set are removed. If you use xmatic on
another device with the same account, that device's pusher stays where it is.
To tell them apart, xmatic keeps salted hashes of its own push addresses in
`push-pushers.json`, filed per account under a salted hash of the account -
never the addresses or the account themselves.

If your homeserver is unreachable at that moment, the pusher stays behind for
now and the attempt shows up in Account › Error log. The hash stays too, and
the pusher is removed at the next start with a session.

Signing out does the same, also deletes the list of banners and forgets the
gateway choice. The registration file goes even if the distributor does not
answer; only giving the registration back to it then fails, and the error log
says so. If the homeserver cannot be reached while you sign out, you are told
so; the hash outlives the sign-out, and the pusher is removed the next time
that account signs in on this device. Signing in with another account in
between leaves it alone.

**Before you uninstall xmatic, switch push off.** Uninstalling cannot reach
your homeserver, so a pusher left on goes on receiving notifications for an
app that is gone, and the gateway goes on seeing them, until the homeserver
gives up on it or you remove it from another client.

## Files

All in xmatic's own data directory:

- `leghorn.json` - the distributor registration and the push address. Deleted
  when push is switched off and at sign-out.
- `push-banners.json` - banners the woken process raised, so the app can close
  them. Rooms appear only as salted hashes.
- `push-pushers.json` - salted hashes of the push addresses this installation
  registered, per account under a salted hash of the account, and of 0.45's
  push key until a session found its pusher. Exists only while one of them may
  still be on a homeserver.

## Coming from 0.45 or earlier

The old push setting is carried over the first time the app runs: it registers
with the distributor anew and gives the old registration back. The old pusher
on the homeserver is recognised by the key it carries and removed with the
first session after that. Until then a push for the old registration shows a
banner asking you to open xmatic once.

## Status

This is new and will need field reports. What is built:

- finding a distributor, registering with it, receiving the address
- the gateway choice
- setting and removing the pusher on the homeserver, including when the
  address changes while the app is closed
- re-registering with the distributor at every start while push is on
- receiving a push while the app runs; while its sync is running, the sync
  brings the message and no extra request is made
- showing the message when a push wakes the closed app ("New message" while
  the phone is still locked after a restart), in the app's language
- choosing between several distributors
- trying a pusher that did not reach the homeserver again once the
  connection is back; one the homeserver refused is reported, not retried

If it does not work for you, Account › Error log holds what failed, with
identifiers already removed, and can be copied out as it stands.
