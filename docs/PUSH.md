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
  room and which message, at what time. Over a week that is an activity
  profile: this account, these rooms, these hours.
- **The push service** (your ntfy server, or Mozilla) sees your push address
  and every push. Gateways send the body unencrypted, so it sees the same room
  and message IDs.

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
  access the app does not. Opening the app stops it.
- Your notification setting. If message text is switched off in Account ›
  Privacy, a push shows "New message" and nothing more, exactly as an ordinary
  arrival does.
- Your push rules. A room you muted stays quiet: the account's own rules decide
  whether a notification is shown, and xmatic does not overrule them.

## What happens after a reboot

The key that unlocks xmatic's encrypted storage lives in Sailfish Secrets and
is bound to the device lock. The woken process only takes it if Secrets hands
it over without asking.

After a restart, until you unlock the phone, a push can't be read. You get a
"New message" banner instead. A new address the distributor hands out in that
time reaches your homeserver at the next app start.

## Turning it off

Account › Push notifications, switch off. xmatic removes the pusher from your
homeserver, unregisters from the distributor and deletes the stored address
and keys.

If your homeserver is unreachable at that moment, the pusher stays behind for
now and the attempt shows up in Account › Error log. xmatic keeps a note of it
and removes it at the next start with a session; signing out clears it too.

Signing out does the same, and also deletes the list of rooms that had a
banner.

## Status

This is new and will need field reports. What is built:

- finding a distributor, registering with it, receiving the address
- the gateway choice
- setting and removing the pusher on the homeserver, including when the
  address changes while the app is closed
- re-registering with the distributor at every start while push is on
- receiving a push while the app runs
- showing the message when a push wakes the closed app ("New message" while
  the phone is still locked after a restart)

What is not:

- choosing between several distributors — the first one found is used

If it does not work for you, Account › Error log holds what failed, with
identifiers already removed, and can be copied out as it stands.
