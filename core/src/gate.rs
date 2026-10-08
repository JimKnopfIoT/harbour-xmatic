//! Who may create, replace or drop the client, and with it the store, the
//! store key and `session.json`. The rules only; the runtime holds the lock.

/// What the assigned client is for. Changed only under the session gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Empty,
    /// Built and vetted, nothing running: the password form waits on it.
    Prelogin,
    /// A sign-in owns it; its network part runs outside the gate.
    LoggingIn,
    Session,
}

/// The slot's state. Every assignment and every removal raises the
/// generation, so a stale holder can tell it lost the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub generation: u64,
    pub phase: Phase,
}

impl Default for Slot {
    fn default() -> Self {
        Self { generation: 0, phase: Phase::Empty }
    }
}

impl Slot {
    pub fn assigned(&mut self, phase: Phase) -> u64 {
        self.generation += 1;
        self.phase = phase;
        self.generation
    }

    pub fn cleared(&mut self) {
        self.generation += 1;
        self.phase = Phase::Empty;
    }

    /// Moves the phase only for the client that `generation` names.
    pub fn advance(&mut self, generation: u64, phase: Phase) -> bool {
        if self.generation != generation || self.phase == Phase::Empty {
            return false;
        }
        self.phase = phase;
        true
    }

    pub fn holds(&self, generation: u64) -> bool {
        self.generation == generation && self.phase != Phase::Empty
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginKind {
    /// Browser or device code: starts over, whatever was begun.
    Start,
    /// The password form: reuses the client the start vetted.
    Password,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// Cancel what runs, drop the client, reset, build.
    Fresh,
    /// Take the prelogin client of this generation.
    Reuse(u64),
}

/// What a sign-in may do with the slot it finds.
pub fn login_entry(slot: Slot, kind: LoginKind) -> Result<Entry, &'static str> {
    match (slot.phase, kind) {
        (Phase::Session, _) => Err("already signed in"),
        // A second tap would sign in twice and leave two devices.
        (Phase::LoggingIn, LoginKind::Password) => Err("a sign-in is already running"),
        (Phase::Prelogin, LoginKind::Password) => Ok(Entry::Reuse(slot.generation)),
        _ => Ok(Entry::Fresh),
    }
}

/// Whether a handed-in store key may replace the installed one: only while no
/// client exists, since every client was built with the key it found.
pub fn key_replaceable(slot: Slot) -> bool {
    slot.phase == Phase::Empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removal_outdates_the_holder() {
        let mut slot = Slot::default();
        let mine = slot.assigned(Phase::LoggingIn);
        assert!(slot.holds(mine));
        slot.cleared();
        assert!(!slot.holds(mine));
        // A later client is not mine either, though it is a client again.
        let theirs = slot.assigned(Phase::LoggingIn);
        assert!(!slot.holds(mine));
        assert!(slot.holds(theirs));
        assert!(!slot.advance(mine, Phase::Session));
        assert_eq!(slot.phase, Phase::LoggingIn);
    }

    #[test]
    fn signed_in_refuses_every_login() {
        let mut slot = Slot::default();
        slot.assigned(Phase::Session);
        assert!(login_entry(slot, LoginKind::Start).is_err());
        assert!(login_entry(slot, LoginKind::Password).is_err());
    }

    #[test]
    fn the_password_reuses_the_vetted_client_once() {
        let mut slot = Slot::default();
        let generation = slot.assigned(Phase::Prelogin);
        assert_eq!(login_entry(slot, LoginKind::Password), Ok(Entry::Reuse(generation)));
        assert!(slot.advance(generation, Phase::LoggingIn));
        assert!(login_entry(slot, LoginKind::Password).is_err());
    }

    #[test]
    fn a_start_replaces_whatever_was_begun() {
        let mut slot = Slot::default();
        assert_eq!(login_entry(slot, LoginKind::Start), Ok(Entry::Fresh));
        slot.assigned(Phase::Prelogin);
        assert_eq!(login_entry(slot, LoginKind::Start), Ok(Entry::Fresh));
        slot.advance(slot.generation, Phase::LoggingIn);
        assert_eq!(login_entry(slot, LoginKind::Start), Ok(Entry::Fresh));
        assert_eq!(login_entry(Slot::default(), LoginKind::Password), Ok(Entry::Fresh));
    }

    #[test]
    fn the_key_changes_only_without_a_client() {
        let mut slot = Slot::default();
        assert!(key_replaceable(slot));
        let generation = slot.assigned(Phase::LoggingIn);
        assert!(!key_replaceable(slot));
        slot.advance(generation, Phase::Prelogin);
        assert!(!key_replaceable(slot));
        slot.cleared();
        assert!(key_replaceable(slot));
    }
}
