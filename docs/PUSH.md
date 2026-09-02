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

1. **A UnifiedPush distributor for Sailfish OS.** At the time of writing the
   one that exists is [Foghorn](https://git.agnos.is/projectmoon/foghorn), which
   uses the Mozilla Push Service. Install it and start its service.
2. **A Matrix push gateway.** A Matrix homeserver cannot talk to a push
   distributor directly: it speaks the Matrix push protocol, and the
   distributor speaks Web Push. Something in between has to translate.

   There is no default for this in xmatic and there cannot be one. The gateway
   sees a room and message identifier for every notification you get, so whose
   gateway you use is a decision only you can make.

### A note on the gateway, as of this writing

The usual translator is
[common-proxies](https://codeberg.org/UnifiedPush/common-proxies), which is
what runs behind the public `matrix.gateway.unifiedpush.org`. Its Matrix
gateway forwards the homeserver's notification to the distributor's endpoint
**without setting a `TTL` or a `Content-Encoding` header**.

A Web Push service requires both. Measured against a real Mozilla endpoint:

| what is sent | answer |
|---|---|
| body only — what the Matrix gateway sends | `400 Missing TTL value` |
| `TTL` alone | `400 Missing Content-Encoding header` |
| `Content-Encoding` alone | `400 Missing TTL value` |
| both, body unencrypted | `201` |

So with a Mozilla-backed distributor, that gateway cannot deliver. The same
project's *generic* gateway sets both headers and works. Until this is fixed
upstream, a gateway you run yourself with those two headers added is the way
through. Nothing in xmatic can work around it — the headers are set by whoever
runs the gateway.

This is also why nothing is encrypted end-to-end on that hop: the push service
validates those headers and relays the body opaquely, and no component in that
chain encrypts. Which brings us to the part that matters.

## What leaves your device when this is on

**Not your messages.** The pusher is registered with `event_id_only`, so a
notification carries a room identifier and a message identifier and nothing
else. The text is fetched by your phone, from your homeserver, and decrypted
here with keys that never leave.

**But metadata does, to two parties that knew nothing before:**

- **The gateway** learns your push address and, for every notification, which
  room and which message, at what time. Over a week that is an activity
  profile: this account, these rooms, these hours.
- **The push service** (Mozilla, with Foghorn) sees your push address and the
  bytes passing through. Because that body travels unencrypted today, it sees
  the same room and message identifiers.

Neither sees a word you wrote. Both see when and where you are active.

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
- registering and removing the pusher on the homeserver
- receiving a push while the app runs
- being woken by a push while the app is closed, fetching the message and
  raising the banner

What is not:

- choosing between several distributors — the first one found is used
- re-registering by itself when a distributor is reinstalled

If it does not work for you, Account › Error log holds what failed, with
identifiers already removed, and can be copied out as it stands.
