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

/// The page a mention for this room goes to: the topmost room or thread page of
/// it with a composer. Each such page asks, and only the one named acts.
function mentionHost(pageStack, roomId) {
    return pageStack.find(function(page) {
        return page.takesMentions === true && page.roomId === roomId
    })
}
