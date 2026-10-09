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

**A UnifiedPush distributor for Sailfish OS.** At the time of writing the one
that exists is [Foghorn](https://git.agnos.is/projectmoon/foghorn). Install it,
start its service, and **connect it to an [ntfy](https://ntfy.sh) server** in
its settings - the public one or your own.

That is all. There is nothing to enter in xmatic.

### The gateway, and why there is nothing to set

A Matrix homeserver cannot push to your phone's address directly: it posts to
a *Matrix push gateway*, which forwards to the address. xmatic uses
[Leghorn](https://git.agnos.is/projectmoon/foghorn/src/branch/master/leghorn),
Foghorn's own connector library, and Leghorn finds the gateway the way Element
does:

- It asks the push server behind your address whether it is a Matrix gateway
  itself. **ntfy is**, so with Foghorn on ntfy the server that holds your
  address is also the gateway, and no third party is added.
- Otherwise - Foghorn on the Mozilla Push Service, its default - it uses the
  public gateway of the UnifiedPush project, `matrix.gateway.unifiedpush.org`.

Account › Push notifications shows which one is in use.

## What leaves your device when this is on

**Not your messages.** The pusher is registered with `event_id_only`, so a
notification carries a room identifier and a message identifier and nothing
else. The text is fetched by your phone, from your homeserver, and decrypted
here with keys that never leave.

**But metadata does, to parties that knew nothing before:**

- **The gateway** learns your push address and, for every notification, which
  room and which message, at what time. Over a week that is an activity
  profile: this account, these rooms, these hours.
- **The push service** (your ntfy server, or Mozilla) sees your push address
  and the bytes passing through. Matrix gateways send that body unencrypted,
  so it sees the same room and message identifiers.

With Foghorn on ntfy those two are one server - your own, if you run it. On
Mozilla they are two, and the gateway is the UnifiedPush project's.

None of them sees a word you wrote. All of them see when and where you are
active.

**Your push address is a bearer secret.** Anyone who holds it can send a
notification to your phone — as often as they like. They cannot forge a
message: the content is fetched from your own homeserver and never comes from
the push. What they can do is make your phone wake up, which costs battery and
radio. Treat the address like a password; xmatic never displays it and never
writes it to the log.

## What stays the same

- The sandbox. The woken process runs under the same Sailjail profile as the
  app, takes the same single-process lock on the message store, and has no
  access the app does not.
- Your notification setting. If message text is switched off in Account ›
  Privacy, a push shows "New message" and nothing more, exactly as an ordinary
  arrival does.
- Your push rules. A room you muted stays quiet: the account's own rules decide
  whether a notification is shown, and xmatic does not overrule them.

## What happens after a reboot

The key that unlocks xmatic's encrypted storage lives in Sailfish Secrets and
is bound to the device lock. It can only be handed out through the system's own
dialog, and a process started in the background cannot answer a dialog.

So after a restart, until you have opened xmatic by hand once, a push cannot be
decrypted. You still get a banner — it says a message arrived and nothing more
— because silence would leave you believing nothing had.

## Turning it off

Account › Push notifications, switch off. xmatic removes the pusher from your
homeserver and gives the registration back, in that order.

If your homeserver is unreachable at that moment, the pusher may stay behind
and the server will keep posting to an address that no longer exists. That
attempt shows up in Account › Error log. Switching off again once you are
online, or signing out, clears it.

Signing out deletes the registration in any case: an address that outlives the
device it was made for is a secret pointing at a stranger.

## Status

This is new and will need field reports. What is built:

- finding a distributor, registering with it, receiving the address
- finding the Matrix gateway for that address
- registering and removing the pusher on the homeserver, and replacing it when
  the distributor moves the address
- re-registering with the distributor at every start
- receiving a push while the app runs
- being woken by a push while the app is closed and raising a banner that
  says "New message" (no store is opened, nothing is fetched)

What is not:

- choosing between several distributors — the first one found is used

If it does not work for you, Account › Error log holds what failed, with
identifiers already removed, and can be copied out as it stands.
