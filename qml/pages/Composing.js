.pragma library

/// Recipients with unverified devices the user has not chosen to trust yet.
/// Empty means nothing stands in the way of sending. Shared by the room and
/// the thread: one warning, one rule.
function pendingUnverified(users, matrix) {
    var out = []
    for (var i = 0; i < users.length; i++) {
        if (!matrix.recipientTrusted(users[i].userId)) {
            out.push(users[i])
        }
    }
    return out
}
